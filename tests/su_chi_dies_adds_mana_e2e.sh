#!/usr/bin/env bash
# E2E test: Su-Chi ({4} 4/4 Artifact Creature) death trigger adds {C}{C}{C}{C}.
#
# Regression test for the "Card Compatibility: Su-Chi" beads issue (mtg-545),
# part of the Old-School 'The Deck' deck (mtg-413,
# decks/old_school/02_thedeck_peterschnidrig.dck).
#
# Su-Chi script (cardsfolder/s/su_chi.txt):
#   T:Mode$ ChangesZone | Origin$ Battlefield | Destination$ Graveyard
#     | ValidCard$ Card.Self | Execute$ TrigAddMana
#   SVar:TrigAddMana:DB$ Mana | Produced$ C | Amount$ 4
#
# Scenario (test_puzzles/su_chi_dies_adds_mana.pzl):
# - P1 board: Su-Chi + 4 Mountains. P1 hand: Lightning Bolt x4.
# - P1 bolts its own Su-Chi twice (3+3 = 6 >= 4 toughness) so Su-Chi dies.
# - The dies trigger fires and adds four colorless mana to P1's pool.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Su-Chi Dies Adds Mana E2E Test ==="
echo

cd "$WORKSPACE_ROOT"

PUZZLE="$WORKSPACE_ROOT/test_puzzles/su_chi_dies_adds_mana.pzl"
LOG=/tmp/su_chi_dies_adds_mana_e2e.txt

if run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE" \
    --p1=fixed --p2=zero \
    --p1-fixed-inputs="cast Lightning Bolt;*;cast Lightning Bolt;*;pass;pass;pass;pass" \
    --p2-fixed-inputs="" \
    --stop-when-fixed-exhausted --seed 42 --verbosity 3 \
    > "$LOG" 2>&1; then
    echo -e "${GREEN}✓ Game completed${NC}"
else
    EXIT_STATUS=$?
    echo -e "${RED}✗ Game failed (exit $EXIT_STATUS)${NC}"
    head -80 "$LOG"
    exit 1
fi

# (a) Su-Chi takes lethal damage from the two bolts.
if grep -qE "Lightning Bolt \([0-9]+\) deals 3 damage to Su-Chi" "$LOG"; then
    echo -e "${GREEN}✓ Su-Chi was bolted${NC}"
else
    echo -e "${RED}✗ Su-Chi was not damaged${NC}"
    grep -iE "su-chi|bolt|damage" "$LOG" | head -8
    exit 1
fi

# (b) The dies trigger fires.
if grep -qE "Trigger: Su-Chi - When CARDNAME dies, add \{C\}\{C\}\{C\}\{C\}" "$LOG"; then
    echo -e "${GREEN}✓ Su-Chi dies trigger fired${NC}"
else
    echo -e "${RED}✗ Su-Chi dies trigger did not fire${NC}"
    grep -iE "su-chi|trigger" "$LOG" | head -8
    exit 1
fi

# (c) The trigger adds mana to P1's pool.
if grep -qE "Su-Chi dies, Su-Chi adds mana to Player 1's pool" "$LOG"; then
    echo -e "${GREEN}✓ Su-Chi added mana on death${NC}"
else
    echo -e "${RED}✗ Su-Chi did not add mana on death${NC}"
    grep -iE "su-chi|mana|pool" "$LOG" | head -8
    exit 1
fi

# (d) Su-Chi moved to the graveyard.
if grep -qE "Su-Chi \([0-9]+\) goes to graveyard" "$LOG"; then
    echo -e "${GREEN}✓ Su-Chi moved to graveyard${NC}"
else
    echo -e "${RED}✗ Su-Chi did not move to graveyard${NC}"
    grep -iE "su-chi|graveyard" "$LOG" | head -8
    exit 1
fi

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "Full log: $LOG"
exit 0
