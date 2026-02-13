---
title: Network seed 23 combat damage life mismatch
status: closed
priority: 3
issue_type: task
labels:
- bug
- network
created_at: 2026-02-12T16:19:30.771870570+00:00
updated_at: 2026-02-12T22:59:13.862311570+00:00
closed_at: 2026-02-12T22:59:13.862311480+00:00
---

# Description

## Summary

Network vs local equivalence test seed 23 fails with a combat damage life total mismatch.

## Symptoms

- Server: Life [14, 8] (Gabriel took 4 damage from Fire Lord Ozai)
- Client: Life [14, 12] (Gabriel did NOT take the 4 damage)
- Action count mismatch: client=944, server=945 (off by 1)

## Reproduction

```bash
./tests/network_vs_local_equivalence_e2e.sh 23 random random
```

## Context

The game log shows damage being logged on both sides:
```
[GAMELOG Turn17 CD] Fire Lord Ozai (28) deals 4 damage to Gabriel (life: 8)
```

But the actual life total on the client doesn't match.

The issue occurs during combat after:
1. Fire Lord Ozai activates ability to exile from opponent's library
2. Sandbenders' Storm is cast from exile
3. Modal spell warning: "ModalChoice effect reached execute_effect - should have been resolved during casting"

May be related to:
- Modal spell not being resolved during casting
- Action count mismatch causing state divergence
- Combat damage calculation timing

## Resolution

**FIXED** in commit c15b104d5 (target_card_ids protocol field) and 0804cd8b0 (fatal error handling).

The root cause was target choices using index-based protocol where the client's `valid_targets`
list could differ from the server's. By sending actual CardIds instead of indices, the target
selection now stays synchronized.

Seed 23 and all other tested seeds (3, 5, 7, 11, 13, 17) now pass heuristic vs random network tests.

## Related Issues

This is separate from the exile-from-library fix in mtg-ar269.
