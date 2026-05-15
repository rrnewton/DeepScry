---
title: 'Heuristic AI infinite-loop after Plainscycling discard: ''Insufficient mana'' retried until timeout'
status: open
priority: 3
issue_type: task
created_at: 2026-05-14T14:28:05.518989869+00:00
updated_at: 2026-05-14T14:28:05.518989869+00:00
---

# Description

## Summary

Network fuzz test seed=1 zero+random hung for 92s and was killed by the timeout. The server log shows that after Gabriel cycled a Rabaroo Troop via Plainscycling, a subsequent attempt to cast Ostrich-Horse (cost 2G) failed with 'Insufficient mana'; the controller appears to retry the same cast forever instead of choosing a different action.

## Reproducer

```bash
cd mtg-forge-rs
./tests/network_vs_local_equivalence_e2e.sh 1 zero random
```

Logs preserved at /tmp/qa-fail-timeout.

## Server log tail

```
[GAMELOG Turn14 DA] Gabriel uses Plainscycling on Rabaroo Troop (cost: 2)
  Rabaroo Troop is discarded
[INFO  state] [SHUFFLE-DEBUG] Before shuffle: lib_len=26 ... After shuffle: ...
[GAMELOG Turn14 M2] Gabriel casts Ostrich-Horse (68) (putting on stack)
  Error casting spell: Invalid game action: Failed to pay mana cost ManaCost { generic: 2, ... green: 1 ... }: Insufficient mana
[ERROR priority] [WASM RESUME] Failed to cast spell 68: ... Insufficient mana
```

(Then the same retry repeats until 92s timeout.)

## Notes

There are two suspected sub-bugs:

1. **Plainscycling resolution**: card was discarded but the gamelog never says 'Gabriel searches for a Plains'/'puts Plains into hand'. Library is shuffled but no card is added to hand. Possibly the cycling effect isn't fetching its land.
2. **Controller forward-progress**: regardless of whether cycling worked, the controller should not pick the same illegal action repeatedly. Need to filter or post-fail-blacklist the option.

## Discovered by

`bug_finding/network_fuzz_test.py` 45-config pass on `qa-fuzz-testing` @ fe820468, 2026-05-14.
