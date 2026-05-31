# Fuzz & Stress Testing Strategy

This document defines the ONE coherent structure for randomized testing in
mtg-forge-rs: where each harness lives, what invariant it guards, whether it is
a deterministic regression test (in `make validate`) or a bug-finding
expedition (in `bug_finding/`), and the shared-helper layer that keeps the
logic DRY.

## The policy (authoritative)

Fuzzing is a bug-finding **activity**, not a regression test.

- **Bug-finding / expeditions** — scripts that explore many random seeds /
  decks / stop points (for minutes to hours) to surface new bugs. They live
  under **`bug_finding/`** and are run periodically by humans / the
  coordinator. They are **NOT** part of `make validate` or CI. Each finding
  produces a beads issue plus a fixed-seed reproducer.
- **Regression testing in `make validate` is DETERMINISTIC.** A SMALL, SHORT
  randomized leg is allowed in validate **only if** it starts from pinned
  deterministic seed(s) (so it is reproducible by anyone from the same SHA).
  Anything that sweeps many random seeds / runs for many minutes belongs in
  `bug_finding/`, not validate.

One-line rule:

> validate = deterministic; a fixed-seed SHORT randomized leg is OK there;
> unbounded / many-seed random = `bug_finding/` expedition.

This mirrors the inviolable network rule (see
[`NETWORK_ARCHITECTURE.md`](NETWORK_ARCHITECTURE.md)): **desync is always
fatal**, and **controllers must be information-independent** (identical
decisions on full-state server vs shadow client). Most of the equivalence
harnesses below exist precisely to catch violations of that rule.

## Directory layout

```
bug_finding/                       # expeditions (NOT in validate / CI)
├── README.md                      # per-harness usage; lists every harness here
├── network_test_lib.py            # SHARED Python helpers (see below)
├── network_fuzz_test.py           # randomized network-game fuzzer
├── fuzz_determinism_netequiv.sh   # determinism + local-vs-network sweep
├── native_wasm_equiv_sweep.sh     # native-vs-WASM sweep (wrapper)
├── native_wasm_equiv_sweep.py     #   ...comparator
├── snapshot_stress_test_single.py # snapshot/resume stress
├── test_snapshot_determinism.py   # snapshot-determinism sweep (+ --quick)
├── flakiness_stress.py            # generic test-flakiness diagnosis utility
└── lib/
    ├── gamelog_filter.sh          # SHARED bash [GAMELOG ...] filter
    └── seed_salts.sh              # SHARED bash mirror of the Rust seed salts

tests/                             # validate legs (deterministic, short)
├── fuzz_determinism_netequiv_e2e.sh    # DETERMINISM only (fixed seeds 1..4, local) -> sweep driver
├── network_vs_local_equivalence_e2e.sh # fixed seed (3) local-vs-network
├── network_vs_local_equivalence.py     # Python equivalent (uses shared lib)
└── snapshot_resume_e2e.sh              # fixed seed 42 + fixed stop points
# native-vs-WASM validate leg is invoked inline in the Makefile
#   (validate-wasm-e2e-step) calling bug_finding/native_wasm_equiv_sweep.sh
#   STRICT twice: (1) --seeds 1 --decks 'decks/old_school2/*.dck' --max-turns 8
#   (broad old-school coverage), and (2) --seed-base 15 --max-turns 25 on
#   decks/old_school2/fireball_multitarget.dck (pins the MULTI-TARGET Fireball
#   DivideEvenly cast at Turn11 — 2 distinct targets — as a strict native==WASM
#   regression guard for mtg-tyvcn). Both assert 0 diverged.

mtg-engine/tests/proptest_invariants.rs  # fixed-seed proptest (validate)
```

## Harness inventory

