---
name: validate-hygiene
description: >
  Principles and a re-audit procedure for keeping `make validate` FAST (all
  cores lit, minimal wall-clock) and its log CLEAN (terse by default,
  self-contained on failure). Use when validate has rotted — the log is noisy
  (raw game/server logs on stdout), the run is slow / single-cored, builds are
  duplicated, or CI has drifted from local — or when adding a new validation
  step and you want it to stay clean.
---

# validate-hygiene

`make validate` is the project's pre-commit gate. It rots predictably: new
steps dump raw output to stdout, builds get duplicated across sub-makes, and a
single long browser/network step starves the other 15 cores. This skill
encodes the principles that keep it clean and a repeatable audit to catch the
rot. Origin: mtg-717 (a 16,325-line, ~16-minute validate run, ~79% of it raw
network-game spew, machine at ~22% of one core).

## The architecture (know this first)

- `make validate` → `scripts/validate.sh` (caching, lock, WIP-commit, CPU
  monitor) → `make validate-impl` → `make -j<N> validate-parallel-steps`.
- `validate-parallel-steps` lists every `validate-<X>-step` as a prerequisite;
  `make -j` runs them concurrently EXCEPT where a step declares another step as
  a prerequisite (that forces a serial edge).
- Each `validate-<X>-step` body routes its commands through
  **`scripts/validate_step.sh`** (the `$(VSTEP)` make variable).
- CI (`.github/workflows/ci.yml`) must run the SAME work, sharded one GitHub
  job per **jobGroup** (see below), calling the same make targets — never a
  hand-re-derived copy that drifts.

## Principles

1. **Build once.** The release `mtg --features network` binary is on the
   critical path of almost everything. Compile it ONCE, up front; every
   downstream step reuses it via `MTG_REUSE_PREBUILT=1` (see
   `tests/lib/test_helpers.sh`, `determinism_e2e.rs`). Data exports
   (`export-wasm`) are feature-independent — export once. Duplicate
   `Compiling mtg-engine` / `Export complete!` lines mid-log are the smell.
   Distinct feature sets (clippy all-features vs wasm,network vs wasm32
   target; wasm-dev vs wasm-network bundles) are legitimately separate builds —
   don't try to merge those.

2. **Detail to a file; terse to the log.** A step's detailed output
   (compiler spew, per-game logs, per-request server logs, browser console,
   per-deck PASS lines) goes to a per-step file, NOT stdout. The validate log
   shows only one `START` line and one `PASS/FAIL (Ns)` line per step. The
   `$(VSTEP)` wrapper does this for you — never echo raw test output directly.

3. **Dump-on-fail = self-contained failure.** On failure the wrapper dumps the
   captured detail INTO the log (tagged), so a red run never requires a re-run
   to diagnose. Tests must hard-fail (exit non-zero), never green-skip.

4. **`[jobGroup.jobId]` 3-level tagging.** Every emitted line is prefixed
   `[jobGroup.jobId]`. Three levels are globally unique and plenty:
   - **jobGroup** = top-level CI shard (`lint`, `unit`, `examples`,
     `agentplay`, `determ`, `wasm`, `network`).
   - **jobId** = one distinct test workflow within a group (`wasm.browser`,
     `network.multideck`, `network.equiv-random`).
   - **testName** = the innermost case (a nextest `mtg-engine game::foo::bar`,
     a per-deck sweep line) — lives INSIDE the per-step detail file, not in the
     terse log.
   Interleaving under `make -j` is FINE and expected; tags make it grep-able
   (`grep '\[network\.' log`) while real completion order is preserved.

5. **Design FOR parallelism; start the long tail EARLY.** Identify the single
   longest-wall-clock path (today: the wasm→network browser chain) and start it
   as early as possible so its single-threaded tail overlaps all the CPU-bound
   Rust work, instead of trailing after it while cores idle. Break up long
   sequential phases; run independent browser tests concurrently; minimise the
   unavoidable start/end fork-join where few cores are busy. The `-j` width
   should track core count, not a hardcoded 4.

