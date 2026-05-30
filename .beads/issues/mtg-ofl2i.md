---
title: 'Native-vs-WASM engine divergence: WASM create_game_from_database hand-rolls setup (different CardID assignment + shuffle order) breaking same-seed determinism'
status: open
priority: 2
issue_type: bug
created_at: 2026-05-30T06:32:43.753296370+00:00
updated_at: 2026-05-30T13:10:46.838292113+00:00
closed_at: 2026-05-30T12:26:50.807448634+00:00
---

# Description

## Native-vs-WASM determinism divergence — PARTIAL (CardID mapping converged; controller-seed divergence remains)

The mtg-forge-rs engine compiles to BOTH a native binary and a WASM module and
MUST produce the SAME game for the SAME seed in either target. The fuzz sweep
(scripts/native_wasm_equiv_sweep.py) showed 100% divergence.

## Root cause #1 (CardID assignment order) — FIXED
- NATIVE (mtg-engine/src/loader/game_init.rs::init_game_with_positional_ids,
  ~L211-238): shuffles the card-DEFINITION vectors FIRST (P1 then P2 via the
  game RNG), THEN assigns positional CardIDs (CardID N == post-shuffle library
  position). `mtg tui` uses this + GameLoop::skip_opening_hands().
- WASM (mtg-engine/src/wasm/fancy_tui.rs::create_game_from_database):
  instantiated CardIDs in DECK order then shuffle_library() permuted the Vec.
  Same permutation, DIFFERENT CardID->card mapping.

Fixed by converging create_game_from_database onto the native sequence (shuffle
def-vectors, assign positional CardIDs, draw 7 silently without re-shuffle).
VERIFIED in the saved divergence diffs: native and WASM now assign IDENTICAL
CardIDs (e.g. Scrubland=57, Strip Mine=56, Sedge Troll=52, Mox Ruby=49, Animate
Dead=48, Shivan Dragon=47 in BOTH targets) and draw identical opening hands.

## Root cause #2 (controller seed derivation) — STILL OPEN
Even with identical CardIDs and opening hands, the games still diverge at the
FIRST controller decision (e.g. native P1 plays Scrubland(57) first, WASM P1
plays Strip Mine(56) first — same hand, different pick). The native `mtg tui`
path seeds each controller with `derive_player_seed(game_seed, PlayerSlot::P1/P2)`
(see mtg-engine/src/game/seed_derivation.rs). The local WASM path used by the
sweep — `launch_game_session` -> `WasmFancyTuiState::new` (fancy_tui.rs L3632,
L1468) — does NOT apply `derive_player_seed` per slot; it uses the default
`controller_seed` so the RandomController's RNG stream starts from a different
state than native. (The WASM *network* / ai_harness paths DO use
derive_player_seed — see wasm/mod.rs L682, wasm/network/ai_harness.rs L150 — so
this is specific to the single-player `launch_game_session` entry point.)

Fix direction: make `launch_game_session`/`WasmFancyTuiState::new` derive
per-slot controller seeds with `derive_player_seed(seed, P1/P2)` exactly as
native does, so the RandomController RNG streams match. This is a SEPARATE fix
from the CardID convergence above and was not completed in this pass.

## Current sweep status (after CardID fix, fresh WASM bundle)
scripts/native_wasm_equiv_sweep.sh STRICT: still 9/9 DIVERGED (3 decks x 3
seeds), all at the first controller pick. The validate-wired bounded leg
(Makefile validate-wasm-e2e-step) therefore REMAINS on --expect-divergence; do
NOT flip it to STRICT until root cause #2 is fixed and the sweep shows equality.

## MTG rules review (CardID-fix portion)
PASS. The CardID-assignment change is init-only: same cards, same shuffle
distribution (CR 103.2), same 7-card opening hand (CR 103.4); only CardID
assignment order canonicalized to match native. No game-visible semantics change,
no information-hiding regression.

## Relationship to Java Forge
N/A — Rust-only cross-compile-target determinism issue; Java Forge has no WASM
target.
