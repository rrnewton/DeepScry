#!/usr/bin/env bash
# E2E test: Jalum Tome ({2}, {T}: Draw a card, then discard a card).
#
# Regression test for the "Card Compatibility: Jalum Tome" beads issue
# (mtg-514). Jalum Tome is
#   A:AB$ Draw | Cost$ 2 T | NumCards$ 1 | SubAbility$ DBDiscard
#   SVar:DBDiscard:DB$ Discard | Defined$ You | NumCards$ 1 | Mode$ TgtChoose
#
# Scenario (test_puzzles/jalum_tome_draw_discard.pzl):
# - P1 board: Jalum Tome + Plains x2 (pays {2}). P1 hand: Mountain.
# - P1 activates Jalum Tome: draws a card from library, then discards the
#   Mountain. Hand stays the same size; one card enters from library and one
#   leaves to the graveyard.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Jalum Tome Draw Then Discard E2E Test ==="
echo

cd "$WORKSPACE_ROOT"

PUZZLE="$WORKSPACE_ROOT/test_puzzles/jalum_tome_draw_discard.pzl"
LOG=/tmp/jalum_tome_draw_discard_e2e.txt

if run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE" \
    --p1=fixed --p2=zero \
    --p1-fixed-inputs="activate Jalum Tome;*;*;*" \
    --p2-fixed-inputs="" \
    --stop-on-choice=8 --seed 42 --verbosity 3 \
    > "$LOG" 2>&1; then
    echo -e "${GREEN}✓ Game completed${NC}"
else
    EXIT_STATUS=$?
    echo -e "${RED}✗ Game failed (exit $EXIT_STATUS)${NC}"
    head -80 "$LOG"
    exit 1
fi

# (a) Jalum Tome's ability activates
if grep -qE "Jalum Tome activates ability: Draw a card, then discard a card" "$LOG"; then
    echo -e "${GREEN}✓ Jalum Tome ability activated${NC}"
else
    echo -e "${RED}✗ Jalum Tome ability did not activate${NC}"
    grep -iE "jalum|activate" "$LOG" | head -8
    exit 1
fi

# (b) A card is drawn
if grep -qE "Player 1 draws " "$LOG"; then
    echo -e "${GREEN}✓ Drew a card${NC}"
else
    echo -e "${RED}✗ No card drawn${NC}"
    grep -iE "draw" "$LOG" | head -8
    exit 1
fi

# (c) A card is discarded
if grep -qE "Player 1 discards " "$LOG"; then
    echo -e "${GREEN}✓ Discarded a card${NC}"
else
    echo -e "${RED}✗ No card discarded${NC}"
    grep -iE "discard" "$LOG" | head -8
    exit 1
fi

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "Full log: $LOG"
exit 0