6. **Verbose mode is opt-in, terse is default.** `VALIDATE_VERBOSE=1` streams
   tagged detail live; `VALIDATE_VERBOSE_DIR=<dir>` persists every step's detail
   as `<group>.<job>.log`. Default runs stay terse.

7. **CI shards mirror local — never re-derive.** CI parallelism = one job per
   jobGroup, each invoking the SAME `make validate-group-<X>` target local uses.
   If you change a step locally, CI inherits it. Hand-maintained CI step lists
   drift from local and silently lose coverage (mtg-717 found CI running 2 of 9
   wasm tests at one point).

8. **Consolidate only where genuinely orthogonal-safe.** Before merging two
   "similar" test clusters, confirm they cover different invariants. The
   equivalence clusters are deliberately distinct: native-vs-WASM sweep
   (cross-compile-target determinism), network-vs-local equivalence (network
   determinism, mtg-380), whole-game rewind/replay. The documented dedup
   rationale lives in `tests/fuzz_determinism_netequiv_e2e.sh`'s header. Prefer
   shrinking wall-clock (turn caps, concurrency) over deleting coverage.

## Adding a new validation step (checklist)

- Wrap the command(s) with `$(VSTEP) <group> <job> "<description>" -- <cmd>`.
  Assign it to one of the existing jobGroups (or define a new top-level one AND
  add the matching CI shard).
- Reuse the prebuilt binary (`MTG_REUSE_PREBUILT=1`) — never add a fresh
  `cargo build` inside a step if the up-front build already produced what you
  need.
- Add it to BOTH `validate-parallel-steps` and `validate-parallel-steps-no-network`
  (if non-network) and to the sequential variants, and to the CI shard for its
  jobGroup.
- Never `git add` images / screenshots / large artifacts a step produces —
  send them to gitignored `debug/` or `scratch/`.

## RE-AUDIT procedure (run when validate feels slow or noisy)

1. **Capture a real run with timing + verbose detail:**
   ```sh
   VALIDATE_VERBOSE_DIR=validate_logs/verbose make validate ARGS=--force
   ```
   Keep the `validate_logs/validate_<sha>.log` and the per-step verbose dir.

2. **Diagram the parallelism.** From the Makefile, list every
   `validate-<X>-step` and its prerequisites. Prerequisite edges between steps
   are the SERIAL chain; everything else is concurrent under `-j`. Draw the
   fork/join and mark the longest prerequisite chain — that is the critical
   path.

3. **Quantify wall-clock per step.** With the `$(VSTEP)` `PASS (Ns)` durations
   (grep `✓ PASS` in the log), rank steps by duration. The top one or two ARE
   the critical path. Cross-check against the CPU-utilization report at the end
   of the run (low average % + a long tail = a single-threaded long pole
   starving cores — the canonical symptom).

4. **Detect duplicate builds.** In a verbose run:
   ```sh
   grep -c 'Compiling mtg-engine' validate_logs/verbose/*.log
   grep -rc 'Export complete' validate_logs/verbose/
   ```
   More than one release+network `mtg` build, or more than one `export-wasm`,
   is a build-once violation — trace which jobIds rebuild and route them to the
   shared prebuilt artifact.

5. **Detect log leaks.** Confirm the terse log has no raw game/server output:
   ```sh
   grep -cE 'NativeAI:|Server:|Life:|Turn [0-9]|INFO  mtg' validate_logs/validate_<sha>.log
   ```
   A non-trivial count means some step bypasses `$(VSTEP)` and echoes raw
   detail to stdout — wrap it.

6. **Check CI parity.** Diff the set of commands CI runs against the local
   per-group make targets. Any test in local-but-not-CI (or vice versa) is
   drift — fix CI to call the shared target.

7. **Fix highest-leverage first.** Order: kill the idle tail (parallelism /
   start-long-pole-early / shrink the longest step) → build-once → log hygiene →
   CI parity. Each fix must keep `make validate` green; cite the resulting
   `validate_logs/validate_<sha>.log`.
