---
title: Network equivalence test seed 4 heuristic/random times out
status: open
priority: 3
issue_type: task
labels:
- bug
created_at: 2026-02-11T17:57:00.851746907+00:00
updated_at: 2026-02-11T17:57:00.851746907+00:00
---

# Description

## Issue

The network equivalence test times out for seed 4 with heuristic/random controllers.

## Reproduction

```bash
python3 tests/network_vs_local_equivalence.py 4 heuristic random
```

This consistently times out after ~180 seconds.

## Observations

- Seed 4 with random/random controllers passes in ~19 seconds
- Other seeds (1, 2, 3, 5, 6, 7, 8, 9, 10) pass with heuristic/random
- This suggests an issue specific to the heuristic controller's decision making for this particular game state

## Possible causes

1. Infinite loop in heuristic evaluation
2. Deadlock in network communication
3. Edge case in game state causing infinite priority passing

## Notes

Discovered while testing fix for mtg-ar269 (mill reveal desync).
