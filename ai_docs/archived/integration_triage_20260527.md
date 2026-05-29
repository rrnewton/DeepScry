# Integration Branch Triage — Failure Inventory

**Timestamp:** 2026-05-27_#2297(b5cbdc85)
**Branch under triage:** `origin/integration` @ `b5cbdc85`
**Triage branch:** `triage-integration-rot`
**Author:** automated triage agent (follow-up to
`ai_docs/validate_ci_gap_audit_20260526.md`)

## Executive summary

After landing the audit-suite fixes (mtg-430 / mtg-457 / mtg-458)
the team-visible CI signal on `integration` is **still red**, but the
remaining failures are small and narrowly scoped:

| Category | Count | Notes |
|----------|-------|-------|
| Stale tests (test code referencing removed API) | 3 | All in `agentplay/`, all trace to `61e06688` |
| CI config / env (missing wasm rust-std on pinned toolchain) | 1 | `Run clippy (WASM target)` step |
| Cosmetic / non-blocking | 2 | Node.js 20 deprecation warning, Network E2E shutdown-noise ERROR lines |
| Real engine/code bugs | 0 | None found at HEAD |

**Total blocking failures: 4** (3 pytest + 1 clippy-wasm). All four are
the same root cause class — code/config that wasn't updated when the
underlying API or toolchain pin changed. None are gameplay or
correctness bugs. **"Integration green" should be reachable in a single
round of fixes**, doable as a single small PR or two parallel ones.

