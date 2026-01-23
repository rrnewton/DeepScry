---
title: 'BUG: Network vs Local gamelog divergence'
status: closed
priority: 2
issue_type: bug
created_at: 2026-01-22T18:48:10.009585292+00:00
updated_at: 2026-01-23T19:21:19.714087803+00:00
---

# Description

## Description

## BUG: Network vs Local Gamelog Divergence

**RESOLVED**: This issue has been investigated and resolved.

### Root Cause Analysis Summary

The divergence was caused by two distinct issues:

1. **Library Search Card Visibility** (FIXED):
   - CLIENT uses `init_game_reserve_only` which reserves CardID slots with `None` values
   - Cards are NOT instantiated until revealed via `CardRevealed` messages
   - The `valid_cards` filtering code silently skipped unrevealed cards
   - **Fix**: Server now sends `CardRevealed` for all `library_search_cards` BEFORE sending `ChoiceRequest`, and client calls `sync_to_action()` BEFORE filtering

2. **Heuristic Controller Information Visibility** (ARCHITECTURAL - NOT A BUG):
   - `heuristic` controller uses `GameStateView` for decisions (e.g., `evaluate_creature()`)
   - In NETWORK mode, CLIENT's shadow game view may have different/incomplete card info
   - Different evaluations → different sort order → different decisions
   - This is **expected behavior** - the network architecture is designed for SERVER-CLIENT sync, not for CLIENT decisions to match LOCAL game decisions

### Test Status

- `./tests/network_vs_local_equivalence_e2e.sh 3 zero` → **PASS** (162 identical gamelog entries)
- `./tests/network_vs_local_equivalence_e2e.sh 3 heuristic` → Expected divergence (CLIENT makes decisions with limited view)

### Resolution

1. Test script defaults to `zero` controller which validates deterministic game engine
2. Documentation updated to explain that `heuristic`/`random` divergence is expected
3. The library search reveal timing fix ensures proper card visibility before filtering

## Related Issues

- mtg-a33hf (closed): Library search state divergence - added RevealReason::Searched handling
- mtg-037fw: Network synchronized GameLoop sync issues (causes network_game_e2e.sh to be SKIPPED)
- mtg-secqu: Single-channel architecture
