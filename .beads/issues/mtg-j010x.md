---
title: Global test-flakiness tracking system (canonical names + stress harness + flakiness DB)
status: open
priority: 2
issue_type: task
created_at: 2026-05-28T18:39:41.006380697+00:00
updated_at: 2026-05-28T19:02:54.037188153+00:00
---

# Description

Global flakiness-tracking system IMPLEMENTED on branch `flakiness-harness` (base integration a7e0a196). Tooling/scripts + doc + data file; minimal engine impact.

## Deliverables

1. CANONICAL TEST IDENTITY — documented in `ai_docs/reference/TEST_FLAKINESS.md`. One flat `validate.` namespace:
   - `validate.<pkg>--<binary>.<module::test>` (cargo/nextest; `--lib` for unit tests). Round-trips to `cargo test -p <pkg> --test <binary> -- --exact <path>`.
   - `validate.shell_script_tests.<stem>` (tests/*.sh auto-discovered by shell_script_tests.rs)
   - `validate.wasm_e2e.<stem>` (web/test_*.js)
   - `validate.network_e2e.<deck_stem>.<seed>` (one name per deck+seed scenario — the granularity at which desyncs show up)
   - `validate.examples.<name>` (cargo run --example)
   The authoritative name->command decoder is `KIND_RUNNERS`/`decode()` in scripts/flakiness_stress.py.

2. STRESS HARNESS — `scripts/flakiness_stress.py`:
   - `one <name> --runs N --concurrency K --timeout S [--record] [--classify CLASS --issue mtg-x]`
   - `stress-all --runs 3 --concurrency 4 [--record]` (bounded sweep; light defaults)
   - `list`
   Per-run outcome pass/fail/timeout; TIMEOUT distinct from FAIL so it classifies as env, not flake. BOUNDED concurrency (default min(4, nproc/2)) by design — oversubscription manufactures the timeout-under-load false-flakes we want to distinguish. Auto-classify: deterministic-pass / timeout-under-load / true-nondeterministic; known-desync set explicitly by a human linking the bug.

3. FLAKINESS DB — `experiment_results/flakiness_db.csv` (tracked, append-only, like perf_history.csv). Columns: timestamp, git_commit, git_depth, canonical_name, kind, runs, fails, timeouts, flakiness_pct, classification, issue, concurrency, notes.

4. REPORT — `scripts/flakiness_report.py`: counts by class, top flakers (latest-per-test), highlights unexplained true-nondeterministic rows. `--all` / `--class X` filters. Answers "is validate's redness real?": all-{timeout-under-load + tracked known-desync} == green-modulo-known-issues; any true-nondeterministic == needs investigation.

## Seeded classifications (current known flakers)
- validate.network_e2e.01_rogue_rogerbrand.3 + validate.network_e2e.monored.13 -> known-desync (mtg-vk4b7)
- validate.network_e2e.white_weenie.7 -> known-desync (mtg-273)
- validate.wasm_e2e.wasm_pack_install -> true-nondeterministic infra (mtg-577)
- validate.mtg-engine--determinism_e2e.all -> timeout-under-load (NOT a real flake — stops SIGTERM timeouts masquerading as failures)

## Verified
report + list + decode for all 5 kinds; bounded live stress demo of a shell test. Seeded DB clean (demo runs were not --record'd).

## Next / future
- Populate real measured rows via `stress-all --record` on an idle box (bigger N, --concurrency 1 for heavy tests).
- Optional NEW Makefile target (do not edit existing validate-* targets — owned by ci-split agent) to wire `stress-all` into a nightly job.
