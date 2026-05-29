#!/usr/bin/env bash
# E2E test: Disenchant ({1}{W} Instant) destroys a target artifact.
#
# Regression test for the "Card Compatibility: Disenchant" beads issue
# (mtg-498). Disenchant is `A:SP$ Destroy | ValidTgts$ Artifact,Enchantment`.
#
# Scenario (test_puzzles/disenchant_destroys_artifact.pzl):
# - P1 hand: Disenchant. P1 board: Plains x2 (pays {1}{W}).
# - P2 board: Jalum Tome (an artifact).
# - P1 casts Disenchant targeting Jalum Tome; it is destroyed and moves to
#   its owner's graveyard.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Disenchant Destroys Artifact E2E Test ==="
echo

cd "$WORKSPACE_ROOT"

PUZZLE="$WORKSPACE_ROOT/test_puzzles/disenchant_destroys_artifact.pzl"
LOG=/tmp/disenchant_destroys_artifact_e2e.txt

if run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE" \
    --p1=fixed --p2=zero \
    --p1-fixed-inputs="cast Disenchant;*;*" \
    --p2-fixed-inputs="" \
    --stop-on-choice=6 --seed 42 --verbosity 3 \
    > "$LOG" 2>&1; then
    echo -e "${GREEN}✓ Game completed${NC}"
else
    EXIT_STATUS=$?
    echo -e "${RED}✗ Game failed (exit $EXIT_STATUS)${NC}"
    head -80 "$LOG"
    exit 1
fi

# (a) Disenchant targets the opponent's artifact
if grep -qE "targeting Jalum Tome" "$LOG"; then
    echo -e "${GREEN}✓ Disenchant targeted Jalum Tome (artifact)${NC}"
else
    echo -e "${RED}✗ Disenchant did not target the artifact${NC}"
    grep -iE "disenchant|target" "$LOG" | head -8
    exit 1
fi

# (b) The artifact is destroyed
if grep -qE "Disenchant \([0-9]+\) destroys Jalum Tome" "$LOG"; then
    echo -e "${GREEN}✓ Jalum Tome destroyed by Disenchant${NC}"
else
    echo -e "${RED}✗ Jalum Tome not destroyed${NC}"
    grep -iE "destroy|jalum" "$LOG" | head -8
    exit 1
fi

# (c) The destroyed artifact moved to graveyard
if grep -qE "Jalum Tome \([0-9]+\) goes to graveyard" "$LOG"; then
    echo -e "${GREEN}✓ Jalum Tome moved to graveyard${NC}"
else
    echo -e "${RED}✗ Jalum Tome did not move to graveyard${NC}"
    grep -iE "graveyard|jalum" "$LOG" | head -8
    exit 1
fi

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "Full log: $LOG"
exit 0
