#!/usr/bin/env bash
# E2E test: Earthquake Dragon's graveyard-return activated ability is offered and
# resolves correctly (mtg-d8zuh fix; mtg-502 WORKING assertion).
#
# Script on card:
#   A:AB$ ChangeZone | Cost$ 2 G Sac<1/Land> | Origin$ Graveyard
#        | Destination$ Hand | ActivationZone$ Graveyard
#
# Scenario (test_puzzles/earthquake_dragon_graveyard_return.pzl):
# - P1 graveyard: Earthquake Dragon. P1 battlefield: 4× Forest.
# - P1 (heuristic) should activate the graveyard ability during main phase,
#   sacrificing a Forest, returning Earthquake Dragon to hand.
# - Expected log: "Earthquake Dragon activates ability: Return Earthquake Dragon
#   from your graveyard to your hand." and subsequently it appears in P1 hand
#   (or is re-cast the next turn).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Earthquake Dragon Graveyard Return E2E Test ==="
echo

cd "$WORKSPACE_ROOT"

PUZZLE="$WORKSPACE_ROOT/test_puzzles/earthquake_dragon_graveyard_return.pzl"
LOG=/tmp/earthquake_dragon_graveyard_return_e2e.txt

if run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE" \
    --p1=heuristic --p2=heuristic \
    --seed 3 --verbosity 2 \
    > "$LOG" 2>&1; then
    echo -e "${GREEN}✓ Game completed${NC}"
else
    EXIT_STATUS=$?
    echo -e "${RED}✗ Game failed (exit $EXIT_STATUS)${NC}"
    head -80 "$LOG"
    exit 1
fi

# Assert: the graveyard-return ability was activated.
# The log line is: "Earthquake Dragon activates ability: Return Earthquake Dragon
# from your graveyard to your hand."
if grep -qiE "Earthquake Dragon activates ability" "$LOG"; then
    echo -e "${GREEN}✓ Earthquake Dragon graveyard-return ability activated${NC}"
else
    echo -e "${RED}✗ Earthquake Dragon graveyard-return ability was NOT activated${NC}"
    echo "--- relevant log lines ---"
    grep -iE "earthquake|graveyard|activat" "$LOG" | head -20
    exit 1
fi

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "Full log: $LOG"
exit 0
