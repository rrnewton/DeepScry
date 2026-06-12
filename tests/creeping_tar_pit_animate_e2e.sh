#!/usr/bin/env bash
# E2E test: Creeping Tar Pit animate + unblockable ability.
# {1}{U}{B}: Until end of turn, this land becomes a 3/2 blue and black Elemental
# creature that can't be blocked this turn (card text). After animating, attacking
# with it should deal unblocked damage even when the opponent has creatures.
#
# This tests the SubAbility$ DBUnblockable chain from AB$ Animate, which grants
# the card a GrantCantBeBlocked persistent effect via the self-targeting path
# (Defined$ Self / RememberObjects$ Self).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Creeping Tar Pit Animate + Unblockable E2E Test ==="
echo

cd "$WORKSPACE_ROOT"

PUZZLE="$WORKSPACE_ROOT/test_puzzles/creeping_tar_pit_animate.pzl"
LOG=/tmp/creeping_tar_pit_animate_e2e.txt

# Use random controller with a known-good seed that animates and attacks
run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE" \
    --p1=random --p2=heuristic \
    --seed 2 --verbosity 3 \
    > "$LOG" 2>&1 || true

# (a) The animate ability fires
if grep -qE "Creeping Tar Pit activates ability" "$LOG"; then
    echo -e "${GREEN}✓ Creeping Tar Pit animate ability activated${NC}"
else
    echo -e "${RED}✗ Creeping Tar Pit animate ability did not activate${NC}"
    cat "$LOG" | tail -20
    exit 1
fi

# (b) Creeping Tar Pit becomes a 3/2 Elemental
if grep -qE "Creeping Tar Pit becomes Creature|Creeping Tar Pit base P/T set to 3/2" "$LOG"; then
    echo -e "${GREEN}✓ Creeping Tar Pit became a 3/2 creature${NC}"
else
    echo -e "${RED}✗ Creeping Tar Pit did not become a creature${NC}"
    cat "$LOG" | tail -20
    exit 1
fi

# (c) Unblockable effect is applied ("can't be blocked this turn")
if grep -qE "Creeping Tar Pit can't be blocked this turn" "$LOG"; then
    echo -e "${GREEN}✓ Creeping Tar Pit received 'can't be blocked' effect${NC}"
else
    echo -e "${RED}✗ 'Can't be blocked' effect not applied (SubAbility DBUnblockable chain broken)${NC}"
    grep -iE "creeping|unblock|cant|block" "$LOG" | head -10
    exit 1
fi

# (d) Creeping Tar Pit deals unblocked combat damage (P1 had Grizzly Bears, but they can't block)
if grep -qE "Creeping Tar Pit \([0-9]+\) deals 3 damage to Player 2" "$LOG"; then
    echo -e "${GREEN}✓ Creeping Tar Pit dealt 3 unblocked damage to opponent${NC}"
else
    echo -e "${RED}✗ Creeping Tar Pit did not deal unblocked damage (may have been blocked)${NC}"
    grep -iE "creeping.*damage|damage.*creeping|blocked" "$LOG" | head -10
    exit 1
fi

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "Full log: $LOG"
exit 0
