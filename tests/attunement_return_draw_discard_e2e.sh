#!/usr/bin/env bash
# E2E test: Attunement ("Return Attunement to its owner's hand: Draw three
# cards, then discard four cards").
#
# Regression test for the "Card Compatibility: Attunement" beads issue (2000
# World Championship Replenish deck, mtg-913 Return-cost followup) and the
# Return<N/Type> activation-cost concept. Script:
#   A:AB$ Draw | Cost$ Return<1/CARDNAME> | NumCards$ 3 | SubAbility$ DBDiscard
#   SVar:DBDiscard:DB$ Discard | NumCards$ 4 | Mode$ TgtChoose
#
# Before the fix, the cost parser had no `Return<...>` arm, so `Cost::parse`
# returned None and the loader skipped the whole activated ability — Attunement
# was a do-nothing enchantment. This test asserts the ability now activates,
# the source is returned to hand (the cost), and the draw-3 / discard-4 halves
# both run.
#
# Scenario (test_puzzles/attunement_return_draw_discard.pzl):
# - P1 board: Island x3 + Attunement. P1 hand: Plains x2, library: Plains x10.
# - P1 activates Attunement: returns it to hand (cost), draws 3, then discards 4.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Attunement Return-Draw-Discard E2E Test ==="
echo

cd "$WORKSPACE_ROOT"

PUZZLE="$WORKSPACE_ROOT/test_puzzles/attunement_return_draw_discard.pzl"
LOG=/tmp/attunement_return_draw_discard_e2e.txt

if run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE" \
    --p1=fixed --p2=zero \
    --p1-fixed-inputs="activate Attunement;*;*;*;*;*" \
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

# (a) Attunement's ability activates (was silently dropped before the fix)
if grep -qE "Attunement activates ability" "$LOG"; then
    echo -e "${GREEN}✓ Attunement ability activated${NC}"
else
    echo -e "${RED}✗ Attunement ability did not activate${NC}"
    grep -iE "attunement|activate" "$LOG" | head -8
    exit 1
fi

# (b) The Return<1/CARDNAME> cost moved Attunement back to its owner's hand
if grep -qE "Attunement.*is returned to hand" "$LOG"; then
    echo -e "${GREEN}✓ Attunement returned to hand (cost paid)${NC}"
else
    echo -e "${RED}✗ Attunement was not returned to hand${NC}"
    grep -iE "attunement|return|hand" "$LOG" | head -8
    exit 1
fi

# (c) Exactly three draws occurred ("Draw three cards")
DRAWS=$(grep -cE "Player 1 draws " "$LOG" || true)
if [ "$DRAWS" -ge 3 ]; then
    echo -e "${GREEN}✓ Attunement drew 3 cards (found $DRAWS draw lines)${NC}"
else
    echo -e "${RED}✗ Attunement did not draw 3 cards (found $DRAWS)${NC}"
    grep -iE "draw" "$LOG" | head -8
    exit 1
fi

# (d) Four discards occurred ("then discard four cards")
DISCARDS=$(grep -cE "Player 1 discards " "$LOG" || true)
if [ "$DISCARDS" -ge 4 ]; then
    echo -e "${GREEN}✓ Attunement discarded 4 cards (found $DISCARDS discard lines)${NC}"
else
    echo -e "${RED}✗ Attunement did not discard 4 cards (found $DISCARDS)${NC}"
    grep -iE "discard" "$LOG" | head -8
    exit 1
fi

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "Full log: $LOG"
exit 0
