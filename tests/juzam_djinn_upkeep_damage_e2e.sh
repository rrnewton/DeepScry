#!/usr/bin/env bash
# E2E test: Juzám Djinn deals 1 damage to its controller at the beginning
# of each upkeep.
#
# Card: Juzám Djinn (cardsfolder/j/juzam_djinn.txt) — mtg-515
# Deck: 05 Mono Black Rogerbrand (mtg-560)
#
# Script:
#   ManaCost:2 B B
#   Types:Creature Djinn
#   PT:5/5
#   T:Mode$ Phase | Phase$ Upkeep | ValidPlayer$ You | TriggerZones$ Battlefield
#     | Execute$ TrigDealDamage
#   SVar:TrigDealDamage:DB$ DealDamage | Defined$ You | NumDmg$ 1
#   Oracle:At the beginning of your upkeep, Juzám Djinn deals 1 damage to you.
#
# This is a mandatory drawback: the trigger MUST fire every upkeep while
# Juzám Djinn is on the battlefield. Silent drop would make the card
# strictly stronger than printed.
#
# Test scenario: P0 has Juzám Djinn on the battlefield at life 20. After
# 2 turns the trigger must have fired 2 times and P0's life total must
# be 18 or less.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Juzám Djinn Upkeep Self-Damage E2E ==="
echo

cd "$WORKSPACE_ROOT"

PUZZLE="$WORKSPACE_ROOT/test_puzzles/juzam_djinn_upkeep_damage.pzl"
LOG=/tmp/juzam_djinn_upkeep_damage_e2e.txt

if run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE" \
    --p1=zero --p2=zero \
    --stop-on-choice=12 --seed 42 --verbosity 3 \
    > "$LOG" 2>&1; then
    echo -e "${GREEN}✓ Game ran${NC}"
else
    EXIT_STATUS=$?
    echo -e "${RED}✗ Game failed (exit $EXIT_STATUS)${NC}"
    head -80 "$LOG"
    exit 1
fi

# Required: the trigger fires and Juzám Djinn deals 1 damage to its controller.
TRIGGER_COUNT=$(grep -cE "Juzám Djinn deals 1 damage to Player 1" "$LOG" || true)
if [[ "$TRIGGER_COUNT" -ge 2 ]]; then
    echo -e "${GREEN}✓ Juzám Djinn upkeep trigger fired ${TRIGGER_COUNT} times${NC}"
else
    echo -e "${RED}✗ Juzám Djinn upkeep trigger did not fire enough times (got $TRIGGER_COUNT, want >=2)${NC}"
    grep -E "Juzam|Djinn|damage|upkeep|trigger" "$LOG" | head -20 || echo "(none)"
    exit 1
fi

# Required: the trigger message matches the card text.
if grep -qE "Trigger: Juzám Djinn - At the beginning of your upkeep, CARDNAME deals 1 damage to you" "$LOG"; then
    echo -e "${GREEN}✓ Trigger description correct${NC}"
else
    echo -e "${RED}✗ Trigger description missing or wrong${NC}"
    grep -E "Trigger:" "$LOG" | head -5 || echo "(none)"
    exit 1
fi

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "Full log: $LOG"
exit 0
