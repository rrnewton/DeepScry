---
title: 'cycle_ability_network_sync_e2e seed=315 random/random: gamelog desync re-emerges'
status: closed
priority: 2
issue_type: bug
created_at: 2026-05-26T20:48:34.490491527+00:00
updated_at: 2026-05-26T22:36:58.090703241+00:00
---

# Description

## Summary

After fixing the test-harness wait-loop hang (mtg-ivrqv), the `cycle_ability_network_sync_e2e.sh` regression test now runs to completion (no more 180s timeout) but FAILS its core assertion: LOCAL and SERVER gamelogs diverge by ~232 lines for seed=315 random/random.

## RESOLUTION (2026-05-26)

**Root cause**: test-harness seed mismatch. NOT an engine bug.

Commit `61e06688` ("fix(rng): centralize seed derivation") changed the network `connect` command so that `--seed-player=N` is now treated as a **master seed**, with per-slot derivation `derive_player_seed(N, slot)` applied internally (P1 gets `N+0x1234_5678_9ABC_DEF0`, P2 gets `N+0xFEDC_BA98_7654_3210`). The local `tui` command's `--seed-p1=N`/`--seed-p2=N` flags continue to take per-controller seeds *directly* (no derivation).

`tests/network_vs_local_equivalence_e2e.sh` was unchanged and kept passing `--seed-p1 3 --seed-p2 3` to local TUI while passing `--seed-player 3` to each network client. After 61e06688 the two modes silently got different per-controller seeds:
- LOCAL:   P1=3,                   P2=3                    (raw, identical streams)
- NETWORK: P1=1311768467463790323, P2=18364758544493064723 (derived, distinct streams)

This caused the RandomController RNG streams to diverge from the very first choice → different action selection in identical game states → the observed 232-line gamelog diff (and the spurious "Barrels of Blasting Jelly" cast that appears only server-side).

The wait-loop fix (mtg-ivrqv) exposed this because before it, the test was timing out at 180s before reaching the gamelog comparison, so nobody noticed the seed mismatch the centralization commit introduced.

**Fix**: pre-derive the per-player seeds in the test harness so LOCAL `--seed-p1`/`--seed-p2` get the SAME values the network client computes from `--seed-player` via `derive_player_seed`. The harness now computes:

```
P1_DERIVED_SEED = printf '%u' $((CONTROLLER_SEED + 0x123456789ABCDEF0))
P2_DERIVED_SEED = printf '%u' $((CONTROLLER_SEED + 0xFEDCBA9876543210))
```

and passes them to the local TUI invocation. Network mode is unchanged.

## Validation

- `bash tests/cycle_ability_network_sync_e2e.sh` → PASS (LOCAL and SERVER gamelogs IDENTICAL)
- `bash tests/network_vs_local_equivalence_e2e.sh` (default seed=3, zero/zero) → PASS
- Seed sweep seed ∈ {100, 200, 315, 400, 500} × controller=random/random → all PASS
- Controller sweep seed=315 × controller ∈ {zero, heuristic} → all PASS
- `cargo fmt --all -- --check` clean
- `make validate` test suite: 1215/1215 PASS (only `lobby_probe` example fails — pre-existing, unrelated to this issue, requires network feature flag in Makefile)

## Files changed

- `tests/network_vs_local_equivalence_e2e.sh` (test harness only; no engine changes)

## Related

- mtg-ivrqv (lobby-lifecycle wait-loop fix that exposed this)
- mtg-ced6d1 (original cycle/Mountaincycling desync fix; still valid, just was being tested with mismatched seeds)
- mtg-c232f4 (separate snapshot bincode regression on same HEAD)
- commit 61e06688 (the root cause: introduced seed-derivation asymmetry between `tui` and `connect`)