| Harness | Invariant guarded | Kind | Lives in | How to run |
|---------|-------------------|------|----------|------------|
| `proptest_invariants.rs` | core game invariants under proptest | regression (validate) | `mtg-engine/tests/` | `cargo test` path; proptest pinned to a FIXED seed/cases budget |
| `fuzz_determinism_netequiv_e2e.sh` | native **determinism** (same-seed→identical gamelog, local-only) | regression (validate) | `tests/` | `bash tests/fuzz_determinism_netequiv_e2e.sh` (fixed seeds 1..4, bounded) |
| `native_wasm_equiv_sweep` (validate leg) | native==WASM (STRICT, byte-identical); incl. **multi-target Fireball DivideEvenly** guard (mtg-tyvcn) | regression (validate) | Makefile → `bug_finding/native_wasm_equiv_sweep.sh` | `--seeds 1 --decks 'decks/old_school2/*.dck' --max-turns 8` + `--seed-base 15 --decks 'decks/old_school2/fireball_multitarget.dck' --max-turns 25` |
| `network_vs_local_equivalence_e2e.sh` | local==network gamelog identity | regression (validate) | `tests/` | `bash tests/network_vs_local_equivalence_e2e.sh 3 random` |
| `snapshot_resume_e2e.sh` | snapshot resume == uninterrupted run | regression (validate) | `tests/` | `bash tests/snapshot_resume_e2e.sh` (seed 42, stops 3/8/25) |
| `fuzz_determinism_netequiv.sh` | native determinism + local==network | **expedition** | `bug_finding/` | `bash bug_finding/fuzz_determinism_netequiv.sh --seeds 40 --pair-mode all` |
| `native_wasm_equiv_sweep.sh`/`.py` | native==WASM | **expedition** | `bug_finding/` | `bash bug_finding/native_wasm_equiv_sweep.sh --seeds 50` |
| `network_fuzz_test.py` | local==network across random seeds/decks/controllers | **expedition** | `bug_finding/` | `python3 bug_finding/network_fuzz_test.py --configs 100` |
| `snapshot_stress_test_single.py` | snapshot resume across random stop points | **expedition** | `bug_finding/` | `python3 bug_finding/snapshot_stress_test_single.py <deck> heuristic heuristic` |
| `test_snapshot_determinism.py` | identical snapshots from identical state | **expedition** (has `--quick` short mode) | `bug_finding/` | `python3 bug_finding/test_snapshot_determinism.py` |
| `flakiness_stress.py` | exposes nondeterminism in an EXISTING test | diagnosis utility | `bug_finding/` | `python3 bug_finding/flakiness_stress.py one <name> --runs N --record` |

Every expedition has a deterministic validate counterpart guarding the same
invariant with pinned seed(s):

- **determinism** (same-seed→identical gamelog, local): `fuzz_determinism_netequiv.sh
  --invariant determinism` (expedition) ↔ `tests/fuzz_determinism_netequiv_e2e.sh`
  (validate; local-only, sub-second, reliably green).
- **local-vs-network equivalence**: `fuzz_determinism_netequiv.sh --invariant
  equivalence` heavy random×old-school-pair sweep (**expedition ONLY**) ↔
  `tests/network_vs_local_equivalence_e2e.sh 3 random` + `... 3 zero` (validate;
  a SINGLE pinned seed, deterministic, stable). The bounded random *equivalence
  sweep* is deliberately NOT in validate — see the split note below.
- native-vs-WASM: `bug_finding/native_wasm_equiv_sweep.sh` heavy sweep
  (expedition) ↔ the Makefile's bounded `--expect-divergence` leg (validate).
- snapshot/resume: `snapshot_stress_test_single.py` random stop points
  (expedition) ↔ `tests/snapshot_resume_e2e.sh` fixed stops (validate).

### Why the network EQUIVALENCE *sweep* is expedition-only (not in validate)

The network local-vs-network equivalence path has open **intermittent**
desyncs on the old-school "rogerbrand" deck family (mtg-586 for the native
validate-network-e2e flake; the mtg-589 WASM-shadow family). The bounded
equivalence sweep that briefly lived in `tests/fuzz_determinism_netequiv_e2e.sh`
(`--invariant equivalence --controllers random`, 1 pair × 2 seeds) PASSES in
isolation but FAILS under full concurrent `make validate` load — e.g.
`random, 01_rogue_rogerbrand vs 02_thedeck_peterschnidrig, seed=1` diverged
(~641-line gamelog diff) only under load, while passing standalone on clean
integration. A randomized validate leg that is green only when the machine is
quiet violates the policy above (validate's randomized legs must be
deterministically green). So:

- validate keeps the cheap, robust local-only **determinism** sweep
  (`fuzz_determinism_netequiv_e2e.sh`) plus the SINGLE-pinned-seed deterministic
  equivalence check (`network_vs_local_equivalence_e2e.sh`).
- the random×old-school-pair **equivalence sweep** stays a `bug_finding/`
  expedition until the mtg-586 / mtg-589 intermittent desyncs are root-caused
  (desync is always fatal; the fix must eliminate the race, not paper it over).

