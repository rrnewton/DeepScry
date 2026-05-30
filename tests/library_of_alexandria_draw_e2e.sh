#!/usr/bin/env bash
# E2E test: Library of Alexandria's "{T}: Draw a card. Activate only if you
# have exactly seven cards in hand." activation restriction.
#
# Regression test for the "Card Compatibility: Library of Alexandria" beads
# issue (mtg-517). The script is:
#   A:AB$ Draw | Cost$ T | PresentZone$ Hand | IsPresent$ Card.YouOwn
#              | PresentCompare$ EQ7
# which now parses into an ActivationCondition (CompareOp::Equal, count 7,
# zone Hand) and is enforced in can_activate.
#
# Two scenarios:
# (a) Seven cards in hand  -> the draw ability IS activatable and draws.
# (b) Six cards in hand    -> the draw ability is NOT offered (only mana/plays).
#
# This generalizes to every `IsPresent$ | PresentCompare$` activation gate
# (Magus of the Library, Cryptic Caves, Mistveil Plains, ...).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Library of Alexandria Draw-Gate E2E Test ==="
echo

cd "$WORKSPACE_ROOT"

PUZZLE7="$WORKSPACE_ROOT/test_puzzles/library_of_alexandria_seven.pzl"
PUZZLE6="$WORKSPACE_ROOT/test_puzzles/library_of_alexandria_six.pzl"
LOG7=/tmp/library_of_alexandria_seven_e2e.txt
LOG6=/tmp/library_of_alexandria_six_e2e.txt

# (a) Seven cards: the draw ability activates and a card is drawn.
run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE7" \
    --p1=fixed --p2=zero \
    --p1-fixed-inputs="activate Library of Alexandria;*" \
    --seed 42 --verbosity 3 \
    > "$LOG7" 2>&1 || true

if grep -qE "Library of Alexandria activates ability: Draw" "$LOG7" && grep -qE "Player 1 draws" "$LOG7"; then
    echo -e "${GREEN}✓ With 7 cards, Library of Alexandria drew a card${NC}"
else
    echo -e "${RED}✗ With 7 cards, draw ability did not fire${NC}"
    grep -iE "library|draw" "$LOG7" | head -8
    exit 1
fi

# (b) Six cards: the draw ability must be UNAVAILABLE. The fixed controller's
# "activate Library of Alexandria" command should fail to match any action,
# producing an InvalidAction error mentioning the available actions.
run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE6" \
    --p1=fixed --p2=zero \
    --p1-fixed-inputs="activate Library of Alexandria;*" \
    --seed 42 --verbosity 3 \
    > "$LOG6" 2>&1 || true

if grep -qE "Library of Alexandria activates ability: Draw" "$LOG6"; then
    echo -e "${RED}✗ With 6 cards, the draw ability was wrongly available${NC}"
    grep -iE "library|draw" "$LOG6" | head -8
    exit 1
else
    echo -e "${GREEN}✓ With 6 cards, the draw ability was correctly unavailable${NC}"
fi

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "Logs: $LOG7 ; $LOG6"
exit 0
