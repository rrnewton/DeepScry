# `make validate` vs GitHub Actions CI — Divergence Audit

**Timestamp:** 2026-05-26_#2291(a13b311f)
**Branch:** `audit-validate-ci-gap` based on `integration` at `a13b311f`.
**Author:** automated audit agent.

## TL;DR

1. **CI has been red on `integration` for ~12 days** (since `61e066882b`,
   2026-05-15). The team has been ignoring red CI. Beads issues
   `mtg-c232f4` and `mtg-ivrqv` describe the failure modes; both have
   landed fixes (`a13b311f`, `6fc7f945`), but a third regression
   (`mtg-nufig`) remains open and keeps the `Test` job red.
2. **The shell-script test harness (`mtg-engine/tests/shell_script_tests.rs`)
   auto-discovers every `tests/*.sh` and runs it as a Rust test.** This
   means CI *was already running* the three "missing" tests via
   `cargo test --workspace`. The orchestrator's working hypothesis
   "local validate runs tests CI doesn't" is mostly wrong — the
   coverage is there, what was missing is that the team was not
   *reading* the CI status.
3. **Real gaps** (steps in local `make validate` that CI genuinely
   does not exercise) are documented in
   [Structural map](#structural-map) below. Most important: CI runs
   `cargo test --workspace` without `--features network`, so any
   `#[cfg(feature = "network")]` Rust integration test is silently
   skipped. The shell scripts are unaffected because each script
   `cargo build --release --features network`s its own binary.

## Test-by-test first-broken commits

| Test | First broken commit | Date | Why |
|------|----------------------|------|-----|
| `tests/snapshot_resume_e2e.sh` (Phase 3 bincode) | `61e066882b fix(rng): centralize seed derivation…` | 2026-05-15 14:04 UTC | Added `#[serde(tag = "controller_type")]` to `ControllerState` in `mtg-engine/src/game/snapshot.rs`. Bincode rejects `deserialize_any`, which internally-tagged enums require. Test was added the previous day (`21ff552a`, 2026-05-14) and was passing in CI; the very next snapshot.rs touch broke it. |
| `tests/network_vs_local_equivalence_e2e.sh` | `67f046f08e feat(server): multi-game lobby…` | 2026-05-15 06:27 UTC | Replaced single-game server lifecycle with long-lived lobby. Test was polling `kill -0 $SERVER_PID` and treating server exit as "game done"; with the lobby server, `SERVER_PID` never exits → 180 s timeout, exit 1. |
| `tests/cycle_ability_network_sync_e2e.sh` | `67f046f08e` (same — it wraps the equivalence harness) | 2026-05-15 06:27 UTC | Same lobby-lifecycle hang masks an earlier real gamelog desync; the hang fix (`9101ba6f`) re-exposes the desync. The underlying desync regression is filed as **mtg-nufig** and is in-scope for the parallel `fix-mtg-nufig` worktree. |

Both root causes were introduced on **the same day** (2026-05-15), and
CI red on `integration` started on that day. The first-broken merge
into `integration` after the last-green run (`ceea322e5a`,
2026-05-15 13:36) is `61e066882b` (2026-05-15 14:04) — which broke
the snapshot test. The network test had already broken at
`67f046f08e` (2026-05-15 06:27) on its source branch, but
`67f046f08e` itself merged into `integration` only as part of
`4e1a7f3d` later that day.

## Structural map

`make validate` (sequential order) vs CI workflow jobs/steps:

| Make step | CI job/step | Coverage |
|-----------|--------------|----------|
| `validate-fmt-step` (`cargo fmt --check`) | `fmt` job: `cargo fmt --all -- --check` | EQUIVALENT |
| `validate-clippy-step` (3 feature combos: native+net, wasm,network, mtg-benchmarks) | `clippy` job: only `mtg-forge-rs --all-features` + `mtg-benchmarks` | **GAP**: CI omits the explicit `--features wasm,network` invocation. |
| `validate-clippy-wasm-step` (clippy on `wasm32-unknown-unknown` target with `wasm-tui`) | (none) | **GAP**: wasm-target-specific clippy lints never run in CI. |
| `validate-test-step` (`cargo nextest run --features network`) | `test` job: `cargo test --verbose --workspace` (no `--features network`) | **PARTIAL**: shell tests run regardless because they self-build, but Rust `#[cfg(feature = "network")]` integration tests are skipped in CI. |
| `validate-examples-step` (`scripts/run_examples.sh`) | `test` job: `Run examples` | EQUIVALENT |
| `validate-agentplay-step` (pytest + agent_game --mock + `scripts/test_mode_equivalence.sh`) | `test` job: pytest + agent_game --mock | **GAP**: CI does not run `test_mode_equivalence.sh`. |
| `validate-commander-step` (`tests/commander_e2e.sh`) | `test` job: `Run commander E2E test` (also auto-discovered) | EQUIVALENT (duplicated). |
| `validate-snapshot-resume-step` (`tests/snapshot_resume_e2e.sh`) | auto-discovered via `shell_script_tests.rs` | EQUIVALENT in coverage; **but no dedicated CI step name**, so failures show as `shell_scripts__snapshot_resume_e2e` inside the omnibus `cargo test` run. |
| `validate-wasm-step` (`make wasm-dev`) | `wasm` job: `Build WASM (dev)` | EQUIVALENT |
| `validate-wasm-e2e-step` (9 node tests: fancy_tui, human_input, click_and_log, font_size_layout, decouple_step3_launch_game_session, card_size_stability, decouple_step6_valid_choices, tapped_rotation, graveyard_overlay) | `wasm` job: only `test_fancy_tui.js` + `test_human_input.js` | **GAP**: 7 of 9 wasm e2e tests omitted from CI. |
| `validate-network-e2e-step` (3 Playwright network tests) | `network` job: 3 Playwright network tests | EQUIVALENT |

### `tests/*_e2e.sh` shell tests wired into CI

ALL of them — automatically — via `mtg-engine/tests/shell_script_tests.rs`
(uses `dir-test` crate, glob `*.sh` in `$CARGO_MANIFEST_DIR/../tests`).
Each becomes a `shell_scripts__<name>` test inside the omnibus
`cargo test --workspace` run.

This is a hidden but important property: **a new `tests/foo_e2e.sh`
is automatically wired into CI without any workflow change.** The
flip side: failures are buried inside the giant `cargo test` log
and produce a single job-level red, not a per-test signal.

## Bisect notes

Targeted manual verification at HEAD `a13b311f`:

- `bash tests/snapshot_resume_e2e.sh` → **7/7 PASS** (bincode fix `cfe4f256` landed).
- `bash tests/network_vs_local_equivalence_e2e.sh` → **PASS** in ~20s (wait-loop fix `9101ba6f` landed).
- `bash tests/cycle_ability_network_sync_e2e.sh` → **FAIL** at ~21s with "LOCAL and SERVER gamelogs differ by 232 lines" (mtg-nufig regression, expected — being fixed in `fix-mtg-nufig` parallel worktree).

Historical bisect was unnecessary because the commit-level forensics
above pinpoint the breakage exactly (one snapshot.rs edit in
`61e06688`, one server lifecycle change in `67f046f0`); the dates
are corroborated by GitHub Actions run history showing CI flipping
from `success` to `failure` on `61e066882b` and never recovering.

## What should worry us

1. **Process failure, not coverage failure.** The CI was correctly
   surfacing all three bugs the whole time. Nobody was looking at it.
   Of the gaps identified in the structural map above, the snapshot
   regression and the network test regressions were NOT among them —
   the auto-discovered shell-test harness was catching them. Closing
   the structural gaps would NOT have prevented the
   "discovered-late" story; reading CI status would have.
2. **`cargo test` without `--features network` is a real coverage
   hole** for any Rust integration tests behind that feature flag.
   `cargo nextest run --features network` (what `make test` does)
   exercises a strictly larger set.
3. **WASM e2e suite divergence (7 of 9 omitted)** is the largest
   structural gap. Several of these tests cover the rebuilt thin-DOM
   game GUI and click/log paths that have shipped real regressions
   in the past.
4. **Single `Test` job is a monolithic bottleneck.** All shell
   scripts serialize inside one `cargo test` invocation. As more
   network shell tests are added (likely), a single slow/flaky test
   can block the whole signal. Worth considering a split into
   multiple jobs (Rust-only tests, shell-script tests, agentplay
   tests) for parallelism and clearer per-failure attribution.
5. **No-disk-space failure observed at `6db45ef1`** (mentioned by
   orchestrator). Not reproduced in this audit, but the cache
   strategy (`actions/cache@v4` keyed on `Cargo.lock`) will grow
   monotonically until the cache is invalidated; adding more
   binary-producing jobs without an explicit cleanup step risks
   this happening more often.
6. **Node.js 20 deprecation warning in every job.** GitHub will
   force Node 24 on 2026-06-02; the `actions/*@v4` actions will
   need updating soon to avoid forced surprises.

## Proposed CI workflow changes

See the workflow patch committed alongside this audit. Summary:

- Add `--features network` to the `Test` job's `cargo test`
  invocation (single-line change, big coverage gain).
- Add a `clippy-wasm` step inside the existing `clippy` job (one
  toolchain target install + one cargo invocation).
- Run all 9 wasm e2e tests in the `wasm` job (no new job needed).
- Run `scripts/test_mode_equivalence.sh` in the `Test` job's
  agentplay step.
- Add a disk-cleanup step at the top of each job to reclaim runner
  space (gha-cleanup style).
- Leave the shell-script auto-discovery alone; it is the right
  shape and has been catching regressions.

The cycle_ability test is currently failing on `integration` HEAD
(mtg-nufig); the parallel `fix-mtg-nufig` worktree is addressing it.
This audit does NOT mask or skip that test in CI — the right thing
is to land the underlying fix and let the existing CI red signal
clear naturally.
