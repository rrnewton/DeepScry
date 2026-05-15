---
title: 'Test infra: parallel WASM fuzz threads collide on web/data/deck_submission.json'
status: open
priority: 3
issue_type: bug
labels:
- test-infra
- fuzz
- wasm
created_at: 2026-05-15T16:04:10.653637229+00:00
updated_at: 2026-05-15T16:04:10.653637229+00:00
---

# Description

## Summary

`bug_finding/network_test_lib.py:325` writes `web/data/deck_submission.json` via `os.replace()` when launching a WASM client. When `network_fuzz_test.py` runs with `--parallel >= 2 --client wasm`, multiple worker threads race on the same path. In the failing case the `.tmp` file gets renamed away before another worker reads it, producing `FileNotFoundError` and failing every parallel WASM job in the batch — purely a harness artifact, not a product bug.

This is the root cause of the observed `WASM ↔ WASM (parallel=3) = 0%` regression in the 2026-05-15 fuzz run. Sequential (parallel=1) WASM still fails for OTHER reasons (see linked bugs), but the parallel mode is unusable in its current form.

Tracker: mtg-1f7ab9

## Reproducer

```bash
cd /home/newton/working_copies/mtg/mtg-forge-rs
python3 bug_finding/network_fuzz_test.py --configs 10 --mode network --client wasm --parallel 3
```

Failing log preserved at: `/tmp/fuzz_after_fixes_wasm_netonly.log`

Workaround in use: `--parallel 1` for WASM-only fuzz runs.

## Fix Options

1. **Per-worker temp directory** (preferred): each fuzz worker gets its own working dir with its own `web/data/` subtree (or override the path via env var so workers don't share state).
2. **File lock** around the `os.replace()` site — simpler but serializes the slowest step of WASM launch and partially defeats `--parallel`.
3. **Refactor deck submission** to pass deck contents in-memory or via a unique-named file per game.

Option 1 is cleanest and matches how `network_fuzz_test.py` already isolates per-game gamelog directories.

## Acceptance Criteria

- `python3 bug_finding/network_fuzz_test.py --configs 10 --mode network --client wasm --parallel 3` runs to completion with no `FileNotFoundError` from `network_test_lib.py:325`.
- Parallel WASM pass rate matches sequential WASM pass rate (i.e. infra is not the bottleneck).
- A note in `bug_finding/README.md` (or equivalent) documents the isolation guarantee.
