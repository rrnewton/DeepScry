# Fuzz & Stress Testing Strategy

This document is the **single canonical reference** for randomized / fuzz /
stress testing in mtg-forge-rs: the one driver, its modes, the Makefile
shortcuts, the deterministic regression legs in `make validate`, the shared
helper layer, and the bug-finding-vs-regression policy.

## The policy (authoritative)

Fuzzing is a bug-finding **activity**, not a regression test.

- **Bug-finding / expeditions** — sweep many random seeds / decks / stop points
  (minutes to hours) to surface NEW bugs. They run from the ONE driver
  `bug_finding/fuzz.py` (or its `make fuzz-*` shortcuts), are run periodically
  by humans / the coordinator, and are **NOT** part of `make validate` or CI.
  Each finding produces a beads issue plus a fixed-seed reproducer.
- **Regression testing in `make validate` is DETERMINISTIC.** A SMALL, SHORT
  randomized leg is allowed in validate **only if** it starts from pinned
  deterministic seed(s) (reproducible by anyone from the same SHA). Anything
  that sweeps many random seeds / runs for many minutes belongs in an
  expedition, not validate.

One-line rule:

> validate = deterministic; a fixed-seed SHORT randomized leg is OK there;
> unbounded / many-seed random = a `bug_finding/fuzz.py` expedition.

This mirrors the inviolable network rule (see
[`NETWORK_ARCHITECTURE.md`](NETWORK_ARCHITECTURE.md)): **desync is always
fatal**, and **controllers must be information-independent** (identical
decisions on full-state server vs shadow client). Most of the equivalence
checks below exist precisely to catch violations of that rule.

## The ONE driver: `bug_finding/fuzz.py`

All expeditions are MODES (subcommands) of a single CLI that reuses the shared
layer (`network_test_lib.py` + `lib/*.sh`) and forwards the specialised
harnesses to their existing modules — one implementation per distinct
comparison semantic (DRY).

```
python3 bug_finding/fuzz.py <mode> [options]
```

| Mode | Invariant guarded | Implementation |
|------|-------------------|----------------|
| `determinism` | native same-seed → identical local gamelog | inline (`run_determinism_test`) |
| `equivalence` | local == network gamelog identity (the desync hunt) | inline (`run_equivalence_test`) |
| `network` | network game runs without crash / error | inline (`run_network_test`) |
| `native-wasm` | native == WASM (STRICT, byte-identical) | forwards → `native_wasm_equiv_sweep.py` |
| `snapshot` | snapshot/resume == uninterrupted, over deck×matchup grid | forwards → `snapshot_stress_test_single.py` |
| `snapshot-determinism` | snapshot taken twice from same state is identical | forwards → `test_snapshot_determinism.py` |
| `flakiness` | exposes nondeterminism in an EXISTING test | forwards → `flakiness_stress.py` |
| `expedition` | the **mtg-813 prize**: wall-clock bug hunt over the old-school corpus × config matrix, all-debug-on, zero desyncs | inline (drives the `determinism`/`equivalence`/`network` runners) |

The inline game modes (`determinism`, `equivalence`, `network`, `expedition`)
share one config-matrix builder: **deck-pairs × seeds × controllers × client
modes**. Common flags:

- `--decks 'GLOB[,GLOB...]'` — deck corpus (default: the 1994 old-school set,
  `decks/old_school/*.dck,decks/old_school2/*.dck`).
- `--seeds N --seed-base K` — N seeds starting at K.
- `--controllers "heuristic random zero"` — each listed controller runs BOTH
  players.
- `--pair-mode chain|all|self` — how deck pairs are formed (chain = consecutive,
  all = every i<j, self = mirror).
- `--client native|wasm|mixed` — network client mode (`network`/`equivalence`).
- `--parallel N` — concurrency (keep LOW; default 3).
- `--configs N` / `--infinite` / `--duration S` — batch sizing / run-until.
- `--debug-dir DIR` — copy failing-run logs (use the gitignored `debug/`).

The forwarded modes (`native-wasm`, `snapshot-determinism`, `flakiness`) pass
their arguments straight through to the underlying harness — run
`python3 bug_finding/fuzz.py <mode> --help` to see the forwarded flags.

