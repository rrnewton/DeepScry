---
title: 'Native-vs-WASM trigger/life drift: WASM tui_run_turn step-loop diverges from native run_game (Su-Chi death trigger, City of Brass self-damage)'
status: closed
priority: 2
issue_type: bug
created_at: 2026-05-30T22:06:25.279984686+00:00
updated_at: 2026-05-31T05:29:49.140528494+00:00
---

# Description

Native-vs-WASM trigger/life drift: WASM tui_run_turn step-loop diverged from native run_game (Su-Chi death trigger, City of Brass self-damage)

## ROOT CAUSE (FIXED)
NOT a run_one_turn-vs-run_game stepping difference. Proven: a native repro that
drives the SAME seeded game via repeated GameLoop::run_one_turn (both fresh-GL-per-turn
AND reused-GL) is BYTE-IDENTICAL to native GameLoop::run_game (ur_burn seed1: all three
end P1=7 P2=8). So the WASM step loop semantics are correct.

The real bug: WASM card-data DESERIALIZATION. CardDefinition.parsed_svars is
`#[serde(skip)]`, so a bincode-deserialized CardDefinition arrives with an EMPTY
parsed_svars map. Trigger parsing (loader/card.rs parse_triggers) resolves
`Execute$ <SVar>` trigger effects via parsed_svars. WasmCardDatabase::load_set
(and load_tokens) in mtg-engine/src/wasm/mod.rs deserialized the per-set .bin
WITHOUT calling rebuild_parsed_svars(), so EVERY SVar-backed trigger parsed to
ZERO effects:
  - City of Brass `T:Mode$ Taps ... Execute$ TrigDamage` self-ping (silent 1 dmg) dropped.
  - Su-Chi `T:... Execute$ <mana SVar>` death trigger dropped.
The native binary loads from cardsfolder with parsed_svars populated, so it fired
the effects -> the two compile targets diverged. The native NETWORK path already
rebuilds parsed_svars after deserialize (network/client.rs, reveal_processor.rs);
WASM load_set just missed the same call.

## FIX (DRY convergence, not a mask)
mtg-engine/src/wasm/mod.rs: load_set + load_tokens now call
`def.rebuild_parsed_svars()` on each deserialized CardDefinition before wrapping
in Arc — identical to the established native network-path pattern. This makes the
WASM card definitions produce the SAME parsed triggers/effects as native; no
WASM-specific game logic was added.

## EVIDENCE
bug_finding/native_wasm_equiv_sweep:
  - BEFORE: old_school2 x seeds1-3 x max-turns8 = 24 PASS / 12 DIVERGED.
  - AFTER:  old_school2 x seeds1-3 x max-turns8 = 36/36 PASS, 0 diverged.
  - AFTER:  old_school + old_school2 x seeds1-3 x max-turns12 = 54/54 PASS, 0 diverged.
First diverging line pre-fix (ur_burn seed1 @#50):
  native: 'Turn6 M1 Psionic Blast deals 4 damage to P2 (life: 5)'
  wasm:   'Turn6 M1 Psionic Blast deals 4 damage to P2 (life: 6)'
  -> traced to missing City of Brass Taps self-ping at Turn3 DA (native P1=19, WASM P1=20 at Turn4).

Regression test: mtg-engine/src/loader/card.rs
test_svar_trigger_survives_bincode_roundtrip_after_rebuild — pins that a bincode
round-trip + rebuild_parsed_svars restores the SVar-backed Taps DealDamage effect,
and that WITHOUT rebuild the effect is dropped (the bug shape).

## VALIDATE LEG FLIPPED TO STRICT
Makefile validate-wasm-e2e-step: dropped --expect-divergence (the mtg-ofl2i
tripwire) and broadened to `--decks 'decks/old_school2/*.dck' --seeds 1
--max-turns 8` (no --expect-divergence). The leg now ASSERTS native==WASM
byte-identical and PASSES. See mtg-ofl2i for the parent flip note.

## MTG rules review: PASS (data-loading correctness fix; restores effects native
already had; no semantics change; controllers unchanged; determinism invariant
RESTORED — native==WASM).