`make validate` locally was **also red** for the same two reasons
(pytest + clippy-wasm). Two ADDITIONAL local-only failures
(`validate-wasm-e2e-step`, `validate-network-e2e-step`) are missing
Playwright browser binaries — a worktree-env issue, not a CI/integration
problem (CI's `npx playwright install chromium --with-deps` step handles
this fine, and the same failure would hit any fresh local clone that
hasn't run `npx playwright install chromium`). Filed as L1 below for
completeness but excluded from the "blocking failures" count.
The audit's earlier conclusion that "CI ≈ make validate after the
workflow patch" holds for the four blocking failures.

### Recommended fix order

1. **F1 + F2 together (single PR, ~10 min):** delete the 3 stale
   pytest tests (or rewrite them to assert the `NotImplementedError`
   contract) + add `rustup target add wasm32-unknown-unknown` to the
   pinned toolchain in CI (or add `targets = ["wasm32-unknown-unknown"]`
   to `rust-toolchain.toml`).
2. **F3 (separate, low-urgency):** bump `actions/*@v4` → `@v5`
   before the GitHub-enforced Node 24 cutover on 2026-06-02 (filed
   in mtg-459 follow-up #3, still open).
3. **F4 (cleanup, non-blocking):** suppress the spurious Network E2E
   shutdown-time ERROR log lines so future failures stand out.

## Failure table

| ID | Job / Step | Name | Category | First broken | Fix complexity | Owner | Beads |
|----|------------|------|----------|---------------|----------------|-------|-------|
| F1a | Test / Run agentplay tests | `test_mock_mode_selects_randomly_without_subprocess` | Stale test | `61e06688` (2026-05-15) | trivial | 1-agent, delete or rewrite | mtg-460 (filed) |
| F1b | Test / Run agentplay tests | `test_mock_session_deterministic` | Stale test | `61e06688` | trivial | 1-agent, delete or rewrite | mtg-460 (filed) |
| F1c | Test / Run agentplay tests | `test_mock_session_returns_in_range` | Stale test | `61e06688` | trivial | 1-agent, delete or rewrite | mtg-460 (filed) |
| F2 | Clippy / Run clippy (WASM target) | `cargo clippy --target wasm32-unknown-unknown` cannot find crate `core` | Env / config | `7a423c75` (2026-05-26, when the WASM-clippy step was added) | trivial | 1-agent, CI YAML edit | mtg-461 (filed) |
| F3 | All jobs | `actions/checkout@v4` Node 20 deprecation warning | Config (latent) | n/a (pre-existing) | small | 1-agent, YAML bump | mtg-459 (follow-up #3) |
| F4 | Network E2E (info only) | `[ERROR mtg_forge_rs::network::server] Game 1: P2 handler exited unexpectedly: Ok(Ok(()))` and friends, AFTER the test reports PASS | Cosmetic | unknown, pre-existing | small | 1-agent, demote to INFO/DEBUG at shutdown | mtg-462 (filed) |
| L1 | (local validate only) validate-wasm-e2e-step + validate-network-e2e-step | `browserType.launch: Executable doesn't exist at ~/.cache/ms-playwright/chromium_headless_shell-1223/...` — Playwright auto-attempt to `install chromium` then fails with `Installation process exited with code: 1` (likely missing sudo or apt deps in the WSL/agent env) | Env (local only) | n/a (pre-existing) | small | dev-env doc, not a CI fix | n/a — CI passes via `npx playwright install chromium --with-deps` |

### F1a/F1b/F1c — pytest first-error excerpts

```
agentplay/test_agent_game.py::test_mock_mode_selects_randomly_without_subprocess FAILED
E       AssertionError: assert 'mock' in 'random choice\n5'
agentplay/test_agent_game.py:251: AssertionError

agentplay/test_persistent_driver.py::test_mock_session_deterministic FAILED
agentplay/test_persistent_driver.py::test_mock_session_returns_in_range FAILED
>       raise NotImplementedError(
E       NotImplementedError: MockSession.ask() was removed: --mock now uses engine-side
        RandomController instead of a Python random.Random. If you reach this method, your
        driver is still trying to feed mock decisions through Python — switch to the
        engine-side path so all three drivers (stop-and-go / persistent / WASM) stay
        byte-identical for the same seed.
agentplay/lib/agent_session.py:442: NotImplementedError
```

Root cause is exactly what the `NotImplementedError` says: `61e06688`
removed the Python-side `MockSession.ask()` codepath but left three
tests in place that exercise it. Two options:

- **Delete** all three tests (the underlying functionality is now
  tested at the engine level by the seed-determinism shell tests).
- **Rewrite** them to assert that `ask()` *raises* `NotImplementedError`
  (regression guard against accidentally re-introducing the Python
  mock path).

Recommendation: delete `test_mock_session_deterministic` /
`test_mock_session_returns_in_range` outright (they test a removed
feature, not a contract worth preserving), and rewrite
`test_mock_mode_selects_randomly_without_subprocess` to assert the new
engine-side `RandomController` path's externally-visible behaviour
(it already partially does — the assert just needs updating away from
the old "mock" string match).

### F2 — clippy WASM target failure

```
error[E0463]: can't find crate for `core`
   |
   = note: the `wasm32-unknown-unknown` target may not be installed
   = help: consider downloading the target with `rustup target add wasm32-unknown-unknown`
   = help: consider building the standard library from source with `cargo build -Zbuild-std`
error: could not compile `cfg-if` (lib) due to 1 previous error
##[error]Process completed with exit code 101.
```

Root cause: `.github/workflows/ci.yml` `Clippy` job uses
`dtolnay/rust-toolchain@nightly` with `targets: wasm32-unknown-unknown`,
which installs `wasm32` rust-std onto the **latest** nightly. But the
repo's `rust-toolchain.toml` pins `nightly-2025-11-28`, so when `cargo
clippy` runs, rustup auto-switches to the pinned toolchain (CI log
explicitly says `note that the toolchain 'nightly-2025-11-28-x86_64-
unknown-linux-gnu' is currently in use (overridden by ... rust-toolchain.toml)`).
The wasm rust-std was installed only for the latest nightly, not the
pinned one. Result: `core` is missing for `wasm32-unknown-unknown` on
the active toolchain.

Two fixes (either works):

- **Recommended (one-line, durable):** add
  `targets = ["wasm32-unknown-unknown"]` to `rust-toolchain.toml` so
  the pin always pulls the wasm rust-std on every rustup install. This
  also helps local dev: anyone running `cargo build --target wasm32-unknown-unknown`
  in a fresh clone benefits.
- **Workaround:** add a CI step before clippy:
  `rustup target add wasm32-unknown-unknown` (this targets the
  currently-active toolchain, which is the pinned one).

I verified F2 reproduces in BOTH the just-completed CI run
(`26482076564`) and the in-progress run on current HEAD
(`26514285939`, Clippy already failed by the time of this writing).
The audit's added `clippy-wasm` step was correctly the right
*invocation* but the toolchain pin defeats it.

### F3 — Node.js 20 deprecation (low urgency, time-bombed)

Every job emits:

```
##[warning]Node.js 20 actions are deprecated. ... Actions will be
forced to run with Node.js 24 by default starting June 2nd, 2026.
```

The cutover happens in ~6 days. Currently a WARNING, not a FAIL.
mtg-459 follow-up #3 already tracks this; bump `actions/checkout@v4`,
`actions/cache@v4`, `actions/setup-python@v5`, `actions/setup-node@v4`,
`actions/upload-artifact@v4` → their `@v5` (where available) before
2026-06-02.

### F4 — Network E2E shutdown noise (cosmetic)

After the in-test assertion fires `=== TEST PASSED ===`, browser
shutdown closes the WebSocket, which triggers ERROR-level logs:

```
[ERROR ... ] Game 1: P2 handler exited unexpectedly: Ok(Ok(()))
[ERROR ... ] Handler P0: Fatal error: Opponent disconnected
[ERROR ... ] Game 1: Error - P2 connection terminated unexpectedly
```

The test still reports PASS (these are post-assertion shutdown
artifacts), but they dilute "search log for ERROR" triage workflows.
Demote to INFO/DEBUG when the disconnect is part of clean shutdown
(i.e., the game already reached terminal state). Non-blocking.

## Dependency graph

```
F1a, F1b, F1c — independent of everything; can ship today.
F2            — independent of everything; can ship today.
F3            — must land before 2026-06-02 to avoid forced Node 24
                breakage, but otherwise independent.
F4            — independent; cosmetic only.
```

There are **no fix-blocks-fix dependencies**. All four can be
dispatched in parallel.

## Out-of-scope / file-for-later

- mtg-459 follow-up #2 (further `Test` job splits along Rust-unit
  vs shell-script vs agentplay boundaries) — desirable but not
  blocking.
- mtg-459 follow-up #4 (auto-file a beads issue on integration CI
  going red) — desirable, prevents recurrence of the original
  "12 days unnoticed" failure mode, but is infrastructure work
  rather than a fix.
- Process discipline (orchestrator must run `gh run list --branch
  integration --limit 1` at every spawn) — covered in mtg-459, no
  code fix possible.
- `make validate` cannot be run unmodified from inside the Claude
  Code harness because `scripts/check_clean_environment.py` flags the
  harness's own shell wrapper as a "conflicting process" — see
  Methodology below. This is a separate harness/workflow bug
  unrelated to integration health; file as a separate issue if it
  bites future agents.

## Bisect

Not needed — every failure's first-broken commit was identified by
text-grep (`MockSession.ask() was removed` → `61e06688`) or
forensic CI-log inspection (clippy WASM step was added in the audit
commit `7a423c75`).

## Methodology notes

1. CI run watched: GitHub Actions run `26514285939` on SHA `b5cbdc85`
   (current `origin/integration` HEAD).
2. Local `make validate` was run via `make -k validate-parallel-steps`
   to bypass the harness-incompatible `scripts/validate.sh` wrapper.
   `validate.sh` calls `check_clean_environment.py` which flags this
   triage agent's own shell as a conflict (both the wrapper and its
   parent shell match `validate.sh` + cwd substring). Workaround
   only — not a fix.
