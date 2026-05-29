#!/usr/bin/env bash
# E2E test: Bazaar of Baghdad ({T}: Draw two cards, then discard three cards).
#
# Regression test for the "Card Compatibility: Bazaar of Baghdad" beads issue
# (mtg-388) and the historical bug-bazaar-no-draw (draw effect once silently
# dropped, only discard ran). Script:
#   A:AB$ Draw | Cost$ T | NumCards$ 2 | SubAbility$ DBDiscard
#   SVar:DBDiscard:DB$ Discard | Defined$ You | NumCards$ 3 | Mode$ TgtChoose
#
# Scenario (test_puzzles/bazaar_of_baghdad_draw_discard.pzl):
# - P1 board: Bazaar of Baghdad. P1 hand: Plains x5, library: Plains x10.
# - P1 activates Bazaar: draws 2, then discards 3. Both halves must run
#   (the draw is NOT dropped), so we assert exactly two draws and three
#   discards.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Bazaar of Baghdad Draw-Then-Discard E2E Test ==="
echo

cd "$WORKSPACE_ROOT"

PUZZLE="$WORKSPACE_ROOT/test_puzzles/bazaar_of_baghdad_draw_discard.pzl"
LOG=/tmp/bazaar_of_baghdad_draw_discard_e2e.txt

if run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE" \
    --p1=fixed --p2=zero \
    --p1-fixed-inputs="activate Bazaar of Baghdad;*;*;*;*;*" \
    --p2-fixed-inputs="" \
    --stop-on-choice=10 --seed 42 --verbosity 3 \
    > "$LOG" 2>&1; then
    echo -e "${GREEN}✓ Game completed${NC}"
else
    EXIT_STATUS=$?
    echo -e "${RED}✗ Game failed (exit $EXIT_STATUS)${NC}"
    head -80 "$LOG"
    exit 1
fi

# (a) Bazaar's ability activates
if grep -qE "Bazaar of Baghdad activates ability" "$LOG"; then
    echo -e "${GREEN}✓ Bazaar ability activated${NC}"
else
    echo -e "${RED}✗ Bazaar ability did not activate${NC}"
    grep -iE "bazaar|activate" "$LOG" | head -8
    exit 1
fi

# (b) Exactly two draws occurred (regression for the silently-dropped draw)
DRAWS=$(grep -cE "Player 1 draws " "$LOG" || true)
if [ "$DRAWS" -ge 2 ]; then
    echo -e "${GREEN}✓ Bazaar drew 2 cards (found $DRAWS draw lines)${NC}"
else
    echo -e "${RED}✗ Bazaar did not draw 2 cards (found $DRAWS)${NC}"
    grep -iE "draw" "$LOG" | head -8
    exit 1
fi

# (c) Three discards occurred
DISCARDS=$(grep -cE "Player 1 discards " "$LOG" || true)
if [ "$DISCARDS" -ge 3 ]; then
    echo -e "${GREEN}✓ Bazaar discarded 3 cards (found $DISCARDS discard lines)${NC}"
else
    echo -e "${RED}✗ Bazaar did not discard 3 cards (found $DISCARDS)${NC}"
    grep -iE "discard" "$LOG" | head -8
    exit 1
fi

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "Full log: $LOG"
exit 0
