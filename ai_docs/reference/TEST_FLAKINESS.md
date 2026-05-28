# Global Test-Flakiness Tracking

Status: living reference. Implements the design in `mtg-j010x`.

This document defines a **canonical name** for every validation unit in the
repo, the **stress harness** that measures how flaky each unit is, the
**flakiness DB** that records the measurements over time, and the **report**
that summarizes the suite.

The motivating problem (2026-05): a `make validate` run can show "16 failed"
where most of those are SIGTERM **timeouts under load** (CPU contention, not a
real flake), mixed in with a couple of **real** network desyncs. Without a
naming scheme and a flakiness model we cannot tell, at a glance, the difference
between:

- an **env-timeout-flake** (test is fine; the box was saturated), versus
- a **true-nondeterministic** flake (test genuinely varies run-to-run), versus
- a **known-desync / real bug** (the engine is wrong; has a tracking issue).

This system gives us that model so "is validate green?" has a precise answer.

---

## 1. Canonical test identity

Every validation unit gets a single stable name in one flat namespace, rooted
at `validate.`. The name chains identifiers from coarse to fine. It is stable
(does not depend on machine, core count, or run order) and unique.

| Kind | Pattern | Example |
|------|---------|---------|
| Cargo / nextest test | `validate.<binary>.<test_path>` | `validate.mtg-engine--determinism_e2e.determinism_holds` |
| Shell-script test (`tests/*.sh`, auto-discovered by `shell_script_tests.rs`) | `validate.shell_script_tests.<script_stem>` | `validate.shell_script_tests.commander_e2e` |
| WASM / Playwright browser e2e (`web/test_*.js`) | `validate.wasm_e2e.<file_stem>` | `validate.wasm_e2e.test_fancy_tui` |
| Multi-deck network e2e (one deck+seed scenario) | `validate.network_e2e.<deck_stem>.<seed>` | `validate.network_e2e.rogerbrand3.3` |
| Example program (`*/examples/*.rs`) | `validate.examples.<example_name>` | `validate.examples.lightning_bolt_game` |

Notes on each kind:

- **Cargo/nextest** (`<binary>`): the test binary, written with `--` between
  the package and the integration-test name so it round-trips to a real cargo
  invocation: `mtg-engine--determinism_e2e` ->
  `cargo test -p mtg-engine --test determinism_e2e`. Unit tests inside a crate
  use `<crate>--lib`. `<test_path>` is the full `module::test_fn` path that
  `cargo test -- --exact <test_path>` accepts.
- **shell_script_tests**: discovered by `mtg-engine/tests/shell_script_tests.rs`
  via the `dir-test` glob `tests/*.sh`. `<script_stem>` is the filename without
  `.sh`. The same stem is what `cargo test --test shell_script_tests --
  shell_scripts__<stem>` selects.
- **wasm_e2e**: the `web/test_*.js` files invoked under `validate-wasm-e2e-step`
  and `validate-network-e2e-step`. `<file_stem>` is the filename without `.js`.
- **network_e2e**: the per-scenario rows in `web/test_network_multideck.js`
  (and `tests/network_vs_local_equivalence_e2e.sh`). One name per `(deck, seed)`
  pair so that, e.g., `rogerbrand3` seed 3 and `monored` seed 13 are *separately*
  trackable -- this is exactly the granularity at which desyncs show up.
  `<deck_stem>` is the deck filename without `.dck` (path stripped).
- **examples**: discovered by `scripts/run_examples.sh` / `cargo run --example`.

A canonical name is parsed back into a runnable command by the stress harness
(`scripts/flakiness_stress.py`); see `KIND_RUNNERS` there for the authoritative
name -> command mapping.

---

## 2. Stress harness

`scripts/flakiness_stress.py` runs a single canonical test **in isolation** N
times and records pass/fail per run, yielding a flakiness rate.

```
# Stress one test 20 times, 4 at a time, append result to the DB:
scripts/flakiness_stress.py one validate.shell_script_tests.commander_e2e \
    --runs 20 --concurrency 4 --record

# Stress every known test once (cheap suite smoke), bounded concurrency:
scripts/flakiness_stress.py stress-all --runs 3 --concurrency 4 --record

# List the canonical names the harness currently knows about:
scripts/flakiness_stress.py list
```

Key flags:

