# Bug Finding Harness

Randomized / fuzz / stress testing for mtg-forge-rs. Unlike the deterministic
regression legs in `tests/` (wired into `make validate` + CI), these tools run
for a long time and are meant to be run **periodically** (or overnight) to
surface new bugs, then file a beads issue + a fixed-seed reproducer per finding.

**The full policy, mode list, Makefile shortcuts, validate-leg inventory, and
shared-helper map live in the canonical doc:**
[`../docs/FUZZ_AND_STRESS_TESTING_STRATEGY.md`](../docs/FUZZ_AND_STRESS_TESTING_STRATEGY.md).
This README is the quick per-mode reference.

## The rule (one line)

> `make validate` = deterministic; a fixed-seed SHORT randomized leg is OK
> there. Anything that sweeps many random seeds / runs for hours is a
> bug-finding **expedition** and runs from `fuzz.py`, NOT validate.

## One driver: `fuzz.py`

Every expedition is a MODE (subcommand) of the single driver
`bug_finding/fuzz.py`. It reuses the shared layer (`network_test_lib.py` +
`lib/*.sh`) and forwards the specialised harnesses to their own modules — one
implementation per distinct comparison semantic (DRY).

```
python3 bug_finding/fuzz.py <mode> [options]
python3 bug_finding/fuzz.py <mode> --help        # per-mode flags
```

| Mode | What it hunts |
|------|---------------|
| `determinism` | native same-seed → identical local gamelog |
| `equivalence` | local == network gamelog identity (the desync hunt) |
| `network` | network game crashes / errors |
| `native-wasm` | native == WASM strict byte-equivalence |
| `snapshot` | snapshot/resume == uninterrupted, over a deck×matchup grid |
| `snapshot-determinism` | snapshot taken twice from same state is identical |
| `flakiness` | nondeterminism in an EXISTING canonical test |
| `expedition` | the **mtg-813 prize**: wall-clock bug hunt over the old-school corpus × config matrix |

Common flags for the inline game modes (`determinism`/`equivalence`/`network`/
`expedition`): `--decks 'GLOB[,GLOB]'`, `--seeds N --seed-base K`,
`--controllers "heuristic random zero"`, `--pair-mode chain|all|self`,
`--client native|wasm|mixed`, `--parallel N`, `--configs N`/`--infinite`/
`--duration S`, `--debug-dir DIR`.

### Makefile shortcuts

```
make fuzz-determinism   ARGS='--seeds 40 --pair-mode all'
make fuzz-equivalence   ARGS='--configs 30 --client wasm'
make fuzz-network       ARGS='--infinite'
make fuzz-native-wasm   ARGS='--seeds 50'
make fuzz-snapshot      ARGS='--decks royal_assassin,monored'
make fuzz-expedition    ARGS='--duration 3600 --modes determinism,equivalence'
```

### Examples

```
# Native determinism over the 1994 old-school corpus:
python3 bug_finding/fuzz.py determinism --seeds 20 --decks 'decks/old_school2/*.dck'

# Local-vs-network desync hunt, native then WASM clients:
python3 bug_finding/fuzz.py equivalence --configs 50
python3 bug_finding/fuzz.py equivalence --configs 20 --client wasm

# Network-only crash fuzz, forever:
python3 bug_finding/fuzz.py network --infinite

# Native-vs-WASM strict sweep (forwarded flags):
python3 bug_finding/fuzz.py native-wasm --seeds 50 --decks 'decks/old_school/*.dck'

# Snapshot/resume stress over decks x matchups (subsumes the old run_stress_tests.sh):
python3 bug_finding/fuzz.py snapshot --decks royal_assassin,white_aggro_4ed,monored \
    --matchups heuristic:heuristic,random:heuristic:--switch-fixed

# Snapshot-determinism at multiple stop points (forwarded):
python3 bug_finding/fuzz.py snapshot-determinism decks/monored.dck --choice 5 10 15

# Flakiness of an existing test (forwarded):
python3 bug_finding/fuzz.py flakiness one validate.shell_script_tests.commander_e2e --runs 20 --record

# The 1-hour prize expedition:
make fuzz-expedition ARGS='--duration 3600 --modes determinism,equivalence --parallel 4'
```

## Shared helpers (single source of truth — do NOT reimplement)

- `network_test_lib.py` — the ONE Python implementation of `run_local_game`,
  `run_network_game` (loopback server + native/WASM clients),
  `run_determinism_test`, `run_equivalence_test`, `run_network_test`,
  `extract_gamelog` / `compare_gamelogs`, the perspective-aware server↔client
  oracle, and error classification. Imported by `fuzz.py` and by
  `tests/network_vs_local_equivalence.py`.
- `lib/gamelog_filter.sh` — the ONE bash `[GAMELOG ...]` filter. Sourced by
  `fuzz_determinism_netequiv.sh` and `tests/network_vs_local_equivalence_e2e.sh`.
  Behaviourally identical to `extract_gamelog`.
- `lib/seed_salts.sh` — the ONE bash mirror of the Rust per-player seed salts
  (`mtg-engine/src/game/seed_derivation.rs`). Sourced by the same two scripts.

## The bash sweep + canary (validate / on-demand)

`fuzz_determinism_netequiv.sh` is the **bash** determinism+equivalence sweep
that powers the deterministic validate leg
(`tests/fuzz_determinism_netequiv_e2e.sh`) and the opt-in heavy
`desync_canary.sh` (`make validate-desync-canary`). It uses the bash shared
layer because the validate legs are shell; `fuzz.py`'s `determinism`/
`equivalence` modes are the Python expedition path. See the canonical doc for
why the two coexist (kept in sync by a parity check).

## Filing a bug from a finding

1. Capture the failing **seed** (+ deck pair / stop point / controller) — that
   is your deterministic reproducer; `fuzz.py` prints one per finding.
2. `bd create` a beads issue: the invariant violated, the exact command, and the
   reproducer seed. (Bug fixes need an MTG rules review before merge —
   `.claude/skills/mtg-rules-review/SKILL.md`.)
3. Add a fixed-seed regression leg (a `tests/*_e2e.sh` or proptest case) so
   `make validate` guards the fix once it lands.

## See also

- [`../docs/FUZZ_AND_STRESS_TESTING_STRATEGY.md`](../docs/FUZZ_AND_STRESS_TESTING_STRATEGY.md) — full strategy + inventory.
- [`../docs/NETWORK_ARCHITECTURE.md`](../docs/NETWORK_ARCHITECTURE.md) — loopback model; desync rules.
- [`../ai_docs/reference/TEST_FLAKINESS.md`](../ai_docs/reference/TEST_FLAKINESS.md) — flakiness tracking.
- [`../ai_docs/reference/NETWORK_ACTION_LOG.md`](../ai_docs/reference/NETWORK_ACTION_LOG.md) — network action log.
