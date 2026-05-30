---
title: Property-based testing with proptest
status: in_progress
priority: 4
issue_type: feature
created_at: 2025-10-26T21:06:34+00:00
updated_at: 2026-05-30T05:45:14.597440427+00:00
---

# Description

Add property-based tests using proptest crate:
- Generate random game states
- Verify invariants hold
- Fuzz testing for parser
- Automated shrinking for bug reproduction

PROGRESS (2026-05-29, branch fuzz-proptest-invariants):
proptest added as a [dev-dependencies] entry (dev-only; 0 normal/build dep
edges for native+wasm, verified via cargo tree). First property suite landed
at mtg-engine/tests/proptest_invariants.rs with 6 properties, all green under
cargo nextest:
  - prop_same_seed_determinism (48 cases): same (seed,deck pair) => identical
    gamelog + final state hash.
  - prop_snapshot_resume_fidelity (48 cases): snapshot@N + resume => same final
    state hash as uninterrupted run (mirrors main.rs::run_resume for Random).
  - prop_undo_rewind_round_trip (32 cases): rewind-to-turn-start + replay@last
    choice => same final state hash.
  - prop_action_log_* (256 cases each): ActionLog<T> push/get round-trip,
    frontier, non-destructive reads, absent reads = None, non-increasing push
    panics. Complements existing example-based unit tests, no duplication.
All games run in-process via GameInitializer::init_game_with_positional_ids +
GameLoop::run_game (no shelling). Auto-discovered by make validate's nextest.
Remaining for full close: parser fuzzing (card-script DSL) is not yet covered
by a proptest; the engine-state invariant + ActionLog goals are met.
