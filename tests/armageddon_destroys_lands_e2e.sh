#!/usr/bin/env bash
# E2E test: Armageddon ({3}{W} Sorcery) destroys every land both players
# control; non-land permanents survive.
#
# Regression test for the "Card Compatibility: Armageddon" beads issue
# (mtg-481). Armageddon is `A:SP$ DestroyAll | ValidCards$ Land`.
#
# Scenario (test_puzzles/armageddon_destroys_lands.pzl):
# - P1 hand: Armageddon. P1 board: City of Brass x4 + Savannah Lions.
# - P2 board: City of Brass x2 + Serendib Efreet.
# - P1 casts Armageddon: all 6 City of Brass (lands) are destroyed; the two
#   creatures remain.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Armageddon Destroys All Lands E2E Test ==="
echo

cd "$WORKSPACE_ROOT"

PUZZLE="$WORKSPACE_ROOT/test_puzzles/armageddon_destroys_lands.pzl"
LOG=/tmp/armageddon_destroys_lands_e2e.txt

run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE" \
    --p1=fixed --p2=zero \
    --p1-fixed-inputs="cast Armageddon;*;*;*" \
    --seed 42 --verbosity 3 \
    > "$LOG" 2>&1 || true

# (a) Armageddon resolves and runs the DestroyAll over lands.
if grep -qE "Armageddon \([0-9]+\) destroys all matching permanents" "$LOG"; then
    echo -e "${GREEN}✓ Armageddon resolved its DestroyAll${NC}"
else
    echo -e "${RED}✗ Armageddon did not resolve DestroyAll${NC}"
    grep -iE "armageddon" "$LOG" | head -8
    exit 1
fi

# (b) All six lands (City of Brass) are destroyed.
DESTROYED=$(grep -cE "City of Brass \([0-9]+\) is destroyed" "$LOG" || true)
if [ "$DESTROYED" -ge 6 ]; then
    echo -e "${GREEN}✓ All 6 lands destroyed${NC}"
else
    echo -e "${RED}✗ Expected 6 lands destroyed, saw $DESTROYED${NC}"
    grep -iE "destroyed" "$LOG" | head -8
    exit 1
fi

# (c) Non-land permanents survive (no creature was destroyed by Armageddon).
if grep -qE "Savannah Lions \([0-9]+\) is destroyed|Serendib Efreet \([0-9]+\) is destroyed" "$LOG"; then
    echo -e "${RED}✗ A creature was wrongly destroyed by Armageddon${NC}"
    exit 1
else
    echo -e "${GREEN}✓ Creatures survived (only lands destroyed)${NC}"
fi

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "Full log: $LOG"
exit 0
