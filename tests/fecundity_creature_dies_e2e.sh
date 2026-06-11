#!/usr/bin/env bash
# E2E test: Fecundity — broad "whenever a creature dies" trigger
#
# Regression test for mtg-913 B12 (the 2000 World Championship Chimera deck's
# card-draw engine) and the mtg-409 follow-up it predicted.
#
# Fecundity reads: "Whenever a creature dies, that creature's controller may
# draw a card." Its dies-trigger uses the BROAD `ValidCard$ Creature` shape.
# Before this fix the loader's dies-trigger parser only recognized three narrow
# shapes — `Card.Self` (Academy Rector), `Card.EquippedBy` (Skullclamp) and
# `Creature.DamagedBy` (Sengir Vampire) — and SILENTLY DROPPED the broad
# `ValidCard$ Creature` form, so Fecundity's trigger never fired.
#
# Scenario:
# - P1 controls Fecundity + a Llanowar Elves.
# - P2 (fixed inputs) casts Blaze targeting the Llanowar Elves and kills it.
# - When the Elves dies, Fecundity's "a creature dies" trigger fires and P1 —
#   the DYING creature's controller, NOT necessarily Fecundity's controller —
#   draws a card. (Here Fecundity's controller and the dead creature's
#   controller are the same player; the cross-player case is exercised in the
#   unit test below.)
#
# Asserts the game log shows:
#   (a) "Trigger: Fecundity - Whenever a creature dies, ..."
#   (b) "Player 1 draws ..."

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Fecundity: broad 'whenever a creature dies' trigger E2E Test ==="
echo

cd "$WORKSPACE_ROOT"

PUZZLE="$WORKSPACE_ROOT/test_puzzles/fecundity_creature_dies.pzl"
LOG=/tmp/fecundity_creature_dies_e2e.txt

# P2 casts Blaze (X=1 -> 1 damage is not enough for a 1/1; we pay X=1 which is
# lethal to the 0/1... Llanowar Elves is 1/1, so X must be >=1). The Blaze
# script announces X; '1' selects X=1 and the second token targets the Elves.
if run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE" \
    --p1=heuristic --p2=fixed \
    --p2-fixed-inputs="cast Blaze;1;Llanowar Elves;*;*;*" \
    --seed 7 --verbosity 3 \
    > "$LOG" 2>&1; then
    echo -e "${GREEN}✓ Game completed${NC}"
else
    EXIT_STATUS=$?
    echo -e "${RED}✗ Game failed (exit $EXIT_STATUS)${NC}"
    head -80 "$LOG"
    exit 1
fi

# (a) The Llanowar Elves dies (the event that should fire Fecundity)
if grep -qE "Llanowar Elves \([0-9]+\) dies" "$LOG"; then
    echo -e "${GREEN}✓ Llanowar Elves died${NC}"
else
    echo -e "${RED}✗ Llanowar Elves did not die — test setup failed${NC}"
    grep -iE "blaze|llanowar" "$LOG" | head -10
    exit 1
fi

# (b) Fecundity's broad "a creature dies" trigger fires
if grep -qE "Trigger: Fecundity - Whenever a creature dies" "$LOG"; then
    echo -e "${GREEN}✓ Fecundity dies-trigger fired${NC}"
else
    echo -e "${RED}✗ Fecundity dies-trigger did NOT fire (mtg-913 B12 regression)${NC}"
    grep -iE "fecundity|trigger|dies" "$LOG" | head -10
    exit 1
fi

# (c) Player 1 (the dead creature's controller) draws a card
if grep -qE "Player 1 draws " "$LOG"; then
    echo -e "${GREEN}✓ Dead creature's controller (Player 1) drew a card${NC}"
else
    echo -e "${RED}✗ No draw by the dead creature's controller${NC}"
    grep -iE "draw" "$LOG" | head -10
    exit 1
fi

echo
echo -e "${GREEN}=== Test PASSED (Fecundity broad creature-dies trigger) ===${NC}"
echo "Full log: $LOG"
