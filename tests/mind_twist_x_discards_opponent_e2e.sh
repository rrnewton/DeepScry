#!/usr/bin/env bash
# E2E test: Mind Twist with X=N actually causes the OPPONENT to discard N
# cards at random.
#
# Regression test for mtg-564 (user bug report, fix-gameplay-bugs-4pack):
# Before the fix, `SP$ Discard | ValidTgts$ Player | NumCards$ X | Mode$ Random`
# parsed to a placeholder PlayerId. `resolve_target_for_effect` then defaulted
# the placeholder to `card_owner` (the CASTER). So when an opponent cast
# Mind Twist with X=8 on you, the engine made the CASTER discard from their
# own (empty) hand and you discarded nothing.
#
# After the fix, the converter assigns `PlayerId::target_opponent()` whenever
# `ValidTgts$ Player` is specified, and the resolver maps that sentinel to
# `opponent_id` (matching CR 116.2c "Target player ..." for a 2-player game).
#
# Test scenario:
# - P0 casts Mind Twist with X=8 (9 Swamps available, pays 8B).
# - P1 has 8 Plains in hand.
# - After resolution, P1's hand must be empty (8 discards).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Mind Twist X=8 Discards Opponent's Hand E2E ==="
echo

cd "$WORKSPACE_ROOT"

PUZZLE="$WORKSPACE_ROOT/test_puzzles/mind_twist_x8_opponent.pzl"
LOG=/tmp/mind_twist_x8_opponent_e2e.txt

if run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE" \
    --p1=fixed --p2=zero \
    --p1-fixed-inputs="cast Mind Twist;pass" \
    --p2-fixed-inputs="" \
    --stop-on-choice=5 --json --seed 42 --verbosity 3 \
    > "$LOG" 2>&1; then
    echo -e "${GREEN}✓ Game completed${NC}"
else
    EXIT_STATUS=$?
    echo -e "${RED}✗ Game failed (exit $EXIT_STATUS)${NC}"
    head -80 "$LOG"
    exit 1
fi

# Required: Player 2 (opponent) discards 8 cards
DISCARD_COUNT=$(grep -cE "^  Player 2 discards " "$LOG" || true)
if [[ "$DISCARD_COUNT" -eq 8 ]]; then
    echo -e "${GREEN}✓ Opponent discarded exactly 8 cards${NC}"
else
    echo -e "${RED}✗ Expected 8 opponent discards, got $DISCARD_COUNT${NC}"
    grep -E "discards " "$LOG" || echo "(none)"
    exit 1
fi

# Required: CASTER (Player 1) discards nothing
if grep -qE "^  Player 1 discards " "$LOG"; then
    echo -e "${RED}✗ Regression: caster discarded instead of opponent${NC}"
    grep -E "^  Player 1 discards " "$LOG"
    exit 1
fi
echo -e "${GREEN}✓ Caster did not discard${NC}"

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "Full log: $LOG"
exit 0
