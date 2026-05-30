# Bug Finding Harness

This directory contains the randomized / fuzz / stress harnesses used to hunt
for bugs by exploring many random game configurations. Unlike the deterministic
regression tests in `tests/` (wired into `make validate` and CI), these tools
run for a long time and are meant to be run **periodically** (or overnight) to
surface new bugs, then file a beads issue + a fixed-seed reproducer per finding.

See [`../docs/FUZZ_AND_STRESS_TESTING_STRATEGY.md`](../docs/FUZZ_AND_STRESS_TESTING_STRATEGY.md)
for the full bug-finding-vs-regression policy, the complete harness inventory
table (expedition ↔ validate-counterpart mapping), and the shared-helper layer.

## The rule (one line)

> `make validate` = deterministic; a fixed-seed SHORT randomized leg is OK
> there. Anything that sweeps many random seeds / runs for hours is a
> bug-finding **expedition** and lives HERE, NOT in validate.

## Requirements for bug-finding scripts

1. Run for a configurable duration (fixed batch or run-until-interrupted).
2. Handle Ctrl-C gracefully (print a summary on SIGINT).
3. Print a summary on exit: total tested, pass/fail counts, failure categories,
   and a reproducer command per unique failure.
4. Save failure logs to temp dirs (or gitignored `debug/`) for triage.

## Shared helpers (single source of truth — do NOT reimplement)

- `network_test_lib.py` — the ONE Python implementation of `run_local_game`,
  `run_network_game` (loopback server + native/WASM clients), `run_equivalence_test`,
  gamelog `extract_gamelog`/`compare_gamelogs`, and error classification.
  Imported by `network_fuzz_test.py` and by `tests/network_vs_local_equivalence.py`.
- `lib/gamelog_filter.sh` — the ONE bash `[GAMELOG ...]` extraction/filter
  (`gamelog_filter`, `gamelog_filter_file`). Behaviourally identical to
  `extract_gamelog`. Sourced by `fuzz_determinism_netequiv.sh` and
  `tests/network_vs_local_equivalence_e2e.sh`.
- `lib/seed_salts.sh` — the ONE bash mirror of the Rust per-player seed salts
  (`mtg-engine/src/game/seed_derivation.rs`): `derive_p1_seed`/`derive_p2_seed`.
  Sourced by the same two scripts. Local-vs-network equivalence depends on
  these matching Rust exactly.

## Expedition scripts (NOT in validate)

### `fuzz_determinism_netequiv.sh`
Sweeps many random seeds checking (1) native determinism (same seed twice ->
identical gamelog) and (2) local-vs-network equivalence. Uses the shared bash
gamelog filter + seed salts.
```
bash bug_finding/fuzz_determinism_netequiv.sh --seeds 40 --pair-mode all
bash bug_finding/fuzz_determinism_netequiv.sh --invariant determinism --decks 'decks/old_school2/*.dck' --seeds 20
```
Bounded validate counterpart: `tests/fuzz_determinism_netequiv_e2e.sh`.

### `native_wasm_equiv_sweep.sh` + `native_wasm_equiv_sweep.py`
Sweeps random seeds comparing native vs WASM gamelogs (instance-ids stripped,
hidden-info draws masked). Open bug **mtg-ofl2i**: native and WASM currently
DIVERGE; the validate leg asserts the known divergence via `--expect-divergence`
(a tripwire that fails the moment they agree, telling you to drop the flag).
```
bash bug_finding/native_wasm_equiv_sweep.sh --seeds 50 --decks 'decks/old_school/*.dck'
```
Bounded validate counterpart: the Makefile `validate-wasm-e2e-step` leg.

### `network_fuzz_test.py`
Randomized network-game fuzzer over the loopback harness (`network_test_lib`).
Random seeds/decks/controllers (native and WASM clients); `--local-equivalence`
also compares against a local run.
```
python3 bug_finding/network_fuzz_test.py --quick                 # 10 configs
python3 bug_finding/network_fuzz_test.py --configs 100 --parallel 4
python3 bug_finding/network_fuzz_test.py --infinite              # until Ctrl-C
python3 bug_finding/network_fuzz_test.py --configs 20 --mode network --local-equivalence
```

### `snapshot_stress_test_single.py`
Snapshot/resume stress for ONE deck: snapshot at many stop points, resume each,
verify the resumed run matches the uninterrupted run.
```
python3 bug_finding/snapshot_stress_test_single.py decks/grizzly_bears.dck heuristic heuristic --replays 3
```
Bounded validate counterpart: `tests/snapshot_resume_e2e.sh`.

### `test_snapshot_determinism.py`
Snapshot-determinism: snapshot twice from the same state -> identical snapshots,
swept across stop points. (Shorter fixed-seed checks also available.)
```
python3 bug_finding/test_snapshot_determinism.py
```

### `flakiness_stress.py`
Generic test-flakiness / nondeterminism diagnosis utility: runs ANY canonical
test N times and records pass/fail to the flakiness DB. Not a game fuzzer, but a
bug-finding/diagnosis tool -> it lives here. See
[`../ai_docs/reference/TEST_FLAKINESS.md`](../ai_docs/reference/TEST_FLAKINESS.md).
```
python3 bug_finding/flakiness_stress.py list
python3 bug_finding/flakiness_stress.py one validate.shell_script_tests.commander_e2e --runs 20 --record
python3 bug_finding/flakiness_stress.py stress-all --runs 3 --concurrency 4 --record
```

## Filing a bug from a finding

1. Capture the failing **seed** (+ deck pair / stop point / controller) — that
   is your deterministic reproducer.
2. `bd create` a beads issue: the invariant violated, the harness + exact
   command, and the reproducer seed. (Bug fixes need an MTG rules review before
   merge — `.claude/skills/mtg-rules-review/SKILL.md`.)
3. Add a fixed-seed regression leg (a `tests/*_e2e.sh` or proptest case) so
   `make validate` guards the fix once it lands.

## Relationship to regression tests

- **`tests/`** — deterministic regression tests run by `make validate` + CI.
- **`bug_finding/`** — randomized exploratory testing for discovering new bugs.

## See also

- [`../docs/FUZZ_AND_STRESS_TESTING_STRATEGY.md`](../docs/FUZZ_AND_STRESS_TESTING_STRATEGY.md) — full strategy + inventory.
- [`../docs/NETWORK_ARCHITECTURE.md`](../docs/NETWORK_ARCHITECTURE.md) — loopback model; desync rules.
- [`../ai_docs/reference/TEST_FLAKINESS.md`](../ai_docs/reference/TEST_FLAKINESS.md) — flakiness tracking.
- [`../ai_docs/reference/NETWORK_ACTION_LOG.md`](../ai_docs/reference/NETWORK_ACTION_LOG.md) — network action log.
