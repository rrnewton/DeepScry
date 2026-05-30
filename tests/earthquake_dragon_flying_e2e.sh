#!/usr/bin/env bash
# E2E test: Earthquake Dragon casts as a 10/10 and its Flying keyword is honored.
#
# Coverage for the "Card Compatibility: Earthquake Dragon" beads issue
# (mtg-502). Earthquake Dragon is a {14}{G} 10/10 Flying, Trample Elemental
# Dragon. A non-flying, non-reach blocker (Grizzly Bears) may NOT block it
# (CR 509.1b / 702.9b), so the Dragon's combat damage must hit the player.
#
# NOTE: the card is classified PARTIAL — its graveyard-return activated
# ability (ActivationZone$ Graveyard) is not yet offered (bug mtg-d8zuh).
# This test only asserts the WORKING aspects (10/10 body + Flying + combat).
#
# Scenario (test_puzzles/earthquake_dragon_flying.pzl):
# - P1 board: Earthquake Dragon + Forests. P2 board: Grizzly Bears.
# - P1 (heuristic) attacks; P2 cannot block; damage hits P2.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Earthquake Dragon Flying E2E Test ==="
echo

cd "$WORKSPACE_ROOT"

PUZZLE="$WORKSPACE_ROOT/test_puzzles/earthquake_dragon_flying.pzl"
LOG=/tmp/earthquake_dragon_flying_e2e.txt

if run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE" \
    --p1=heuristic --p2=heuristic \
    --seed 7 --verbosity 2 \
    > "$LOG" 2>&1; then
    echo -e "${GREEN}✓ Game completed${NC}"
else
    EXIT_STATUS=$?
    echo -e "${RED}✗ Game failed (exit $EXIT_STATUS)${NC}"
    head -80 "$LOG"
    exit 1
fi

# (a) Earthquake Dragon is a 10/10 and attacks.
if grep -qE "declares Earthquake Dragon \([0-9]+\) \(10/10\) as attacker" "$LOG"; then
    echo -e "${GREEN}✓ Earthquake Dragon (10/10) attacked${NC}"
else
    echo -e "${RED}✗ Earthquake Dragon never attacked as a 10/10${NC}"
    grep -iE "earthquake|attacker" "$LOG" | head -8
    exit 1
fi

# (b) Grizzly Bears is NEVER declared as a blocker for the flyer.
if grep -qE "declares Grizzly Bears .* as blocker for Earthquake Dragon" "$LOG"; then
    echo -e "${RED}✗ Grizzly Bears illegally blocked the flyer${NC}"
    grep -iE "blocker" "$LOG" | head -8
    exit 1
else
    echo -e "${GREEN}✓ Grizzly Bears did not (could not) block the flyer${NC}"
fi

# (c) Earthquake Dragon's combat damage hit the defending PLAYER.
if grep -qE "Earthquake Dragon \([0-9]+\) deals 10 damage to Player 2" "$LOG"; then
    echo -e "${GREEN}✓ Earthquake Dragon dealt 10 combat damage to Player 2${NC}"
else
    echo -e "${RED}✗ Earthquake Dragon did not damage the player for 10${NC}"
    grep -iE "earthquake.*deals" "$LOG" | head -8
    exit 1
fi

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "Full log: $LOG"
exit 0
