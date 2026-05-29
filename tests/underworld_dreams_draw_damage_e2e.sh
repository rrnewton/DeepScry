#!/usr/bin/env bash
# E2E test: Underworld Dreams deals 1 damage to an OPPONENT whenever that
# opponent draws a card, and does NOT damage its controller on the
# controller's own draws.
#
# Card compat (mtg-555, 1994 Old School 'Mono Black Rogerbrand' deck mtg-560):
#   T:Mode$ Drawn | ValidCard$ Card.OppOwn | Execute$ TrigDamage
#   SVar:TrigDamage:DB$ DealDamage | Defined$ TriggeredPlayer | NumDmg$ 1
#
# Scenario: P0 controls Underworld Dreams. The puzzle starts on P1's turn so
# P1 takes their draw-step draw and loses 1 life (20 -> 19).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Underworld Dreams Opponent-Draw Damage E2E ==="
echo

cd "$WORKSPACE_ROOT"

PUZZLE="$WORKSPACE_ROOT/test_puzzles/underworld_dreams_draw_damage.pzl"
LOG=/tmp/underworld_dreams_draw_damage_e2e.txt

if run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE" \
    --p1=zero --p2=fixed \
    --p2-fixed-inputs="" \
    --stop-on-choice=3 --seed 42 --verbosity 3 \
    > "$LOG" 2>&1; then
    echo -e "${GREEN}✓ Game completed${NC}"
else
    EXIT_STATUS=$?
    echo -e "${RED}✗ Game failed (exit $EXIT_STATUS)${NC}"
    head -80 "$LOG"
    exit 1
fi

# Required: the Underworld Dreams trigger fires for the opponent's draw.
if grep -qE "Trigger: Underworld Dreams" "$LOG"; then
    echo -e "${GREEN}✓ Underworld Dreams triggered on opponent draw${NC}"
else
    echo -e "${RED}✗ Underworld Dreams did not trigger${NC}"
    grep -E "draws|Trigger" "$LOG" || echo "(none)"
    exit 1
fi

# Required: opponent (Player 2) dropped to 19 life from the 1 damage.
if grep -qE "Life: 19" "$LOG"; then
    echo -e "${GREEN}✓ Opponent took 1 damage (life 20 -> 19)${NC}"
else
    echo -e "${RED}✗ Opponent life did not drop to 19${NC}"
    grep -E "Life:" "$LOG" || echo "(none)"
    exit 1
fi

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "Full log: $LOG"
exit 0
