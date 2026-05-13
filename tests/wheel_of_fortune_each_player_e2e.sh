#!/usr/bin/env bash
# E2E test: Wheel of Fortune — each player discards their hand, then draws seven
#
# Regression test for mtg-356951 (Card Compatibility: Wheel of Fortune).
# Verifies the SP$ Discard Mode$ Hand Defined$ Player + SubAbility$ DBEachDraw
# chain resolves with both effects applying to BOTH players, in order
# (discard before draw, per CR 608.2c).
#
# Test scenario:
# - P1 hand: Wheel of Fortune + 2 Plains; library: 12 Plains
# - P2 hand: 2 Forests; library: 12 Forests
# - Cast Wheel of Fortune
# - Verify:
#   (a) P1 discards 2 Plains (everything except the cast Wheel)
#   (b) P2 discards 2 Forests
#   (c) P1 draws 7 Plains
#   (d) P2 draws 7 Forests
#   (e) Discard log lines precede draw log lines
#   (f) Friendly log line "causes each player to discard their hand"

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Wheel of Fortune: Each Player Discards then Draws Seven E2E Test ==="
echo

cd "$WORKSPACE_ROOT"

PUZZLE="$WORKSPACE_ROOT/test_puzzles/wheel_of_fortune_each_player.pzl"
LOG=/tmp/wheel_of_fortune_e2e.txt

if run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE" \
    --p1=fixed --p2=zero \
    --p1-fixed-inputs="cast Wheel of Fortune" \
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

# (a) P1 discards 2 Plains
P1_DISCARDS=$(grep -cE "^  Player 1 discards Plains" "$LOG" || true)
if [ "$P1_DISCARDS" -ge 2 ]; then
    echo -e "${GREEN}✓ P1 discarded ${P1_DISCARDS} Plains${NC}"
else
    echo -e "${RED}✗ P1 only discarded ${P1_DISCARDS} Plains (expected ≥2)${NC}"
    exit 1
fi

# (b) P2 discards 2 Forests
P2_DISCARDS=$(grep -cE "^  Player 2 discards Forest" "$LOG" || true)
if [ "$P2_DISCARDS" -ge 2 ]; then
    echo -e "${GREEN}✓ P2 discarded ${P2_DISCARDS} Forests${NC}"
else
    echo -e "${RED}✗ P2 only discarded ${P2_DISCARDS} Forests (expected ≥2)${NC}"
    exit 1
fi

# (c) P1 draws 7 Plains (Wheel-induced; subsequent turn draws may add more)
P1_DRAWS=$(grep -cE "^  Player 1 draws Plains" "$LOG" || true)
if [ "$P1_DRAWS" -ge 7 ]; then
    echo -e "${GREEN}✓ P1 drew ${P1_DRAWS} Plains (≥7)${NC}"
else
    echo -e "${RED}✗ P1 only drew ${P1_DRAWS} Plains (expected ≥7)${NC}"
    exit 1
fi

# (d) P2 draws 7 Forests
P2_DRAWS=$(grep -cE "^  Player 2 draws Forest" "$LOG" || true)
if [ "$P2_DRAWS" -ge 7 ]; then
    echo -e "${GREEN}✓ P2 drew ${P2_DRAWS} Forests (≥7)${NC}"
else
    echo -e "${RED}✗ P2 only drew ${P2_DRAWS} Forests (expected ≥7)${NC}"
    exit 1
fi

# (e) Discard log lines precede draw log lines (sequential resolution)
FIRST_DISCARD_LINE=$(grep -nE "Player [12] discards" "$LOG" | head -1 | cut -d: -f1)
FIRST_WHEEL_DRAW_LINE=$(grep -nE "Player [12] draws" "$LOG" | head -1 | cut -d: -f1)
if [ -n "$FIRST_DISCARD_LINE" ] && [ -n "$FIRST_WHEEL_DRAW_LINE" ] && \
   [ "$FIRST_DISCARD_LINE" -lt "$FIRST_WHEEL_DRAW_LINE" ]; then
    echo -e "${GREEN}✓ Discards precede draws (line $FIRST_DISCARD_LINE < $FIRST_WHEEL_DRAW_LINE)${NC}"
else
    echo -e "${RED}✗ Effect ordering wrong: first discard=$FIRST_DISCARD_LINE, first draw=$FIRST_WHEEL_DRAW_LINE${NC}"
    exit 1
fi

# (f) Friendly summary log line (no raw Player 4294967295 / 255 sentinel values)
if grep -qE "causes each player to (discard their hand|draw)" "$LOG"; then
    echo -e "${GREEN}✓ Friendly 'each player' summary log emitted${NC}"
else
    echo -e "${RED}✗ Missing 'each player' summary log${NC}"
    exit 1
fi

if grep -qE "Player 4294967295|discard 255 card" "$LOG"; then
    echo -e "${RED}✗ Regression: raw sentinel values leaked into game log${NC}"
    grep -E "Player 4294967295|discard 255 card" "$LOG" | head -3
    exit 1
fi

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "Full log: $LOG"
exit 0
