---
title: 'Network desync (WASM↔WASM): 3-color mana payment for Glider Kids cast (fuzz seed=1, Turn6 M1)'
status: open
priority: 2
issue_type: bug
labels:
- network
- desync
- wasm
- fuzz
created_at: 2026-05-15T16:04:09.465133099+00:00
updated_at: 2026-05-15T16:04:09.465133099+00:00
---

# Description

## Summary

Network fuzz harness on integration @ ff1817f7 found a FATAL state-hash mismatch during a 3-color mana payment for casting Glider Kids in a wasm/heuristic vs wasm/heuristic game.

- Seed: `1`
- Mode: network, both clients WASM, both controllers heuristic
- Fail point: Turn 6, Main 1 (action_count = 318)
- Symptom: state hash diverges between server and client during multi-mana-tap sequence preceding cast resolution of Glider Kids
- This is NOT Seismic Sense (mtg-c54e90) and NOT the cycle/scry desync (mtg-ced6d1).

Tracker: mtg-1f7ab9

## Reproducer

```bash
cd /home/newton/working_copies/mtg/mtg-forge-rs
./tests/network_vs_local_equivalence_e2e.sh 1 heuristic heuristic --client wasm
```

Original failing run log: `/tmp/network_fuzz_i9srnsmt`

## Hypothesis

Mana-pool / activated-ability ordering in the WASM controller probably diverges from the native server when the heuristic taps a 3-color mana base for the cast cost. Likely related class to the WASM combat-blocker fix in ff1817f7 (mtg-e05f9c) — i.e. a controller method missing an override / not consuming an OpponentChoice in the WASM client.

## Investigation Steps

1. Reproduce with the command above; capture both gamelogs.
2. Diff server gamelog against client gamelog; find the first diverging line around Turn6 M1.
3. Check the heuristic mana-payment path (likely `auto_pay_mana_for_*` / mana-tap choice routing) for a missing WASM controller override or a missing `submit_choice_with_targets`-style call mirror.
4. Add a regression unit test or e2e puzzle once the root cause is found.

## Acceptance Criteria

- `./tests/network_vs_local_equivalence_e2e.sh 1 heuristic heuristic --client wasm` passes with byte-identical local + network gamelogs.
- Network fuzz suite (--configs 10, --client wasm, --parallel 1) does not reproduce this failure on subsequent runs.
- Regression test added that exercises the 3-color mana-payment path through a WASM controller.