### Why `flakiness_stress.py` lives in `bug_finding/`

It is not a game fuzzer — it runs an EXISTING canonical test N times and records
pass/fail to the flakiness DB. But its purpose is identical to the rest of this
directory: surface nondeterminism / bugs by repeated randomized execution, then
file an issue. It belongs with the other bug-finding tools rather than in
general `scripts/` tooling. Its canonical name→command decoder (`KIND_RUNNERS` /
`decode()`) and the flakiness DB are documented in
[`../ai_docs/reference/TEST_FLAKINESS.md`](../ai_docs/reference/TEST_FLAKINESS.md).

## Shared-helper layer (DRY — one implementation each)

Before consolidation the `[GAMELOG ...]` extraction/filter pipeline was
copy-pasted into the fuzz sweep and into `network_vs_local_equivalence_e2e.sh`
(three bash copies total), and the per-player seed-derivation salts were
hand-copied as hex into two bash scripts. Now:

- **Bash gamelog filter** — `bug_finding/lib/gamelog_filter.sh::gamelog_filter`
  (and `gamelog_filter_file`). Strips ANSI, keeps `[GAMELOG ...]` lines, drops
  the known timing-noise lines (tap-for-mana, bare "resolves", per-event life
  deltas). Sourced by `bug_finding/fuzz_determinism_netequiv.sh` and
  `tests/network_vs_local_equivalence_e2e.sh`. Behaviourally identical to the
  Python `network_test_lib.py::extract_gamelog` (verified by a parity check).
- **Bash seed salts** — `bug_finding/lib/seed_salts.sh::derive_p1_seed` /
  `derive_p2_seed`. The ONE bash mirror of
  `mtg-engine/src/game/seed_derivation.rs` (`P1_SALT = 0x1234_5678_9ABC_DEF0`,
  `P2_SALT = 0xFEDC_BA98_7654_3210`). Sourced by both the fuzz sweep and
  `network_vs_local_equivalence_e2e.sh`. Local-vs-network equivalence depends
  on these matching the Rust source exactly; the Rust test
  `seed_derivation::tests::matches_canonical_native_salt_constants` pins the
  Rust side, and the equivalence validate legs catch any bash drift
  immediately.
- **Python network harness** — `bug_finding/network_test_lib.py` is the single
  Python implementation of `run_local_game`, `run_network_game` (loopback
  server + native/WASM clients), `run_equivalence_test`, `extract_gamelog`, and
  `compare_gamelogs`. Imported by `network_fuzz_test.py` and by
  `tests/network_vs_local_equivalence.py`.

Note: `native_wasm_equiv_sweep.py` keeps its OWN `_normalise_stream` (strips
card-instance-id suffixes and masks hidden-info draws) because the native and
WASM targets legitimately assign instance IDs from different offsets — that is a
DIFFERENT normalization from the local-vs-network filter and must not be
conflated. One filter per distinct comparison semantic; we do not invent a
third abstraction to force them together.

## Running an expedition + filing a bug

1. Pick an expedition from `bug_finding/` and run it with a wide sweep
   (`--seeds N` / `--configs N` / `--iterations N`). Keep concurrent process
   count low; clean up stuck processes with this worktree's
   `scripts/kill_zombie_processes.py` (never a global `pkill`).
2. On a finding, capture the failing **seed** (plus deck pair / stop point /
   controller) — that is your deterministic reproducer.
3. `bd create` a beads issue: the invariant violated, the harness + exact
   command, and the reproducer seed. (Bug-fix branches require an MTG rules
   review before merging — see `.claude/skills/mtg-rules-review/SKILL.md`.)
4. Add a fixed-seed regression leg (a `tests/*_e2e.sh` or a proptest case) so
   `make validate` guards the fix once it lands.

## See also

- [`../bug_finding/README.md`](../bug_finding/README.md) — per-harness usage.
- [`NETWORK_ARCHITECTURE.md`](NETWORK_ARCHITECTURE.md) — loopback server/client
  model; "desync is always fatal"; information-independence rule.
- [`../ai_docs/reference/TEST_FLAKINESS.md`](../ai_docs/reference/TEST_FLAKINESS.md)
  — flakiness DB + `flakiness_stress.py` usage.
- [`../ai_docs/reference/NETWORK_ACTION_LOG.md`](../ai_docs/reference/NETWORK_ACTION_LOG.md)
  — append-only ActionLog the equivalence harnesses exercise.
