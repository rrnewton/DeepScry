---
title: 'CI Test job: cargo test is serial+slow; Cargo.lock changes cause cold-cache 50min+ runs (migrate to nextest)'
status: open
priority: 2
issue_type: task
created_at: 2026-05-28T05:24:28.687364504+00:00
updated_at: 2026-05-28T05:24:28.687364504+00:00
---

# Description

## Problem

The CI `Test` job runs `cargo test --verbose --workspace --features network` (see `.github/workflows/ci.yml` step "Run tests"). Plain `cargo test` runs tests SERIALLY within each test binary. Two binaries dominate wall-clock:

- `tests/determinism_e2e.rs`: a `dir_test` over `decks/**/*.dck` (65 deck files), each running a full deterministic game twice. Measured 495s on a fast 16-core box; far longer on the 2-vCPU GitHub runner.
- `tests/shell_script_tests.rs`: runs all 23 `tests/*.sh` e2e scripts (several rebuild the release binary mid-test, full games). Measured 1046s (17min) serially.

These run in `cargo test`'s single-binary-serial model with NO per-test timeout (unlike `cargo nextest`, which `make validate` uses and which parallelizes across all tests + has a slow-timeout).

## Symptom that triggered this (2026-05-27)

On feature branch `deploy-healthcheck-and-probes` (adds rustls/ring deps via `tokio-tungstenite rustls-tls-webpki-roots` + `rustls` in the `network` feature), the CI `Test` job ran 51-62 min and was cancelled twice, looking like a "hang". Root cause: NOT a deadlock. The branch changed `Cargo.lock` (added ring, rustls, rustls-webpki, tokio-rustls, hyper-rustls), which changes the `actions/cache` key `hashFiles('**/Cargo.lock')` => COLD cache miss => step 9 "Run tests" had to compile the ENTIRE debug test profile (all ~80 test binaries + ring + rustls) from scratch on a 2-vCPU runner, on top of the already-slow ~25min serial test execution.

Evidence:
- Integration `Test` job "Run tests" step: ~6.5 min (warm `Cache cargo build` restored in 1.2min).
- This branch attempt-1: "Run tests" 49+ min before cancellation at 62min total.
- Local reproduction of the EXACT command `cargo test --workspace --features network` on a warm target: completed GREEN (exit 0) in ~30 min; determinism_e2e=495s, shell_script_tests=1046s, network_e2e 13/13 pass.

So the cold-compile is a ONE-TIME cost (cache warms after one full run); the serial slowness is the persistent underlying issue.

## Fix options

1. ~~Migrate the CI Test step to `cargo nextest run`~~ — **TRIED 2026-05-28, REVERTED (made it WORSE), see update below.**
2. **Split the slow binaries (`determinism_e2e`, `shell_script_tests`) into their own CI job(s)** so they run concurrently with the fast unit tests on separate runners. THIS is now the recommended fix.
3. Consider trimming the `decks/**/*.dck` determinism matrix or running a representative subset on PR + full set nightly.
4. Use a larger GitHub runner (4-8 vCPU) for the Test job so parallel test execution actually has cores.

## 2026-05-28 UPDATE — nextest was tried on CI and REVERTED

Migrated the CI Test step to `cargo nextest run --workspace --features network` (commit 43f2fa11 on `deploy-healthcheck-and-probes`) and watched it on a real 2-vCPU `ubuntu-latest` runner (cold cache, run 26559653726):

- `Install cargo-nextest` (prebuilt tarball): instant, fine.
- `Build release binary`: 6 min.
- `Run tests` (`cargo nextest run`): still going at **62+ min** (total job >70min) when cancelled — **LONGER than the serial `cargo test` baseline (~49min cold)**.

Root cause of the regression: `shell_script_tests` each spawn a full `mtg` game SUBPROCESS. nextest runs tests across ALL binaries concurrently (up to test-threads), so on a 2-vCPU runner it oversubscribes the cores with many heavy game subprocesses → thrashing. Worse, this is *test-execution* cost (not one-time build), so nextest would make integration's Test job permanently slow even with a warm cache. **Reverted to serial `cargo test`** (the repo's proven baseline: ~6.5min warm, ~49min cold-once).

Lesson: nextest's cross-binary parallelism is a win on many-core dev boxes (that's why `make validate` uses it) but a LOSS on a 2-vCPU CI runner for subprocess-heavy e2e tests. The right fix is structural (option 2: split the slow shell/determinism binaries into a separate parallel CI job, or option 4: bigger runner), not a test-runner swap.

## Note for future agents

A 50min+ `Test` job after a dependency (`Cargo.lock`) change is most likely a cold cargo-build cache miss, not a code deadlock. Verify by running `cargo nextest run` / `cargo test --workspace --features network` locally with a hard `timeout` — if it completes, there is no hang.
