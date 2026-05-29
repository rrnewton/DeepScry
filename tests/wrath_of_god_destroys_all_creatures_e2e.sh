#!/usr/bin/env bash
# E2E test: Wrath of God ({2}{W}{W} Sorcery) destroys all creatures.
#
# Regression test for "Card Compatibility: Wrath of God" (mtg-558).
# Script: A:SP$ DestroyAll | ValidCards$ Creature | NoRegen$ True.
#
# Scenario (test_puzzles/wrath_of_god_destroys_all_creatures.pzl):
# - P1 hand: Wrath of God. P1 board: Plains x4 (pays {2}{W}{W}).
# - P2 board: Grizzly Bears, Savannah Lions, Black Knight.
# - P1 casts Wrath of God; all three creatures are destroyed and
#   move to their owner's graveyard.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Wrath of God Destroys All Creatures E2E Test ==="
echo

cd "$WORKSPACE_ROOT"

PUZZLE="$WORKSPACE_ROOT/test_puzzles/wrath_of_god_destroys_all_creatures.pzl"
LOG=/tmp/wrath_of_god_destroys_all_creatures_e2e.txt

if run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE" \
    --p1=fixed --p2=zero \
    --p1-fixed-inputs="cast Wrath of God;*;*" \
    --p2-fixed-inputs="" \
    --stop-on-choice=6 --seed 42 --verbosity 3 \
    > "$LOG" 2>&1; then
    echo -e "${GREEN}✓ Game completed${NC}"
else
    EXIT_STATUS=$?
    echo -e "${RED}✗ Game failed (exit $EXIT_STATUS)${NC}"
    head -80 "$LOG"
    exit 1
fi

# (a) Wrath of God resolves
if grep -qE "Wrath of God \([0-9]+\) resolves" "$LOG"; then
    echo -e "${GREEN}✓ Wrath of God resolved${NC}"
else
    echo -e "${RED}✗ Wrath of God did not resolve${NC}"
    grep -iE "wrath" "$LOG" | head -8
    exit 1
fi

# (b) Each opposing creature destroyed and moves to graveyard
for cname in "Grizzly Bears" "Savannah Lions" "Black Knight"; do
    if grep -qE "${cname} \([0-9]+\) is destroyed" "$LOG" \
       && grep -qE "${cname} \([0-9]+\) goes to graveyard" "$LOG"; then
        echo -e "${GREEN}✓ ${cname} destroyed and sent to graveyard${NC}"
    else
        echo -e "${RED}✗ ${cname} not destroyed/graveyarded as expected${NC}"
        grep -iE "${cname}|destroy|graveyard" "$LOG" | head -10
        exit 1
    fi
done

# (c) NoRegen flag honored in the log emit
if grep -qE "Wrath of God \([0-9]+\) destroys all matching permanents \(can't be regenerated\)" "$LOG"; then
    echo -e "${GREEN}✓ NoRegen$ True honored in log${NC}"
else
    echo -e "${RED}✗ NoRegen log line missing${NC}"
    grep -iE "regen|wrath" "$LOG" | head -8
    exit 1
fi

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "Full log: $LOG"
exit 0
