#!/usr/bin/env bash
# E2E (validate-wired) bounded fuzz: native DETERMINISM across a SMALL
# seed x deck-pair sample.
#
# This is the fast, CI/`make validate`-wired guard for the determinism
# invariant the network rearchitecture (append-only ActionLog<T>, two-store
# ownership) must preserve:
#
#   determinism  -- same deck pair + seed run twice => byte-identical
#                   [GAMELOG ...] streams (heuristic AND random controllers).
#
# SCOPING NOTE — why there is NO local-vs-network EQUIVALENCE sweep here:
#   This leg used to ALSO run a bounded equivalence sweep
#   (`--invariant equivalence --controllers random`, 1 old-school pair x 2
#   seeds). That sweep was REMOVED from validate because:
#     (a) It is REDUNDANT with validate's existing deterministic fixed-seed
#         equivalence coverage: `tests/network_vs_local_equivalence_e2e.sh 3
#         random` + `... 3 zero` (single pinned seed, stable, fast).
#     (b) The network local-vs-network EQUIVALENCE path has open INTERMITTENT
#         desyncs on the old-school "rogerbrand" deck family (mtg-586, and the
#         WASM-shadow mtg-589 family). The bounded sweep PASSES in isolation but
#         FAILS under full concurrent `make validate` load — a load-sensitive
#         flake. A randomized validate leg that is only green when the machine
#         is quiet violates the project policy: validate's randomized legs must
#         be DETERMINISTICALLY green (pinned-seed + reliably reproducible).
#   The heavy random x old-school-pair EQUIVALENCE hunt now lives ONLY in the
#   bug_finding expedition: `bug_finding/fuzz_determinism_netequiv.sh
#   --invariant equivalence ...`. That is where intermittent-desync hunting
#   belongs until the mtg-586/mtg-589 family is root-caused. See
#   docs/FUZZ_AND_STRESS_TESTING_STRATEGY.md (determinism = validate regression
#   leg; equivalence sweep = expedition).
#
# The HEAVY standalone sweep lives in bug_finding/fuzz_determinism_netequiv.sh; this
# wrapper just calls it with a bounded corpus tuned to stay well under a few
# seconds so it does not bloat `make validate`. It HARD-FAILS (exit 1) on ANY
# divergence and NEVER soft-skips: a missing binary / cardsfolder is a hard
# error (exit 2), matching the project rule that validate tests must not
# silently skip.
#
# Budget rationale (see timings in the sweep script):
#   - determinism games are sub-second each (no network round-trips):
#       3 pairs x 4 seeds x 2 controllers = 24 game-pairs (48 games) ~ a few s.
#
# Usage: tests/fuzz_determinism_netequiv_e2e.sh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

# Reuse the prebuilt release+network binary if the caller already built it
# (CI / make validate set MTG_REUSE_PREBUILT=1); otherwise build once.
ensure_mtg_binary

echo "=== Fuzz determinism + local-vs-network equivalence (bounded) ==="
echo

cd "$WORKSPACE_ROOT"

# Default corpus = the 1994 old-school decks (decks/old_school*/). The sweep
# script defaults to exactly this glob; we pin a small slice here for speed.
OUT_DIR="$WORKSPACE_ROOT/debug/fuzz_validate_$$"

# --- determinism (cheap; wider net: 3 pairs x 4 seeds x 2 ctrls) ------------
# This is the ONLY invariant wired into validate from this harness. It is
# local-only (no network round-trips, no server/client procs), sub-second per
# game, and fully deterministic (same seed twice => byte-identical gamelog), so
# it is reliably green regardless of machine load. The local-vs-network
# EQUIVALENCE sweep that used to live here was removed (see the SCOPING NOTE in
# the header) — deterministic equivalence coverage stays in
# tests/network_vs_local_equivalence_e2e.sh, and the random equivalence sweep
# moved to the bug_finding expedition.
echo "--- Invariant: native determinism (heuristic + random) ---"
MTG_BIN="$MTG_BIN" bash "$WORKSPACE_ROOT/bug_finding/fuzz_determinism_netequiv.sh" \
    --invariant determinism \
    --pair-mode chain --max-pairs 3 \
    --start-seed 1 --seeds 4 \
    --controllers "heuristic random" \
    --timeout 60 \
    --out "$OUT_DIR/det"

# The sub-sweep exits non-zero on ANY divergence (set -e propagates), so
# reaching here means every bounded combo passed. Clean up the (passing)
# output dir; it lives under gitignored debug/ regardless.
rm -rf "$OUT_DIR"

echo
echo "=== ✓ bounded fuzz determinism PASSED ==="
exit 0
