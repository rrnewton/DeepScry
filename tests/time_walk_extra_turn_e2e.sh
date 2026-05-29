#!/usr/bin/env bash
# E2E test: Time Walk grants its CASTER an extra turn that is taken
# immediately after the current turn, before the opponent's turn (CR 500.7).
#
# Card compat (mtg-551, 1994 Old School 'Troll Disk' deck mtg-562):
#   A:SP$ AddTurn | NumTurns$ 1
#
# Regression guard: the AddTurn handler previously pushed the extra turn onto
# TurnStructure::extra_turns (a write-only, never-drained field) instead of
# GameState::extra_turns (the queue the turn-rotation code actually consumes),
# so the extra turn silently never happened and play rotated straight to the
# opponent. See mtg-engine/src/game/actions/mod.rs (Effect::AddTurn).
#
# Scenario: P0 starts on turn 1 with Time Walk + two Islands. P0 casts Time
# Walk; the log must show the caster taking the extra turn, and turn 2 must be
# P0's again (not P1's).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Time Walk Extra Turn E2E ==="
echo

cd "$WORKSPACE_ROOT"

PUZZLE="$WORKSPACE_ROOT/test_puzzles/time_walk_extra_turn.pzl"
LOG=/tmp/time_walk_extra_turn_e2e.txt

if run_mtg_with_timeout 40 tui \
    --start-state "$PUZZLE" \
    --p1=fixed --p1-fixed-inputs="cast Time Walk;*" \
    --p2=zero \
    --seed 42 --verbosity 2 \
    > "$LOG" 2>&1; then
    echo -e "${GREEN}✓ Game completed${NC}"
else
    EXIT_STATUS=$?
    echo -e "${RED}✗ Game failed (exit $EXIT_STATUS)${NC}"
    head -80 "$LOG"
    exit 1
fi

# Required: Time Walk resolves.
if grep -qE "Time Walk \([0-9]+\) resolves" "$LOG"; then
    echo -e "${GREEN}✓ Time Walk resolved${NC}"
else
    echo -e "${RED}✗ Time Walk did not resolve${NC}"
    grep -E "Time Walk" "$LOG" || echo "(none)"
    exit 1
fi

# Required: the CASTER (Player 1) is granted the extra turn (correct attribution).
if grep -qE "Time Walk \([0-9]+\) grants Player 1 1 extra turn" "$LOG"; then
    echo -e "${GREEN}✓ Extra turn attributed to the caster (Player 1)${NC}"
else
    echo -e "${RED}✗ Extra turn attributed to the wrong player${NC}"
    grep -E "extra turn" "$LOG" || echo "(none)"
    exit 1
fi

# Required: the extra turn is actually queued and taken.
if grep -qE "Extra turn for Player 1!" "$LOG"; then
    echo -e "${GREEN}✓ Extra turn taken (queue consumed)${NC}"
else
    echo -e "${RED}✗ Extra turn was never taken (dead-field regression)${NC}"
    grep -E "Turn [0-9]+ -|Extra turn" "$LOG" || echo "(none)"
    exit 1
fi

# Required: turn 2 is Player 1's again (consecutive), NOT Player 2's. Verify the
# turn-2 header names Player 1.
TURN2=$(grep -E "^Turn 2 - " "$LOG" | head -1)
if echo "$TURN2" | grep -qE "Player 1"; then
    echo -e "${GREEN}✓ Turn 2 is the caster's extra turn ($TURN2)${NC}"
else
    echo -e "${RED}✗ Turn 2 was not the caster's extra turn: $TURN2${NC}"
    grep -E "^Turn [0-9]+ - " "$LOG" | head -4
    exit 1
fi

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "Full log: $LOG"
exit 0
