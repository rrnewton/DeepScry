---
title: 'cycle_ability_network_sync_e2e seed=315 random/random: gamelog desync re-emerges'
status: open
priority: 2
issue_type: bug
created_at: 2026-05-26T20:48:34.490491527+00:00
updated_at: 2026-05-26T20:48:34.490491527+00:00
---

# Description

## Summary

After fixing the test-harness wait-loop hang (mtg-ivrqv), the `cycle_ability_network_sync_e2e.sh` regression test now runs to completion (no more 180s timeout) but FAILS its core assertion: LOCAL and SERVER gamelogs diverge by ~232 lines for seed=315 random/random.

Discovered 2026-05-26 on branch `fix-mtg-ivrqv` (based on integration `6db45ef1`).

## Symptom

Excerpt from divergence:
```
3c3
<   [GAMELOG Turn2 M2] Gabriel plays Thriving Grove (79)
---
>   [GAMELOG Turn2 M1] Gabriel plays Thriving Grove (79)
9,10c9,11
<   [GAMELOG Turn4 M1] Gabriel plays Forest (76)
<   [GAMELOG Turn4 DA] Gabriel uses Plainscycling on Rabaroo Troop (cost: 2)
---
>   [GAMELOG Turn4 M1] Gabriel casts Barrels of Blasting Jelly (72) (putting on stack)
>   [GAMELOG Turn4 M1] Gabriel plays Forest (74)
>   [GAMELOG Turn4 M1] Gabriel uses Plainscycling on Rabaroo Troop (cost: 2)
```

Phase tag drift (`M2` vs `M1`) and the appearance of `casts Barrels of Blasting Jelly` in network mode that is absent in local mode indicate a state divergence between the two runs, not just a re-ordering artifact.

The test does NOT report any `ABILITY SYNC BUG` or `FATAL DESYNC` markers in client logs — clients exit with matching `winner=Some(1)`. So this is a *gamelog-level* desync visible only via the strict line-for-line equivalence assertion, not a fatal in-game desync.

## Context / suspected cause

This may be a regression introduced between when mtg-ced6d1 (cycle/Mountaincycling fix) was originally validated and HEAD `6db45ef1`. The harness timeout introduced by `67f046f0` (multi-game lobby) masked this for some unknown number of commits — `cycle_ability_network_sync_e2e` was hitting the 180s timeout, not actually comparing gamelogs.

Candidate culprits to investigate (commits between mtg-ced6d1 landing and 6db45ef1):
- `61f28fd9` Merge branch 'fix-cycle-desync' into integration (the fix itself)
- `290fc29f` Merge branch 'fix-scry-choice-pipeline' into integration
- `a22c05f9` fix(ci): add missing ScryOrder/Surveil match arms to ChoiceContext
- `67f046f0` (multi-game lobby) — unlikely to affect engine determinism

## Reproducer

```sh
git checkout 6db45ef1   # or any commit with the fix-mtg-ivrqv wait-loop fix
git submodule update --init forge-java
cargo build --release --features network
bash tests/cycle_ability_network_sync_e2e.sh
## -> "LOCAL and SERVER gamelogs differ by 232 lines"
```

Logs preserved at `/tmp/network_vs_local_e2e_<pid>/` (see test output for exact path).

## NOT blocking mtg-ivrqv

The wait-loop fix in mtg-ivrqv is correct and complete on its own. This separate desync regression was previously hidden by the timeout and is now exposed. Filing as a distinct bug.

## Related

- mtg-ivrqv (lobby-lifecycle wait-loop fix that exposed this)
- mtg-ced6d1 (original cycle/Mountaincycling desync fix being regressed)
- mtg-c232f4 (separate snapshot bincode regression on same HEAD)