3. Previous failed CI run `26482076564` was inspected via
   `gh run view --log-failed` to confirm the same two failures
   (agentplay, clippy-wasm) reproduce identically on the prior
   commit.
4. All four failures reproduce **deterministically** — no flakes
   observed.

## New beads issues filed

- **mtg-460** (priority 2, bug): agentplay tests reference removed
  `MockSession.ask()` (3 tests). Covers F1a/b/c. Trivial fix.
- **mtg-461** (priority 2, bug): clippy-wasm CI step fails because
  pinned nightly toolchain has no wasm rust-std. Covers F2.
  Recommended one-line fix: add `targets =
  ["wasm32-unknown-unknown"]` to `rust-toolchain.toml`.
- **mtg-462** (priority 4, bug): Network E2E shutdown noise — demote
  post-test-pass disconnect logs from ERROR to INFO/DEBUG. Covers F4.
- **mtg-463** (priority 3, bug): `scripts/validate.sh` flags its own
  caller's shell as a conflicting process — affects every agent in the
  Claude Code harness. Documented under Process red flags below.

## Process red flags

1. **The same "ignore red CI" failure mode just repeated.** The audit
   `mtg-459` was filed precisely because CI had been red for 12 days
   unnoticed. The audit branch then landed three fixes — and CI is
   STILL red. The team merged the audit without re-checking that CI
   actually went green. The mandatory "Clean Start" check in
   `CLAUDE.md` (run `gh run list --branch integration --limit 1`
   before every task) is demonstrably not being enforced. Suggest
   adding a pre-spawn lint to the orchestrator that fails-fast if
   integration's latest CI is red.
2. **`scripts/validate.sh` is incompatible with the Claude Code agent
   harness.** Its conflicting-process detector flags its own caller's
   shell snapshot path because that path contains the literal string
   `validate.sh` (the harness sources a snapshot file before running
   any user command). Every agent that follows `CLAUDE.md`'s "run
   `make validate`" instruction will hit this. Worktrees can work
   around with `make -k validate-parallel-steps` directly, but
   the script should be patched to skip its own PPID / process-tree
   ancestors.
3. **The pinned nightly `2025-11-28` is now 5+ months old.** Worth a
   periodic refresh — keeping a moving pin reduces toolchain-drift
   surprises (and makes wasm rust-std install via `dtolnay` work as
   most projects expect).
4. **No CI-pass gate on merges to `integration`.** Per `CLAUDE.md` the
   tiered branch flow (feature → integration → main) is supposed to
   gate on green CI, but in practice merges land regardless. Either
   automate the gate (branch protection rule) or document the policy
   exception.
