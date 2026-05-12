---
title: 'Fix WASM rewind/replay desync: mana_state_version included in Replay hash'
status: closed
priority: 3
issue_type: bug
created_at: 2026-05-12T22:33:14.850750573+00:00
updated_at: 2026-05-12T22:33:18.209921303+00:00
closed_at: 2026-05-12T22:33:18.209921233+00:00
---

# Description

## Bug

Detected by replay_verifier in fancy.html (rogue_rogerbrand mirror, seed 41).
After a user makes any second choice on turn 1 (e.g. play Mox, then play Bayou)
the verifier surfaces:

    REWIND/REPLAY FATAL: turn-start state hash for turn 1 changed across rewinds
    (expected 0x39f66a97714fa9e0, got 0xa0e86d98b8132581)

## Root cause

`UndoLog::rewind_to_turn_start` (mtg-engine/src/undo.rs:991) unconditionally
bumps `game.mana_state_version` to invalidate the `ManaEngine` memoization
cache. That field is a pure cache-invalidation counter — but the Replay-mode
hash (`compute_state_hash` → `strip_metadata`) was NOT excluding it. So:

  - rewind #1: bump mana_state_version V → V+1, cache turn-start hash H(V+1)
  - forward play: more bumps from taps/etb, mana_state_version → V+N
  - rewind #2: bump V+N → V+N+1, hash H(V+N+1) ≠ H(V+1) → fatal

## Fix

Added `"mana_state_version"` to `EXCLUDED_FIELDS` in
`mtg-engine/src/game/state_hash.rs`. It was already excluded from
`EXCLUDED_FIELDS_UNDO_TEST` and `EXCLUDED_FIELDS_NETWORK`; the Replay-mode
exclusion was the missing case.

## Regression tests

- `game::state_hash::tests::mana_state_version_excluded_from_replay_hash`
- `game::state_hash::tests::mana_state_version_excluded_in_all_replay_paths`
- `undo::tests::rewind_to_turn_start_produces_stable_replay_hash`

The undo-side test (rewind→forward→rewind, asserting turn-start Replay hash
stability) was confirmed to FAIL on the unfixed code: hash mismatch
17192338584256235471 vs 8150816592150228069.

## Related

- mtg-zl49k: Web GUI undo/rewind architecture bugs
- mtg-ee5054: Replay verifier missing from game.html (gui_view_model)