- `--runs N` how many isolated repetitions (default 10).
- `--concurrency K` how many run in parallel (default = min(4, nproc//2)).
  **Keep this bounded.** The whole point is to measure the *test*, not CPU
  contention -- if you oversubscribe you manufacture exactly the
  timeout-under-load false-flakes you are trying to distinguish. For a clean
  measurement of a heavy test, use `--concurrency 1`.
- `--timeout SECS` per-run wall-clock cap (default 300). A run that exceeds it
  is recorded as a `timeout` outcome (distinct from a `fail`), which is the
  signal that classifies as `timeout-under-load`.
- `--record` append a row to the flakiness DB (default: print only).
- `--classify CLASS` override the auto-classification when recording.

**CPU courtesy:** `stress-all` defaults to `--runs 3` and bounded concurrency so
a full sweep stays light. For a serious flakiness sweep (e.g. 100 runs of a
single suspect test) run it overnight on an idle box and pin `--concurrency 1`.

### Classification rule (auto)

After the N runs the harness assigns one of:

- `deterministic-pass` -- 0 fails, 0 timeouts.
- `timeout-under-load` -- some runs hit the wall-clock timeout but the
  non-timeout runs all passed. This is an **env** signal, not a real flake.
- `true-nondeterministic` -- genuine pass/fail variation that is not explained
  by timeouts.
- `known-desync` -- network-determinism failure with a tracking issue; set
  explicitly with `--classify known-desync --issue mtg-xxxx` (the harness never
  auto-promotes a fail to `known-desync` -- a human links the bug).

---

## 3. Flakiness DB

`experiment_results/flakiness_db.csv` -- one row per (test, measurement).
Tracked in git, append-only (like `perf_history.csv`).

Columns:

| Column | Meaning |
|--------|---------|
| `timestamp` | ISO-8601 UTC of the measurement |
| `git_commit` | short SHA under test |
| `git_depth` | `git rev-list --count HEAD` |
| `cpu` | host CPU id, same convention as the benchmark dirs (`scripts/run_benchmark.sh get_cpu_name`, e.g. `AMD_Ryzen_7_9800X3D_8-Core_Processor`). Flakiness (esp. `timeout-under-load`) is core-count sensitive, so every row records its host. |
| `canonical_name` | the `validate.*` name |
| `kind` | one of `cargo`/`shell_script_tests`/`wasm_e2e`/`network_e2e`/`examples` |
| `runs` | N repetitions |
| `fails` | count of failed (non-timeout) runs |
| `timeouts` | count of runs that hit the wall-clock cap |
| `flakiness_pct` | `100 * (fails + timeouts) / runs`, 2 dp |
| `classification` | `deterministic-pass`/`timeout-under-load`/`true-nondeterministic`/`known-desync` |
| `issue` | linked beads issue (e.g. `mtg-vk4b7`) or empty |
| `concurrency` | concurrency used for the measurement (context for timeouts) |
| `notes` | freeform |

---

## 4. Report

`scripts/flakiness_report.py` reads the DB and prints a dashboard: counts by
class, the top flakers (latest measurement per test), and any tests classified
as real bugs. Run it from anywhere in the repo:

```
scripts/flakiness_report.py                 # latest-per-test summary
scripts/flakiness_report.py --all           # every row, not just latest
scripts/flakiness_report.py --class known-desync   # filter to one class
```

The report is the thing a coordinator looks at to answer "is validate's
redness real?" -- a run that is all `timeout-under-load` + `known-desync`
(already-tracked) is *green-modulo-known-issues*, whereas any
`true-nondeterministic` row is an unexplained flake that needs investigation.

---

## Known-flaker registry (NOT measurement rows)

This is the curated list of tests we *already know* are flaky and why. It is
documentation, **not** DB data: the DB ships empty (header only) and only gets
rows from actual stress runs (`flakiness_stress.py ... --record`). Seeding the
CSV with `runs=0,fails=0,timeouts=0` rows was a bad idea — a zero-run row is
not a measurement, it just pollutes `flakiness_report.py`'s
latest-measurement-per-test logic with fake "0% flaky" data. Keep the knowledge
here; let the CSV hold only real numbers.

To turn a registry entry into real DB data, stress it and record with the
matching `--classify`/`--issue`, e.g.:

```
scripts/flakiness_stress.py one validate.network_e2e.01_rogue_rogerbrand.3 \
    --runs 20 --concurrency 1 --classify known-desync --issue mtg-vk4b7 --record
```

| canonical_name | class | issue | why |
|---|---|---|---|
| `validate.network_e2e.01_rogue_rogerbrand.3` + `validate.network_e2e.monored.13` | `known-desync` | mtg-vk4b7 | rogerbrand3 / monored network desync (real engine bug) |
| `validate.network_e2e.white_weenie.7` | `known-desync` | mtg-273 | white_weenie seed 7 desync; excluded from multideck quick set |
| `validate.wasm_e2e.<wasm-pack-install>` (infra) | `true-nondeterministic` | mtg-577 | wasm-pack install race during the wasm build step |
| `validate.mtg-engine--determinism_e2e.*` | `timeout-under-load` | (none) | SIGTERM timeouts under contention -- NOT a real flake |

The last row is the important one: those determinism_e2e SIGTERMs were being
counted as failures. Classifying them as `timeout-under-load` stops them from
masquerading as real flakes.
