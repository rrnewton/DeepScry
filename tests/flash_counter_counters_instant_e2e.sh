#!/usr/bin/env bash
# E2E test: Flash Counter ({1}{U} Instant) counters a target instant spell.
#
# Regression test for "Card Compatibility: Flash Counter" (mtg-506).
# Script: A:SP$ Counter | TargetType$ Spell | ValidTgts$ Instant.
#
# Scenario (test_puzzles/flash_counter_counters_instant.pzl):
# - P1 hand: Lightning Bolt; P1 board: Mountain x2.
# - P2 hand: Flash Counter; P2 board: Island x2.
# - P1 casts Lightning Bolt; P2 responds with Flash Counter, targeting
#   the Bolt. Bolt is countered and moves to the graveyard.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Flash Counter Counters Instant E2E Test ==="
echo

cd "$WORKSPACE_ROOT"

PUZZLE="$WORKSPACE_ROOT/test_puzzles/flash_counter_counters_instant.pzl"
LOG=/tmp/flash_counter_counters_instant_e2e.txt

if run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE" \
    --p1=fixed --p2=fixed \
    --p1-fixed-inputs="cast Lightning Bolt;*;*" \
    --p2-fixed-inputs="cast Flash Counter;*;*" \
    --stop-on-choice=10 --seed 42 --verbosity 3 \
    > "$LOG" 2>&1; then
    echo -e "${GREEN}✓ Game completed${NC}"
else
    EXIT_STATUS=$?
    echo -e "${RED}✗ Game failed (exit $EXIT_STATUS)${NC}"
    head -80 "$LOG"
    exit 1
fi

# (a) Both spells cast
if grep -qE "Player 1 casts Lightning Bolt" "$LOG"; then
    echo -e "${GREEN}✓ Lightning Bolt cast${NC}"
else
    echo -e "${RED}✗ Lightning Bolt was not cast${NC}"
    grep -iE "lightning|bolt" "$LOG" | head -8
    exit 1
fi

if grep -qE "Player 2 casts Flash Counter" "$LOG"; then
    echo -e "${GREEN}✓ Flash Counter cast${NC}"
else
    echo -e "${RED}✗ Flash Counter was not cast${NC}"
    grep -iE "flash counter" "$LOG" | head -8
    exit 1
fi

# (b) Flash Counter targets the Lightning Bolt on the stack
if grep -qE "targeting Lightning Bolt" "$LOG"; then
    echo -e "${GREEN}✓ Flash Counter targets Lightning Bolt${NC}"
else
    echo -e "${RED}✗ Flash Counter did not target the Bolt${NC}"
    grep -iE "target" "$LOG" | head -8
    exit 1
fi

# (c) Flash Counter resolves and counters the Bolt
if grep -qE "Flash Counter \([0-9]+\) counters Lightning Bolt" "$LOG"; then
    echo -e "${GREEN}✓ Flash Counter countered Lightning Bolt${NC}"
else
    echo -e "${RED}✗ Counterspell effect did not fire${NC}"
    grep -iE "counter|resolve" "$LOG" | head -10
    exit 1
fi

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "Full log: $LOG"
exit 0
