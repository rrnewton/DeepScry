#!/usr/bin/env bash
# E2E test: Red Elemental Blast's modes honour their per-mode ValidTgts color
# restriction (Charm color qualifier).
#
# Regression test for the "Card Compatibility: Red Elemental Blast" beads issue
# (mtg-536) and the Charm-color bug (mtg-af24s). REBL is:
#   A:SP$ Charm | Choices$ DBCounter,DBDestroy
#   SVar:DBDestroy:DB$ Destroy | ValidTgts$ Permanent.Blue
#   SVar:DBCounter:DB$ Counter | TargetType$ Spell | ValidTgts$ Card.Blue
#
# Scenario (test_puzzles/red_elemental_blast_destroy_blue.pzl):
# - P1 hand: Red Elemental Blast. P1 board: a red Mountain.
# - P2 board: Phantom Monster (a 3/3 BLUE creature).
# - P1 casts REBL choosing "Destroy target blue permanent": the only legal
#   target is the BLUE Phantom Monster; P1's own red Mountain must NOT be a
#   legal target (CR 115.4 — illegal targets are not chosen).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Red Elemental Blast Color-Restriction E2E Test ==="
echo

cd "$WORKSPACE_ROOT"

PUZZLE="$WORKSPACE_ROOT/test_puzzles/red_elemental_blast_destroy_blue.pzl"
LOG=/tmp/red_elemental_blast_color_e2e.txt

run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE" \
    --p1=fixed --p2=zero \
    --p1-fixed-inputs="cast Red Elemental Blast;*;*;*" \
    --seed 42 --verbosity 3 \
    > "$LOG" 2>&1 || true

# (a) REBL destroys the blue Phantom Monster.
if grep -qE "destroys Phantom Monster" "$LOG"; then
    echo -e "${GREEN}✓ REBL destroyed the blue Phantom Monster${NC}"
else
    echo -e "${RED}✗ REBL did not destroy the blue creature${NC}"
    grep -iE "elemental|destroy|target" "$LOG" | head -8
    exit 1
fi

# (b) REBL must NOT have destroyed P1's own red Mountain.
if grep -qE "destroys Mountain" "$LOG"; then
    echo -e "${RED}✗ REBL illegally destroyed a red Mountain${NC}"
    grep -iE "destroy|mountain" "$LOG" | head -8
    exit 1
else
    echo -e "${GREEN}✓ REBL did not touch the red Mountain (color restriction honoured)${NC}"
fi

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "Full log: $LOG"
exit 0
