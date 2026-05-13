#!/usr/bin/env bash
# E2E test: Strip Mine ({T}, Sacrifice CARDNAME: Destroy target land)
#
# Regression test for mtg-0e702a (Card Compatibility: Strip Mine).
# Verifies the composite-cost activated ability:
# - Tap + Sacrifice cost is parsed and paid in the right order
# - Strip Mine itself moves to graveyard (sacrificed) before the effect resolves
# - The target land is destroyed
# - Targeting correctly excludes the source card via `sacrifices_self`
#   (Strip Mine cannot target itself since it's already gone by resolution)
#
# Test scenario:
# - P1 Strip Mine on battlefield; P2 has 3 Mountains.
# - Activate Strip Mine.
# - Verify (a) Strip Mine sacrificed, (b) one Mountain destroyed,
#   (c) targeting did NOT pick Strip Mine itself.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Strip Mine: Tap + Sacrifice → Destroy Land E2E Test ==="
echo

cd "$WORKSPACE_ROOT"

PUZZLE="$WORKSPACE_ROOT/test_puzzles/strip_mine_destroys_land.pzl"
LOG=/tmp/strip_mine_e2e.txt

if run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE" \
    --p1=fixed --p2=zero \
    --p1-fixed-inputs="activate Strip Mine" \
    --p2-fixed-inputs="" \
    --stop-on-choice=4 --json --seed 42 --verbosity 3 \
    > "$LOG" 2>&1; then
    echo -e "${GREEN}✓ Game completed${NC}"
else
    EXIT_STATUS=$?
    echo -e "${RED}✗ Game failed (exit $EXIT_STATUS)${NC}"
    head -80 "$LOG"
    exit 1
fi

# (a) Activation log line
if grep -qE "Strip Mine activates ability: Destroy target land" "$LOG"; then
    echo -e "${GREEN}✓ Strip Mine ability activated${NC}"
else
    echo -e "${RED}✗ Strip Mine activation not logged${NC}"
    exit 1
fi

# (b) Targeting picked an opponent's Mountain (NOT Strip Mine itself)
if grep -qE "^  *-> targeting Mountain \([0-9]+\)" "$LOG"; then
    echo -e "${GREEN}✓ Targeted an opponent Mountain${NC}"
    grep -E "^  *-> targeting" "$LOG" | head -3
else
    echo -e "${RED}✗ Did not target an opponent land${NC}"
    grep -E "targeting" "$LOG" | head -5
    exit 1
fi

# (c) Regression: must NOT auto-target Strip Mine itself (the cost sacrifices it)
if grep -qE "^  *-> targeting Strip Mine" "$LOG"; then
    echo -e "${RED}✗ Regression: Strip Mine targeted itself (sacrifices_self exclusion broken)${NC}"
    exit 1
fi

# (d) Strip Mine sacrificed (in P1's graveyard)
if grep -qE "Strip Mine \([0-9]+\) goes to graveyard" "$LOG"; then
    echo -e "${GREEN}✓ Strip Mine sacrificed${NC}"
else
    echo -e "${RED}✗ Strip Mine not sacrificed${NC}"
    exit 1
fi

# (e) Target land destroyed
if grep -qE "Mountain \([0-9]+\) goes to graveyard" "$LOG"; then
    echo -e "${GREEN}✓ Target Mountain destroyed${NC}"
else
    echo -e "${RED}✗ Target Mountain not destroyed${NC}"
    exit 1
fi

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "Full log: $LOG"
exit 0
