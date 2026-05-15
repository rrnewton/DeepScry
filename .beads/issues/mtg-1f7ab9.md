---
title: 'Network fuzz testing: desync tracking and progress'
status: open
priority: 2
issue_type: task
labels:
- network
- fuzz
- tracking
created_at: 2026-05-15T16:05:41.736974846+00:00
updated_at: 2026-05-15T16:06:34.050464457+00:00
---

# Description

## Purpose

Persistent tracker for the network fuzz-and-fix cycle. Records each fuzz run (date, commit, mode-by-mode pass rates) and links every NEW bug discovered by `bug_finding/network_fuzz_test.py` to its dedicated minibeads issue. Use this issue as the single entry point when re-running fuzz, comparing pass rates over time, or triaging follow-up work.

## Scope

- ONLY tracks failures surfaced by the network fuzz harness (state-hash desync, FATAL choice mismatches, gamelog drift, test-infra races).
- Each fuzz run gets its own dated section under "History".
- Per-bug issues are filed separately and back-linked here.

## Current Baseline (2026-05-15, integration @ ff1817f7)

Network-only mode (apples-to-apples vs initial QA baseline):

| Mode                     | Initial QA | After ff1817f7 | Delta   |
|--------------------------|-----------:|---------------:|--------:|
| Native vs Native         | 70%        | 80%            | +10pp   |
| Mixed (native vs WASM)   | 50%        | 25%            | -25pp   |
| WASM vs WASM (parallel=3)| 20%        | 0%             | -20pp * |
| WASM vs WASM (parallel=1)| 20%        | 0%             | -20pp   |
| Random vs Random         | 63%        | n/a            | n/a     |

`*` parallel=3 WASM regression is dominated by the test-infra race on `web/data/deck_submission.json` (mtg-7fe89f), not a product regression.

Local-equivalence mode (`--local-equivalence`) shows much lower pass rates than network-only mode; suspected to be mostly gamelog annotation drift (Turn2 M1 vs M2) on otherwise identical action sequences. See mtg-4437dc.

| Mode (equivalence) | Pass |
|--------------------|-----:|
| Native             | 35%  |
| Mixed              |  5%  |
| WASM               | 10%  |

## Expected Next Baseline (after fix-seismic-sense + fix-cycle-desync land on integration)

| Mode    | Predicted |
|---------|----------:|
| Native  | ~100%     |
| Mixed   | 70-80%    |
| WASM    | ~40%      |

Predictions per `tg show rerun-fuzz-after-fixes` analysis: Seismic Sense (mtg-c54e90) was the dominant remaining cause of WASM/Mixed failures, and the cycle desync (mtg-ced6d1) was the second-largest contributor.

## History of Fuzz Runs

### Run 1 -- 2026-05-15, integration @ ff1817f7

Tested fixes already on integration:
- WASM combat desync (ff1817f7)
- RNG determinism (61e06688)
- Plainscycling cost (ceea322e)
- SMART damage (mtg-e05f9c)

NOT yet on integration when this run was taken:
- fix-seismic-sense (b2a606f2 / commit 0286571d) -- see mtg-c54e90, FIXED on feature branch, awaiting CI before merge
- fix-cycle-desync (3b052c70) -- see mtg-ced6d1, FIXED on feature branch, awaiting CI before merge

Configs: native=20, mixed=20, wasm=10. Pass rates as in the table above.

Logs preserved at:
- /tmp/fuzz_after_fixes_native.log         (80% pass)
- /tmp/fuzz_after_fixes_native_equiv.log   (35% pass, equiv mode)
- /tmp/fuzz_after_fixes_mixed_netonly.log  (25% pass)
- /tmp/fuzz_after_fixes_mixed.log          (5% pass, equiv mode)
- /tmp/fuzz_after_fixes_wasm_seq.log       (0% pass, parallel=1)
- /tmp/fuzz_after_fixes_wasm_netonly.log   (0% pass, parallel=3 - infra race)
- /tmp/fuzz_after_fixes_wasm.log           (10% pass, equiv mode)

## Linked Issues

### Already-known fixes awaiting promotion to integration
- **mtg-c54e90** -- Seismic Sense Dig desync -- FIXED on branch `fix-seismic-sense` (commit `0286571d`), awaiting CI before merge to `integration`. Dominant cause of remaining native vs native and many WASM/Mixed failures in Run 1.
- **mtg-ced6d1** -- Cycle / Mountaincycling library-reorder desync -- FIXED on branch `fix-cycle-desync` (commit `3b052c70`), awaiting CI before merge to `integration`. Second-largest contributor to remaining failures.

### NEW bugs filed from Run 1 (2026-05-15)
- **mtg-b9988d** -- WASM vs WASM desync during 3-color mana payment for Glider Kids cast (seed=1, Turn6 M1, action_count=318). Repro: `./tests/network_vs_local_equivalence_e2e.sh 1 heuristic heuristic --client wasm`. Log: `/tmp/network_fuzz_i9srnsmt`.
- **mtg-f3f847** -- WASM vs WASM desync during combat blocking assignment (seed=7, Turn13 DB, action_count=804). Distinct from the WASM lethal-blocker fix in ff1817f7 (mtg-e05f9c). Repro: `./tests/network_vs_local_equivalence_e2e.sh 7 heuristic heuristic --client wasm`. Log: `/tmp/network_fuzz_x255yy_x`.
- **mtg-7fe89f** -- Test-infra race: parallel WASM fuzz threads collide on `web/data/deck_submission.json` (priority 3). Workaround: `--parallel 1` for WASM-only.
- **mtg-4437dc** -- Local-equivalence mode shows Turn2 M1 vs M2 gamelog annotation drift between local and network coordinators (priority 3). Could be cosmetic formatter drift OR real phase-tracking divergence; requires triage before equivalence mode results can be trusted.

### Other related bugs surfaced in Run 1 but not yet root-caused
- `/tmp/network_fuzz_if449fkp` -- P2 hash mismatch at action_count=680 (suspected Seismic Sense + something else).
- `/tmp/network_fuzz_3wvuurb3` -- hash mismatch during multi-mana-tap Turn15 M2 (likely related to mtg-b9988d; will be re-evaluated after fix-seismic-sense merges).

## How to Re-run

```bash
cd /home/newton/working_copies/mtg/mtg-forge-rs
## Native (apples-to-apples baseline)
python3 bug_finding/network_fuzz_test.py --configs 20 --mode network --seed-base 0
## Mixed
python3 bug_finding/network_fuzz_test.py --configs 20 --mode network --client wasm --seed-base 0
## WASM parallel=1 (avoid deck_submission.json race until mtg-7fe89f is fixed)
python3 bug_finding/network_fuzz_test.py --configs 10 --mode network --client wasm --parallel 1
```

After each run, append a new "Run N -- date, integration @ <sha>" subsection above with the pass-rate table, link any new bug issues, and update the "Current Baseline" snapshot.

## See Also

- docs/NETWORK_ARCHITECTURE.md -- deterministic sequential simulation model
- bug_finding/network_fuzz_test.py -- the harness
- tests/network_vs_local_equivalence_e2e.sh -- the per-seed reproducer used by most linked bugs
