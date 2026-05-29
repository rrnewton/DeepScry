#!/usr/bin/env bash
# E2E test: Terror ({1}{B} Instant) destroys a target nonblack creature.
#
# Regression test for the "Card Compatibility: Terror" beads issue (mtg-549).
# Terror is `A:SP$ Destroy | ValidTgts$ Creature.nonArtifact+nonBlack |
# NoRegen$ True`.
#
# Scenario (test_puzzles/terror_destroys_creature.pzl):
# - P1 hand: Terror. P1 board: Swamp x2 (pays {1}{B}).
# - P2 board: Grizzly Bears (a green, nonartifact, nonblack creature).
# - P1 casts Terror targeting Grizzly Bears; it is destroyed and moves to
#   its owner's graveyard.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Terror Destroys Creature E2E Test ==="
echo

cd "$WORKSPACE_ROOT"

PUZZLE="$WORKSPACE_ROOT/test_puzzles/terror_destroys_creature.pzl"
LOG=/tmp/terror_destroys_creature_e2e.txt

if run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE" \
    --p1=fixed --p2=zero \
    --p1-fixed-inputs="cast Terror;*;*" \
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

# (a) Terror targets the opponent's creature
if grep -qE "targeting Grizzly Bears" "$LOG"; then
    echo -e "${GREEN}✓ Terror targeted Grizzly Bears${NC}"
else
    echo -e "${RED}✗ Terror did not target the creature${NC}"
    grep -iE "terror|target" "$LOG" | head -8
    exit 1
fi

# (b) The creature is destroyed
if grep -qE "Terror \([0-9]+\) destroys Grizzly Bears" "$LOG"; then
    echo -e "${GREEN}✓ Grizzly Bears destroyed by Terror${NC}"
else
    echo -e "${RED}✗ Grizzly Bears not destroyed${NC}"
    grep -iE "destroy|grizzly" "$LOG" | head -8
    exit 1
fi

# (c) The destroyed creature moved to graveyard
if grep -qE "Grizzly Bears \([0-9]+\) goes to graveyard" "$LOG"; then
    echo -e "${GREEN}✓ Grizzly Bears moved to graveyard${NC}"
else
    echo -e "${RED}✗ Grizzly Bears did not move to graveyard${NC}"
    grep -iE "graveyard|grizzly" "$LOG" | head -8
    exit 1
fi

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "Full log: $LOG"
exit 0
