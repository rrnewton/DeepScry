---
title: Network equivalence test seed 4 heuristic/random times out
status: open
priority: 3
issue_type: task
labels:
- bug
created_at: 2026-02-11T17:57:00.851746907+00:00
updated_at: 2026-02-19T14:07:40.953670743+00:00
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

---

## Update (2026-02-19_#1836)

After the random controller WASM fixes (token ID mismatch, guard field reset), fuzz testing shows heuristic controller failures persist:

- seed=2 heuristic/heuristic: `DESYNC DETECTED: NetworkController 0 received invalid choice index 3 (only 2 options)`
- seed=7 heuristic/heuristic: `ABILITY SYNC BUG - server has 2 abilities, local has 1`
- seed=7 opposite direction: gamelog DIFFERENCES

Pass rate: 1/10 seeds with heuristic vs heuristic (seed=1 passes).

The ABILITY SYNC BUG occurs in Turn 18 combat (seq=212 LethalDamageAssignment). The WASM shadow game has `action_count=1006 (local=1006)` but just before, at seq=212, shows `action_count=1005 (local=1006)` - a 1-action divergence. The server sends `available_count=0` but the WASM heuristic finds `abilities=1`, causing a desync. The WebSocket closes mid-game.

Random controller (seeds 1-10) all pass - this is heuristic-specific.

Root cause hypothesis: Heuristic AI may be using card information (e.g., hand contents, ability details) that differs between full server state and shadow state, causing different decision paths that diverge game state.

Reproducer: `RAYON_NUM_THREADS=2 python3 tests/network_vs_local_equivalence.py 2 heuristic heuristic native wasm`
