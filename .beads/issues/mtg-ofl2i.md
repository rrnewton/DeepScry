---
title: 'Native-vs-WASM engine divergence: WASM create_game_from_database hand-rolls setup (different CardID assignment + shuffle order) breaking same-seed determinism'
status: open
priority: 2
issue_type: bug
created_at: 2026-05-30T06:32:43.753296370+00:00
updated_at: 2026-05-31T05:29:58.447127107+00:00
closed_at: 2026-05-30T12:26:50.807448634+00:00
---

# Description

## Native-vs-WASM determinism divergence — root causes #1 & #2 FIXED; THIRD residual (trigger/life-drift) remains OPEN

The mtg-forge-rs engine compiles to BOTH a native binary and a WASM module and
MUST produce the SAME game for the SAME seed in either target.

## Root cause #1 (CardID assignment order) — FIXED (d4adbb77, was a2c95308)
WASM create_game_from_database now mirrors native init_game_with_positional_ids:
shuffle card-DEFINITION vectors first (P1 then P2 via game RNG), THEN assign
positional CardIDs, draw 7 silently without re-shuffle. Verified: identical
CardIDs + opening hands native vs WASM.

## Root cause #2 (controller seed derivation) — FIXED (301c4a3f)
The local WASM launch path (launch_game_session -> WasmFancyTuiState::new) used
a hardcoded controller seed (42) for both players instead of the canonical
derive_player_seed(master_seed, P1/P2) that native `mtg tui` uses. Fixed:
- WasmFancyTuiState::new now takes the master seed; create_ai_controller derives
  per-slot seeds via derive_player_seed (mtg-engine/src/wasm/fancy_tui.rs).
- Persistent p1/p2 AI controllers carried across turns (RNG advances
  continuously, mirroring native's single run_game() call).
- DRY: reuses crate::game::seed_derivation::derive_player_seed; no duplicated
  salt logic.
Also fixed a WASM gamelog-gating bug: log_effect_execution gated gamelog() lines
on should_print_to_stdout() (stdout-only), silently dropping
exiles/destroys/counters lines in WASM Memory mode. New logger_captures_or_prints()
(Stdout|Both|Memory) is the correct gate (mtg-engine/src/game/game_loop/logging.rs).

## Sweep result after #1+#2 (FRESH `make wasm-dev` bundle), 2026-05-30_#2523(301c4a3f)
bug_finding/native_wasm_equiv_sweep.sh STRICT, decks/old_school2/*.dck x seeds 1-3,
max-turns 8: **28/36 PASS** (was 0/9 = 100% diverged before the fix). The action
SEQUENCE is now byte-identical for the vast majority of games — the seeding +
CardID convergence is confirmed working. Deterministic (re-run reproduces).

## THIRD residual (still OPEN) — trigger-processing / silent-life divergence
8/36 combos still diverge. NOT a seeding/RNG-start issue (early actions match
byte-for-byte). Two sub-buckets, both genuine engine-behavior divergence between
native `run_game` (continuous) and the WASM `run_one_turn` step loop:

(A) SILENT LIFE-TOTAL DRIFT — action sequence identical, accumulated life differs:
  - ur_burn seed1 @#52: City of Brass `Taps` trigger (deals 1 dmg to controller
    on tap) applied a different number of times in WASM vs native. No gamelog
    line either side (self-damage is silent); only the life delta diverges.
  - artifact_aggro s3 @#42, erhnamgeddon_gw s1/s3, lestree_zoo s2 @#67
    (native P1=12 vs WASM P1=16 — 4 missed pings). Same family: pain-land /
    tap-trigger self-damage count mismatch.

(B) DECISION / TRIGGER-SEQUENCE DRIFT — action sequence itself diverges:
  - triple_s_sage seed2 @#61: Su-Chi "when dies, add {C}{C}{C}{C}" death trigger
    FIRES + logs in native (actions/mod.rs:7040-7050, ungated gamelog) but does
    NOT fire in WASM (the trigger line + mana-pool line are absent → the trigger
    did not execute, not merely log-suppressed).
  - white_weenie seed2 @#28: native plays an extra Plains before casting Balance;
    WASM casts Balance without it (controller decision drift downstream of (A)'s
    silent state difference perturbing later picks).
  - mono_black_control seed2 @#44: turn-boundary / discard-vs-draw ordering.

ROOT-CAUSE HYPOTHESIS for the residual: the WASM AI-vs-AI path drives the game
one turn at a time via GameLoop::run_one_turn, whereas native runs the whole
game through a single GameLoop::run_game. Triggered abilities that fire at turn
boundaries / on tap / on death are processed differently across the per-turn
re-entry, causing (A) different self-damage trigger counts and (B) some triggers
(Su-Chi death) not firing. This is a SEPARATE root cause from CardID assignment
(#1) and controller seeding (#2) and needs its own fix: converge the WASM
run_one_turn trigger/SBA processing onto native run_game semantics.

## Tripwire status — NOT flipped (correct per process)
The validate-wired leg (Makefile validate-wasm-e2e-step: --seeds 1
--decks ur_burn --max-turns 8 --expect-divergence) STILL diverges (ur_burn @#52),
so --expect-divergence stays GREEN and is left in place. Do NOT flip to STRICT
until the THIRD residual (trigger-processing convergence) is fixed and the sweep
shows full equality. Keep this issue OPEN.

## MTG rules review (this pass: seeding + log-gate)
PASS. Changes are init-time RNG seed derivation + gamelog line-gating only; no
game-visible rule semantics changed; controllers remain information-independent
(CR 103/130; docs/NETWORK_ARCHITECTURE.md determinism invariant). The residual
(A)/(B) divergences are PRE-EXISTING engine bugs newly EXPOSED by the now-aligned
seeds, not introduced by this change.

## Relationship to Java Forge
N/A — Rust-only cross-compile-target determinism issue; Java Forge has no WASM target.

## UPDATE 2026-05-30: THIRD residual FIXED (mtg-8scpx) — native-vs-WASM leg flipped to STRICT
The third root cause was NOT a run_one_turn-vs-run_game stepping difference. It was
WASM card deserialization: WasmCardDatabase::load_set/load_tokens deserialized the
per-set .bin without calling CardDefinition::rebuild_parsed_svars(), so parsed_svars
() was empty and every SVar-backed trigger (Execute$ <SVar>) parsed
to ZERO effects — dropping City of Brass's silent Taps self-ping and Su-Chi's death
trigger. Native loads from cardsfolder with parsed_svars populated, hence the divergence.
Fixed in mtg-engine/src/wasm/mod.rs (mirrors the native network path which already
rebuilds). Sweep: old_school2 36/36 PASS (was 24/12), old_school+old_school2 54/54 PASS.
Makefile validate-wasm-e2e-step: --expect-divergence DROPPED; the leg now runs
old_school2 x seeds1 x maxturns8 in STRICT mode and ASSERTS native==WASM. All three
root causes (#1 CardID order, #2 controller seeding, #3 parsed_svars rebuild) are now
fixed. This issue can be CLOSED once validate confirms the STRICT leg green.
