#!/usr/bin/env bash
# E2E test: Triskelion's "remove a +1/+1 counter: deal 1 damage" activation
#
# Regression test for mtg-396 (Card Compatibility: Triskelion).
# Before the SubCounter cost was implemented, Triskelion's activated ability
# was silently dropped during ability parsing because the parser only knew
# how to handle SubCounter<N/LOYALTY> (planeswalker minus abilities). Cards
# whose costs use other counter types (e.g. SubCounter<1/P1P1>) had
# `cost = None` and the parser skipped them.
#
# After the fix, `Cost::SubCounter { amount, counter_type }` is parsed,
# affordability is checked against the source card's counters, payment
# removes the counters, and the ability is exposed as an action.
#
# Test scenario:
# - P1 Triskelion (3 +1/+1 counters), Grizzly Bears in opp.battlefield
# - Activate Triskelion once
# - Verify (a) ability appears in available actions, (b) counter removed,
#   (c) 1 damage dealt to some target.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Triskelion: SubCounter Activation E2E Test ==="
echo

cd "$WORKSPACE_ROOT"

PUZZLE="$WORKSPACE_ROOT/test_puzzles/triskelion_pings.pzl"
LOG=/tmp/triskelion_subcounter_e2e.txt

if run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE" \
    --p1=fixed --p2=zero \
    --p1-fixed-inputs="activate Triskelion" \
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

# (a) Triskelion's activated ability is exposed as an action
if grep -qE "\[[0-9]+\] activate Triskelion" "$LOG"; then
    echo -e "${GREEN}✓ 'activate Triskelion' appears in available actions${NC}"
else
    echo -e "${RED}✗ Triskelion activation not exposed (SubCounter cost likely not parsed)${NC}"
    grep -E "activate" "$LOG" | head -5 || echo "(no activate lines)"
    exit 1
fi

# (b) Counter removal logged
if grep -qE "Triskelion loses 1 P1P1 counter\(s\) \(now 2\)" "$LOG"; then
    echo -e "${GREEN}✓ One P1P1 counter removed (Triskelion now has 2)${NC}"
else
    echo -e "${RED}✗ Counter removal not logged correctly${NC}"
    grep -E "counter" "$LOG" | head -5
    exit 1
fi

# (c) 1 damage dealt to some target
if grep -qE "takes 1 damage" "$LOG"; then
    echo -e "${GREEN}✓ 1 damage dealt to target${NC}"
    grep -E "takes 1 damage" "$LOG" | head -3
else
    echo -e "${RED}✗ No damage dealt${NC}"
    grep -iE "damage" "$LOG" | head -5
    exit 1
fi

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "Full log: $LOG"
exit 0
