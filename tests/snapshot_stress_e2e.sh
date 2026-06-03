#!/usr/bin/env bash
# E2E GATE for snapshot/resume DETERMINISM stress (mtg-89).
#
# This wires the bug_finding/ snapshot stress harnesses into make validate + CI
# (auto-discovered by mtg-engine/tests/shell_script_tests.rs, which globs
# tests/*.sh). Those harnesses had silently bit-rotted against the engine's CLI
# (--stop-every -> --stop-on-choice) and tool layout, reporting spurious passes
# /failures with nothing gating them — which is exactly what let mtg-89 reopen.
# Gating them here means any future CLI/path drift HARD-FAILS CI instead of
# rotting unnoticed.
#
# What it checks, for the three close-criteria decks (royal_assassin,
# white_aggro_4ed, monored — note "moonred" in the issue text is a typo for
# the existing mono-red deck) in BOTH random/random and heuristic/heuristic:
#   1. snapshot_stress_test_single.py: a stop-and-go game (snapshot at a choice,
#      `mtg resume`, repeat to end) produces a game-action log IDENTICAL to one
#      uninterrupted run with the same seed.
#   2. test_snapshot_determinism.py (one bounded case): a snapshot taken at
#      choice N equals a snapshot taken right after resuming from it (modulo the
#      engine's own EXCLUDED_FIELDS metadata) — i.e. resume is byte-stable.
#
# Deterministic + bounded: fixed seed 42, --replays 1, default short stop
# points; each game is a couple of seconds. Any divergence, missing deck, or
# missing tool is a HARD FAIL (exit 1) — never a silent skip.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== snapshot/resume determinism stress e2e (mtg-89) ==="
echo

# Build (or reuse, under MTG_REUSE_PREBUILT=1) the RELEASE binary and export
# MTG_BIN. The python harnesses both honor MTG_BIN (snapshot_stress_test_single
# is release-only by design; test_snapshot_determinism falls back to debug for
# local dev but uses MTG_BIN when exported).
ensure_mtg_binary

cd "$WORKSPACE_ROOT"

command -v python3 >/dev/null 2>&1 || { echo -e "${RED}Error: python3 is required${NC}"; exit 1; }

STRESS="$WORKSPACE_ROOT/bug_finding/snapshot_stress_test_single.py"
DETERMINISM="$WORKSPACE_ROOT/bug_finding/test_snapshot_determinism.py"
for tool in "$STRESS" "$DETERMINISM"; do
    [ -f "$tool" ] || { echo -e "${RED}Error: missing stress harness $tool${NC}"; exit 1; }
done

SEED=42
DECKS=(royal_assassin white_aggro_4ed monored)
MODES=(random heuristic)

FAIL=0

# --- Phase 1: stop-and-go log-equivalence, 3 decks x 2 modes ----------------
for deck in "${DECKS[@]}"; do
    DECK_PATH="$WORKSPACE_ROOT/decks/$deck.dck"
    [ -f "$DECK_PATH" ] || { echo -e "${RED}Error: required deck not found: $DECK_PATH${NC}"; exit 1; }
    for mode in "${MODES[@]}"; do
        echo "--- stress: $deck ($mode vs $mode) ---"
        if MTG_BIN="$MTG_BIN" python3 "$STRESS" "$DECK_PATH" "$mode" "$mode" \
                --seed "$SEED" --replays 1 --json --quiet; then
            echo -e "${GREEN}  ✓ $deck $mode${NC}"
        else
            echo -e "${RED}  ✗ $deck $mode (stop-and-go log diverged from baseline)${NC}"
            FAIL=1
        fi
    done
done

# --- Phase 2: per-snapshot determinism (one bounded case) -------------------
echo "--- determinism: monored random (snapshot@N == resume-then-snapshot@0) ---"
if MTG_BIN="$MTG_BIN" python3 "$DETERMINISM" "$WORKSPACE_ROOT/decks/monored.dck" \
        --p1 random --p2 random --seed "$SEED" --choice 5 \
        --temp-dir "$(mktemp -d -t mtg_snap_det_XXXXXX)"; then
    echo -e "${GREEN}  ✓ monored random determinism${NC}"
else
    echo -e "${RED}  ✗ monored random snapshot non-deterministic${NC}"
    FAIL=1
fi

echo
if [ "$FAIL" -ne 0 ]; then
    echo -e "${RED}✗ snapshot stress e2e FAILED${NC}"
    exit 1
fi
echo -e "${GREEN}✓ snapshot stress e2e passed (6 stop-and-go + 1 determinism case)${NC}"
