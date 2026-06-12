#!/usr/bin/env bash
# E2E test: Annihilator combat trigger (CR 702.86).
# When a creature with Annihilator N attacks, the defending player
# sacrifices N permanents before blockers are declared.
#
# Tests Emrakul, the Aeons Torn (Annihilator 6) attacking:
# - P0 attacks with Emrakul (Annihilator 6)
# - P1 has 7 permanents; must sacrifice 6 of them
# - Log should show "Annihilator 6" trigger and sacrifice events
#
# We use the heuristic controller which will attack with Emrakul
# since P1 is at low life, and zero controller for P1.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Annihilator Combat Trigger E2E Test ==="
echo

cd "$WORKSPACE_ROOT"

PUZZLE="$WORKSPACE_ROOT/test_puzzles/annihilator_trigger_sacrifices.pzl"
LOG=/tmp/annihilator_trigger_e2e.txt

run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE" \
    --p1=heuristic --p2=zero \
    --seed 42 --verbosity 3 \
    > "$LOG" 2>&1 || true

# (a) Annihilator 6 trigger fires when Emrakul attacks
# Expected log line: "Trigger: Emrakul, the Aeons Torn — Annihilator 6 (defending player sacrifices 6 permanents)"
if grep -qiE "Trigger.*Emrakul.*Annihilator 6|Annihilator 6.*defending player sacrifices" "$LOG"; then
    echo -e "${GREEN}✓ Annihilator 6 trigger fired (Emrakul's combat trigger logged)${NC}"
else
    echo -e "${RED}✗ Annihilator 6 trigger did not fire${NC}"
    grep -iE "emrakul|annihilator|sacrifice|trigger|attacker" "$LOG" | head -20
    exit 1
fi

# (b) Defending player sacrifices permanents due to Annihilator
SAC_COUNT=$(grep -cE "Player 2 sacrifices " "$LOG" || true)
if [ "$SAC_COUNT" -ge 6 ]; then
    echo -e "${GREEN}✓ Defending player sacrificed $SAC_COUNT permanents (Annihilator 6 = 6 required)${NC}"
else
    echo -e "${RED}✗ Expected at least 6 sacrifice events, saw $SAC_COUNT${NC}"
    grep -iE "sacrifice" "$LOG" | head -10
    exit 1
fi

# (b) Check that the game ran at all
if grep -qE "Game Over|Turn [0-9]" "$LOG"; then
    echo -e "${GREEN}✓ Game completed successfully${NC}"
else
    echo -e "${RED}✗ Game did not complete${NC}"
    cat "$LOG" | tail -20
    exit 1
fi

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "Full log: $LOG"
exit 0
