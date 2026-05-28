---
title: Global test-flakiness tracking system (canonical names + stress harness + flakiness DB)
status: open
priority: 2
issue_type: task
created_at: 2026-05-28T18:39:41.006380697+00:00
updated_at: 2026-05-28T18:39:41.006380697+00:00
---

# Description

Build a GLOBAL flakiness-tracking system for the whole validation suite. User goal (2026-05-28): "have a clear flakiness state that models the flakiness of our individual tests" — we currently have NO global naming/tracking for the bits of validation.

PIECES:
1. CANONICAL TEST IDENTITY: a stable name chaining identifiers that uniquely names every validation unit across all kinds — e.g. `validate.<binary>.<test>` for cargo/nextest tests, `validate.shell_script_tests.<script>` for tests/*.sh, `validate.wasm_e2e.<file>` for web/test_*.js, `validate.network_e2e.<deck>.<seed>` for the multi-deck network e2e. One namespace covering nextest cases, shell scripts, wasm/playwright e2e, network e2e, examples.
2. STRESS HARNESS: given a canonical test name, run it in ISOLATION N times (controlled concurrency so we measure the test, not contention), record pass/fail each run -> flakiness rate. Support a "stress all" sweep.
3. FLAKINESS DB: a tracked file (CSV/JSON, like experiment_results/perf_history.csv) recording per-test flakiness over time + a classification: DETERMINISTIC-PASS / TIMEOUT-UNDER-LOAD (env, not a real flake) / TRUE-NONDETERMINISTIC / KNOWN-DESYNC(bug). Stamped with commit SHA/date.
4. GLOBAL PICTURE: a report summarizing suite flakiness (how many tests, % flaky, by class) — a dashboard like scripts/temp/oldschool_progress.py.

WHY NOW: the All Hallow's validate showed "16 failed" that were actually SIGTERM timeouts (contention, NOT real flakes), alongside REAL network desyncs (mtg-vk4b7). Without a flakiness model we can't tell env-timeout-flake from true-nondeterminism from real-bug at a glance. This directly supports getting + keeping validate GREEN+STABLE.

RUNS CONCURRENTLY WITH: mtg-578 (CI split + tent-pole reduction) and aggressive investigation of currently-flaking tests (mtg-vk4b7 rogerbrand/monored desync, mtg-273 white_weenie seed7, mtg-577 wasm-pack race). Existing related: mtg-89/mtg-103 (snapshot stress tests) — reuse patterns.

DELIVERABLES: canonical-name scheme doc + a `scripts/` stress-harness (run-one-N-times + stress-all) + the flakiness DB + a report script. Classify the current known flakers to seed the DB.
