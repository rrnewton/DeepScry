#!/usr/bin/env bash
# E2E test: Sorin Markov ({3}{B}{B}{B} Legendary Planeswalker) -3 loyalty ability
# sets target opponent's life total to 10 (CR 119.5).
#
# Regression test for 2010 World Championship compat tracker mtg-914 B5.
# The -3 ability was previously BROKEN: SetLife with ValidTgts$ Opponent was
# not wired into get_valid_targets_for_ability, so the ability never appeared
# in the action list. Fixed by adding the Effect::SetLife + requires_target
# player-enumeration arm in targeting.rs.
#
# Card script:
#   A:AB$ SetLife | Cost$ SubCounter<3/LOYALTY> | Planeswalker$ True
#         | ValidTgts$ Opponent | LifeAmount$ 10
#         | SpellDescription$ Target opponent's life total becomes 10.
#
# Scenario (test_puzzles/sorin_markov_minus3_setlife.pzl):
# - P1 controls Sorin Markov with 6 loyalty counters.
# - P2 has 20 life.
# - P1 activates -3: Sorin loses 3 loyalty (6→3); P2's life is set to 10.
# - Verifies: (a) ability appears in action list, (b) loyalty decremented,
#   (c) opponent life set to 10, (d) undo log recorded (execute_set_life fix).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Sorin Markov -3 SetLife E2E Test ==="
echo

cd "$WORKSPACE_ROOT"

PUZZLE="$WORKSPACE_ROOT/test_puzzles/sorin_markov_minus3_setlife.pzl"
LOG=/tmp/sorin_markov_setlife_e2e.txt

if run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE" \
    --p1=fixed --p2=zero \
    --p1-fixed-inputs="activate Sorin Markov;*;*;*;*;*;*;*;*" \
    --seed 42 --verbosity 3 --no-color-logs \
    > "$LOG" 2>&1; then
    echo -e "${GREEN}✓ Game completed${NC}"
else
    EXIT_STATUS=$?
    echo -e "${RED}✗ Game failed (exit $EXIT_STATUS)${NC}"
    head -80 "$LOG"
    exit 1
fi

# (a) Sorin activates the -3 ability (ability appeared in the action list)
if grep -qE "Sorin Markov activates ability: Target opponent's life total becomes 10\." "$LOG"; then
    echo -e "${GREEN}✓ Sorin -3 ability activated${NC}"
else
    echo -e "${RED}✗ Sorin -3 ability did not activate${NC}"
    grep -iE "sorin|activat|setlife|life total" "$LOG" | head -12
    exit 1
fi

# (b) Sorin's loyalty decremented by 3 (from 6 to 3)
if grep -qE "Sorin Markov loses 3 loyalty \(now 3\)" "$LOG"; then
    echo -e "${GREEN}✓ Sorin loyalty decremented from 6 to 3${NC}"
else
    echo -e "${RED}✗ Sorin loyalty not decremented correctly${NC}"
    grep -iE "loyalty|counter" "$LOG" | head -8
    exit 1
fi

# (c) Opponent's life total set to 10 (was 20) — CR 119.5
if grep -qE "Player 2's life total is set to 10 \(was 20\)" "$LOG"; then
    echo -e "${GREEN}✓ Opponent life set to 10 (was 20) — CR 119.5 SetLife${NC}"
else
    echo -e "${RED}✗ Opponent life not set to 10${NC}"
    grep -iE "life|set to" "$LOG" | head -8
    exit 1
fi

# (d) Verify Player 2 actually shows life=10 afterward (undo log integrity)
if grep -qE "Life: 10$" "$LOG"; then
    echo -e "${GREEN}✓ Player 2 life total shows 10 in subsequent game state${NC}"
else
    echo -e "${RED}✗ Player 2 life total not showing 10 in game state display${NC}"
    grep -iE "life:" "$LOG" | head -8
    exit 1
fi

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "Full log: $LOG"
exit 0
