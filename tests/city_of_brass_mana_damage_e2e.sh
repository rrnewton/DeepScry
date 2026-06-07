#!/usr/bin/env bash
# E2E test: City of Brass ({T}: Add one mana of any color; whenever it becomes
# tapped, it deals 1 damage to you).
#
# Regression test for the "Card Compatibility: City of Brass" beads issue
# (mtg-406). Script:
#   A:AB$ Mana | Cost$ T | Produced$ Any | Amount$ 1
#   T:Mode$ Taps | ValidCard$ Card.Self | Execute$ TrigDamage
#   SVar:TrigDamage:DB$ DealDamage | Defined$ You | NumDmg$ 1
#
# Scenario (test_puzzles/city_of_brass_mana_damage.pzl):
# - P1 board: City of Brass. P1 hand: Lightning Bolt.
# - P1 taps City of Brass for {R} to cast Lightning Bolt at P2.
# - Lightning Bolt deals 3 to P2 (proving the any-color mana paid for it),
#   and P1 drops to 19 life from the becomes-tapped trigger.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== City of Brass Mana + Tap Damage E2E Test ==="
echo

cd "$WORKSPACE_ROOT"

PUZZLE="$WORKSPACE_ROOT/test_puzzles/city_of_brass_mana_damage.pzl"
LOG=/tmp/city_of_brass_mana_damage_e2e.txt

if run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE" \
    --p1=fixed --p2=zero \
    --p1-fixed-inputs="cast Lightning Bolt;P2" \
    --p2-fixed-inputs="" \
    --seed 42 --verbosity 3 \
    > "$LOG" 2>&1; then
    echo -e "${GREEN}✓ Game completed${NC}"
else
    EXIT_STATUS=$?
    echo -e "${RED}✗ Game failed (exit $EXIT_STATUS)${NC}"
    head -80 "$LOG"
    exit 1
fi

# (a) City of Brass produced colored mana (any color -> {R} here)
if grep -qE "Tap City of Brass for \{R\}" "$LOG"; then
    echo -e "${GREEN}✓ City of Brass produced {R} (any color)${NC}"
else
    echo -e "${RED}✗ City of Brass did not produce colored mana${NC}"
    grep -iE "city|brass|mana|tap" "$LOG" | head -8
    exit 1
fi

# (b) The any-color mana paid for Lightning Bolt (3 damage to P2)
if grep -qE "Lightning Bolt \([0-9]+\) deals 3 damage to Player 2" "$LOG"; then
    echo -e "${GREEN}✓ Lightning Bolt resolved using City of Brass mana${NC}"
else
    echo -e "${RED}✗ Lightning Bolt did not resolve${NC}"
    grep -iE "lightning|bolt|damage" "$LOG" | head -8
    exit 1
fi

# (c) The becomes-tapped trigger dealt 1 damage to its controller (P1 -> 19)
if grep -qE "Life: 19" "$LOG"; then
    echo -e "${GREEN}✓ City of Brass tap trigger dealt 1 damage to controller${NC}"
else
    echo -e "${RED}✗ City of Brass tap trigger did not deal damage${NC}"
    grep -iE "life|damage" "$LOG" | head -8
    exit 1
fi

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "Full log: $LOG"
exit 0
