---
title: 'BUG: Network vs Local gamelog divergence'
status: closed
priority: 2
issue_type: bug
created_at: 2026-01-22T18:48:10.009585292+00:00
updated_at: 2026-01-25T19:11:11.223791600+00:00
---

# Description

## BUG: Network vs Local Gamelog Divergence

**RESOLVED**: This issue has been fully resolved with commit d4ba8b80.

### Root Cause Analysis Summary

The divergence was caused by multiple issues, now all fixed:

1. **Library Search Card Visibility** (FIXED in earlier commits):
   - Server now sends `CardRevealed` for all library_search_cards BEFORE sending ChoiceRequest
   - Client calls `sync_to_action()` BEFORE filtering

2. **LibrarySearchByName Random Instance Selection** (FIXED in d4ba8b80):
   - When multiple cards share the same name (e.g., 3 Mountains during Mountaincycling),
     LOCAL mode used inner controller RNG to pick a specific instance
   - NETWORK mode always picked first instance because protocol only sent unique names
   - **Fix**: Added `name_counts` to protocol, server sends counts, client generates
     synthetic CardIds encoding (name_idx, instance_idx) for inner controller to pick
   - Client decodes response [name_idx+1, instance_idx] to select specific card

3. **Heuristic Controller Information Visibility** (WAS BUG, NOW FIXED):
   - Controllers must NEVER depend on hidden information (opponent hand, library order)
   - Any controller that produces different results on server vs client has an info-leakage bug
   - ALL controller types (heuristic, random, zero) must produce identical gamelogs

### Test Status

- `./tests/network_vs_local_equivalence_e2e.sh 3 zero` → **PASS** (identical gamelogs)
- `./tests/network_vs_local_equivalence_e2e.sh 3 random` → **PASS** (99 identical entries)
- `./tests/network_vs_local_equivalence_e2e.sh 1 random` → **PASS** (identical gamelogs)

### Files Modified in d4ba8b80

- `protocol.rs`: Added `name_counts: Vec<usize>` to LibrarySearchByName
- `server.rs`: Populate name_counts from valid_cards grouped by name
- `local_controller.rs`: Generate synthetic CardIds, decode instance selection
- `controller.rs`: Skip validation of instance_idx for LibrarySearchByName

### Resolution

The `random` controller now produces IDENTICAL gamelogs between LOCAL and NETWORK
modes when both use the same seed. This validates the network protocol correctly
propagates randomized choices through the inner controller.
