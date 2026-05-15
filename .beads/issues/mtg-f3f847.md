---
title: 'Network desync (WASM↔WASM): combat blocking assignment (fuzz seed=7, Turn13 DB)'
status: open
priority: 2
issue_type: bug
labels:
- network
- desync
- wasm
- fuzz
- combat
created_at: 2026-05-15T16:04:10.049194681+00:00
updated_at: 2026-05-15T16:04:10.049194681+00:00
---

# Description

## Summary

Network fuzz harness on integration @ ff1817f7 found a FATAL state-hash mismatch during the Declare Blockers step of Turn 13 in a wasm/heuristic vs wasm/heuristic game.

- Seed: `7`
- Mode: network, both clients WASM, both controllers heuristic
- Fail point: Turn 13, Declare Blockers (action_count = 804)
- Symptom: state hash mismatch immediately after blocker assignment is submitted; gamelog shows blockers assigned differently on server vs client.
- This is distinct from the WASM lethal-blocker-choice fix in ff1817f7 (mtg-e05f9c). That fix added overrides for `choose_blocker_for_lethal_damage` / `choose_blocker_for_remaining_damage`. This bug surfaces in the prior step (initial blocker declaration) and survives that fix.
- This is NOT Seismic Sense (mtg-c54e90) and NOT the cycle/scry desync (mtg-ced6d1).

Tracker: mtg-1f7ab9

## Reproducer

```bash
cd /home/newton/working_copies/mtg/mtg-forge-rs
./tests/network_vs_local_equivalence_e2e.sh 7 heuristic heuristic --client wasm
```

Original failing run log: `/tmp/network_fuzz_x255yy_x`

## Hypothesis

Suspected to be another WASM controller method that lacks an override paralleling its native NetworkLocalController/RemoteController counterparts (same class as mtg-e05f9c). Most likely candidates:
- `assign_blockers` / `declare_blockers` choice routing
- A missing `peek_opponent_choice` / `submit_choice_with_targets` call when more than one blocker is selected

Inspect `mtg-engine/src/wasm/network/{remote,local}_controller.rs` for any `choose_*blocker*`, `assign_*`, or `declare_*` methods that fall through to the trait default impl.

## Investigation Steps

1. Reproduce; capture both gamelogs.
2. Diff to find the first diverging entry at Turn 13 DB.
3. Locate the responsible WasmRemoteController / WasmNetworkLocalController gap.
4. Mirror the pattern from ff1817f7 (the lethal-damage-blocker fix).
5. Add a regression test using a deterministic combat puzzle.

## Acceptance Criteria

- `./tests/network_vs_local_equivalence_e2e.sh 7 heuristic heuristic --client wasm` passes with byte-identical local + network gamelogs.
- The WASM combat-blocker code path has tests covering BOTH the existing lethal/remaining-damage choice AND the initial blocker assignment.
- Network fuzz no longer reproduces this seed.