### Makefile shortcuts (common flag-combos)

```
make fuzz-determinism   ARGS='--seeds 40 --pair-mode all'
make fuzz-equivalence   ARGS='--configs 30 --client wasm'
make fuzz-network       ARGS='--infinite'
make fuzz-native-wasm   ARGS='--seeds 50'
make fuzz-snapshot      ARGS='--decks royal_assassin,monored'
make fuzz-expedition    ARGS='--duration 3600 --modes determinism,equivalence'
```

Each target builds the network binary then invokes the matching `fuzz.py` mode;
everything after `ARGS=` is forwarded.

### The 1-hour expedition (mtg-813 prize)

`fuzz.py expedition` is the prize driver (successor to the action_count-in-hash
prize, mtg-q97bw → mtg-813): take the old-school corpus, play it with RANDOM
seeds and ALL debug checks ON (network runs always pass `--network-debug`; the
maximally-strict state hash incl. `action_count` is engine-default
post-1070b585), rotate the requested check modes across deck pairs until a
wall-clock budget is exhausted, and aggregate findings with per-finding
reproducers (saved under `debug/expedition/`). The prize is met when an hour of
expedition surfaces ZERO new fatal divergences.

```
# Full hour, network desync hunt, low concurrency:
make fuzz-expedition ARGS='--duration 3600 --modes determinism,equivalence --client native --parallel 4'
```

Keep concurrency modest to avoid port collisions; clean up stuck processes with
this checkout's `scripts/kill_zombie_processes.py` (never a global `pkill`,
which would kill another agent's worktree games).

## Deterministic regression legs (in `make validate`)

These are a SEPARATE category: fixed-seed, short, reliably green, wired into
`make validate` (via `scripts/validate.py` and the `tests/*.sh` auto-discovery
in `mtg-engine/tests/shell_script_tests.rs`). They guard the same invariants as
the expeditions but at pinned seeds so they are deterministically reproducible.
They are intentionally NOT routed through the random `fuzz.py` driver — validate
must never depend on an unbounded sweep.

| Validate leg | Invariant | How invoked |
|--------------|-----------|-------------|
| `mtg-engine/tests/proptest_invariants.rs` | core game invariants under proptest (fixed seed/cases) | `cargo test` (nextest) |
| `mtg-engine/tests/mana_cache_debug_stress_test.rs` | incremental mana-source cache == from-scratch battlefield scan, over a 20-game old-school tourney (seed 42) | `cargo test` (nextest) |
| `tests/fuzz_determinism_netequiv_e2e.sh` | native determinism (same seed → identical local gamelog), fixed seeds 1..4 | auto-discovered + `validate.py` (`network.fuzz`) |
| `tests/network_vs_local_equivalence_e2e.sh` | local == network gamelog identity, single pinned seed | `validate.py` (`network.equiv-{random,zero,heuristic}`) |
| native-vs-WASM STRICT leg | native == WASM byte-identical (incl. multi-target Fireball DivideEvenly, Black Vise, Spirit Link guards) | `validate.py` calls `bug_finding/native_wasm_equiv_sweep.sh` STRICT |
| `tests/snapshot_resume_e2e.sh` | `mtg resume` snapshot == uninterrupted run (seed 42, stops 3/8/25) | auto-discovered + `validate.py` (`determ.snapshot-resume`) |
| `tests/snapshot_stress_e2e.sh` | the Python snapshot harnesses still agree with the engine (anti-bit-rot gate for `snapshot_stress_test_single.py` + `test_snapshot_determinism.py`), seed 42, `--replays 1` | auto-discovered (`tests/*.sh`) |

> The bash determinism/equivalence sweep `fuzz_determinism_netequiv.sh` and its
> validate counterpart `tests/fuzz_determinism_netequiv_e2e.sh` use the **bash**
> shared layer (`lib/gamelog_filter.sh` + `lib/seed_salts.sh`) because the
> validate legs are shell. `fuzz.py`'s `determinism`/`equivalence` modes use the
> **Python** shared layer (`network_test_lib.py`). The two filters are
> behaviourally identical (verified by a parity check); this deliberate
> bash/Python pairing — shell for the shell validate legs, Python for the Python
> driver — is the only place the gamelog filter is "duplicated", and it is kept
> in sync on purpose.

