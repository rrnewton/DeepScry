#!/usr/bin/env bash
# E2E test: Sedge Troll — conditional +1/+1 "as long as you control a Swamp"
#
# Regression test for mtg-398 (Card Compatibility: Sedge Troll).
#
# Sedge Troll is a 2/2 Troll with the static ability
#   S:Mode$ Continuous | Affected$ Card.Self | AddPower$ 1 | AddToughness$ 1
#     | IsPresent$ Swamp.YouCtrl
# i.e. "Sedge Troll gets +1/+1 as long as you control a Swamp." (CR 613.4c
# layer-7c continuous P/T modification, gated by an IsPresent condition).
#
# Before the fix the engine (a) silently dropped the IsPresent$ condition and
# (b) skipped Affected$ Card.Self in the P/T calculation, so Sedge Troll was
# always 2/2. This test pins the corrected behaviour: with a Swamp present the
# Troll is a 3/3 and deals 3 combat damage.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Sedge Troll: conditional +1/+1 (Swamp) E2E Test ==="
echo

cd "$WORKSPACE_ROOT"

PUZZLE="$WORKSPACE_ROOT/test_puzzles/sedge_troll_swamp_boost.pzl"
LOG=/tmp/sedge_troll_swamp_boost_e2e.txt

if run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE" \
    --p1=heuristic --p2=zero \
    --stop-on-choice=6 --json --seed 42 --verbosity 3 \
    > "$LOG" 2>&1; then
    echo -e "${GREEN}✓ Game completed${NC}"
else
    EXIT_STATUS=$?
    echo -e "${RED}✗ Game failed (exit $EXIT_STATUS)${NC}"
    head -80 "$LOG"
    exit 1
fi

# (a) With a Swamp present, Sedge Troll is buffed to 3/3 and attacks as a 3/3
if grep -qE "declares Sedge Troll \([0-9]+\) \(3/3\) as attacker" "$LOG"; then
    echo -e "${GREEN}✓ Sedge Troll is 3/3 with a Swamp present${NC}"
else
    echo -e "${RED}✗ Sedge Troll not boosted to 3/3${NC}"
    grep -iE "sedge troll" "$LOG" | head -5
    exit 1
fi

# (b) The boosted Troll deals 3 combat damage (not its base 2)
if grep -qE "Sedge Troll \([0-9]+\) deals 3 damage to Player 2" "$LOG"; then
    echo -e "${GREEN}✓ Sedge Troll dealt 3 combat damage${NC}"
else
    echo -e "${RED}✗ Sedge Troll did not deal 3 damage${NC}"
    grep -iE "damage" "$LOG" | head -5
    exit 1
fi

echo
echo -e "${GREEN}=== Test PASSED (Sedge Troll conditional Swamp boost) ===${NC}"
echo "Full log: $LOG"
exit 0