### Why the network EQUIVALENCE *sweep* is expedition-only (not in validate)

The network local-vs-network equivalence path has open **intermittent** desyncs
on the old-school "rogerbrand" deck family (mtg-586 native; mtg-589
WASM-shadow). A bounded equivalence sweep PASSES in isolation but FAILS under
full concurrent `make validate` load (e.g. `01_rogue_rogerbrand vs
02_thedeck_peterschnidrig, seed=1` diverged only under load). A randomized
validate leg that is green only when the machine is quiet violates the policy
(validate's randomized legs must be deterministically green). So validate keeps
only the cheap local-only **determinism** sweep plus the single-pinned-seed
deterministic equivalence check; the random×old-school-pair **equivalence
sweep** stays a `fuzz.py equivalence` / `fuzz.py expedition` expedition until
the mtg-586 / mtg-589 desyncs are root-caused (desync is always fatal; the fix
must eliminate the race, not paper it over).

### The opt-in desync regression CANARY (`make validate-desync-canary`)

`bug_finding/desync_canary.sh` is a third category between the cheap validate
legs and the open-ended expeditions: an **opt-in heavy regression canary**. It
is a thin DRY wrapper over `fuzz_determinism_netequiv.sh --invariant
equivalence` that sweeps the historically-dangerous mechanics BROADLY —
cycling/search/shuffle (avatar pair), burn/combat-damage (monored mirror),
counter/stack-interaction (counterspells mirror) — across all three controllers
and broad seed ranges. It is NOT part of `make validate` (too heavy, ~tens of
minutes); run it on demand:

```
systemd-run --user --scope -- make validate-desync-canary          # full
systemd-run --user --scope -- make validate-desync-canary ARGS=--quick
```

Honest green/red split: the **GREEN corpus** (avatar / monored / counterspells)
drives the exit code; the **KNOWN-RED tier** (rogerbrand mirror) is run,
captured, and reported every time as XFAIL but does NOT gate (pre-existing
tracked desyncs mtg-586 / mtg-589 / mtg-609 / mtg-768). If a known-red leg ever
PASSES, the canary says so (promote it + update the baseline).

### Why `flakiness_stress.py` is a `fuzz.py` mode

It is not a game fuzzer — it runs an EXISTING canonical test N times and records
pass/fail to the flakiness DB. But its purpose is identical to the rest: surface
nondeterminism / bugs by repeated randomized execution, then file an issue. Its
canonical name→command decoder and the flakiness DB are documented in
[`../ai_docs/reference/TEST_FLAKINESS.md`](../ai_docs/reference/TEST_FLAKINESS.md).

## Directory layout

```
bug_finding/                       # expeditions (NOT in validate / CI)
├── README.md                      # per-mode usage; points here
├── fuzz.py                        # ★ THE ONE DRIVER (all modes / subcommands)
├── network_test_lib.py            # SHARED Python helpers (runners + oracles)
├── fuzz_determinism_netequiv.sh   # bash determinism+equiv sweep (validate-leg/canary engine)
├── desync_canary.sh               # opt-in heavy desync canary (make validate-desync-canary)
├── native_wasm_equiv_sweep.sh     # native-vs-WASM sweep wrapper (toolchain gating; validate entry)
├── native_wasm_equiv_sweep.py     #   ...comparator (own _normalise_stream; see DRY note)
├── snapshot_stress_test_single.py # snapshot/resume stress (one deck)
├── test_snapshot_determinism.py   # snapshot-determinism sweep
├── flakiness_stress.py            # generic test-flakiness diagnosis utility
└── lib/
    ├── gamelog_filter.sh          # SHARED bash [GAMELOG ...] filter
    └── seed_salts.sh              # SHARED bash mirror of the Rust seed salts

tests/                             # deterministic validate legs (fixed-seed, short)
├── fuzz_determinism_netequiv_e2e.sh    # determinism only (fixed seeds 1..4, local)
├── network_vs_local_equivalence_e2e.sh # single pinned seed local-vs-network
├── network_vs_local_equivalence.py     # Python equivalent (uses network_test_lib)
├── snapshot_resume_e2e.sh              # mtg resume == uninterrupted (seed 42)
└── snapshot_stress_e2e.sh              # anti-bit-rot gate for the Python snapshot harnesses
# native-vs-WASM validate leg is invoked from scripts/validate.py calling
#   bug_finding/native_wasm_equiv_sweep.sh STRICT (old_school2 + Fireball/Black
#   Vise/Spirit Link pins). All assert "0 diverged".

mtg-engine/tests/proptest_invariants.rs           # fixed-seed proptest (validate)
mtg-engine/tests/mana_cache_debug_stress_test.rs  # mana-cache from-scratch consistency (validate)
```

## Shared-helper layer (DRY — one implementation each)

- **Python runners + oracles** — `bug_finding/network_test_lib.py` is the single
  Python implementation of `run_local_game`, `run_network_game` (loopback server
  + native/WASM clients), `run_determinism_test`, `run_equivalence_test`,
  `run_network_test`, `extract_gamelog` / `compare_gamelogs`, and the
  perspective-aware server↔client oracle. Imported by `fuzz.py` and by
  `tests/network_vs_local_equivalence.py`.
- **Bash gamelog filter** — `lib/gamelog_filter.sh` (`gamelog_filter` /
  `gamelog_filter_file`). Sourced by `fuzz_determinism_netequiv.sh` and
  `tests/network_vs_local_equivalence_e2e.sh`. Behaviourally identical to the
  Python `extract_gamelog` (verified by a parity check).
- **Bash seed salts** — `lib/seed_salts.sh` (`derive_p1_seed` / `derive_p2_seed`),
  the ONE bash mirror of `mtg-engine/src/game/seed_derivation.rs`. Sourced by the
  same two scripts. The Rust test
  `seed_derivation::tests::matches_canonical_native_salt_constants` pins the Rust
  side; the equivalence validate legs catch any bash drift immediately.

Note: `native_wasm_equiv_sweep.py` keeps its OWN `_normalise_stream` (strips
card-instance-id suffixes and masks hidden-info draws) because native and WASM
targets legitimately assign instance IDs from different offsets — a DIFFERENT
normalization from the local-vs-network filter, and it must not be conflated.
One filter per distinct comparison semantic; we do not invent a third
abstraction to force them together.

## Running an expedition + filing a bug

1. Run a mode with a wide sweep, e.g.
   `make fuzz-expedition ARGS='--duration 3600'` or
   `python3 bug_finding/fuzz.py equivalence --configs 100 --client wasm`. Keep
   concurrency low; clean up stuck processes with
   `scripts/kill_zombie_processes.py` (never a global `pkill`).
2. On a finding, capture the failing **seed** (+ deck pair / stop point /
   controller) — `fuzz.py` prints a deterministic reproducer per finding.
3. `bd create` a beads issue: the invariant violated, the exact reproducer
   command, and the seed. (Bug-fix branches require an MTG rules review before
   merging — see `.claude/skills/mtg-rules-review/SKILL.md`.)
4. Add a fixed-seed regression leg (a `tests/*_e2e.sh` or a proptest case) so
   `make validate` guards the fix once it lands.

## See also

- [`../bug_finding/README.md`](../bug_finding/README.md) — per-mode usage.
- [`NETWORK_ARCHITECTURE.md`](NETWORK_ARCHITECTURE.md) — loopback server/client
  model; "desync is always fatal"; information-independence rule.
- [`../ai_docs/reference/TEST_FLAKINESS.md`](../ai_docs/reference/TEST_FLAKINESS.md)
  — flakiness DB + `flakiness` mode usage.
- [`../ai_docs/reference/NETWORK_ACTION_LOG.md`](../ai_docs/reference/NETWORK_ACTION_LOG.md)
  — append-only ActionLog the equivalence harnesses exercise.
