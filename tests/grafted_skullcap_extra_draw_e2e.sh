#!/usr/bin/env bash
# E2E test: the general "at the beginning of your draw step" phase trigger
# (Phase$ Draw -> TriggerEvent::BeginningOfDraw).
#
# Regression for the previously SILENTLY DROPPED Phase$ Draw trigger: the
# phase-trigger parser only mapped Upkeep / EndOfTurn / BeginCombat and let
# `Phase$ Draw` fall into the `_ => None` arm, so the whole trigger vanished
# and no extra card was drawn. Now `Phase$ Draw` maps to BeginningOfDraw,
# fires after the mandatory draw in draw_step, and `DB$ Draw` is converted to
# a DrawCards effect.
#
# Card under test: Grafted Skullcap (cardsfolder/g/grafted_skullcap.txt)
#   T:Mode$ Phase | Phase$ Draw | ValidPlayer$ You | TriggerZones$ Battlefield
#     | Execute$ TrigDraw | ...
#   SVar:TrigDraw:DB$ Draw
#
# Scenario (test_puzzles/grafted_skullcap_extra_draw.pzl):
# - P0 board: Grafted Skullcap + Forest. P0 turn 2, starts at upkeep.
# - On P0's draw step P0 draws TWO cards (mandatory + Skullcap). P1 (no
#   Skullcap) draws only one on its own draw step.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Grafted Skullcap Extra-Draw (BeginningOfDraw trigger) E2E Test ==="
echo

cd "$WORKSPACE_ROOT"

PUZZLE="$WORKSPACE_ROOT/test_puzzles/grafted_skullcap_extra_draw.pzl"
LOG=/tmp/grafted_skullcap_extra_draw_e2e.txt

if run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE" \
    --p1=zero --p2=zero \
    --seed 42 --verbosity 3 \
    > "$LOG" 2>&1; then
    echo -e "${GREEN}✓ Game completed${NC}"
else
    EXIT_STATUS=$?
    echo -e "${RED}✗ Game failed (exit $EXIT_STATUS)${NC}"
    head -80 "$LOG"
    exit 1
fi

# (a) The draw-step trigger fires (previously silently dropped).
if grep -qE "Trigger: Grafted Skullcap - At the beginning of your draw step" "$LOG"; then
    echo -e "${GREEN}✓ Grafted Skullcap draw-step trigger fired${NC}"
else
    echo -e "${RED}✗ Grafted Skullcap draw-step trigger did NOT fire${NC}"
    grep -iE "draw step|skullcap|trigger" "$LOG" | head -8
    exit 1
fi

# (b) On P0's FIRST draw step (the first "--- Draw Step ---" block) Player 1
#     drew TWO cards: the mandatory draw + the Skullcap extra draw. We inspect
#     just the lines up to the second draw-step header.
FIRST_DRAW_BLOCK=$(awk '/--- Draw Step ---/{c++} c==1{print} c==2{exit}' "$LOG")
P1_DRAWS=$(echo "$FIRST_DRAW_BLOCK" | grep -cE "Player 1 draws " || true)
if [ "$P1_DRAWS" -ge 2 ]; then
    echo -e "${GREEN}✓ Player 1 drew 2 cards on its draw step (found $P1_DRAWS)${NC}"
else
    echo -e "${RED}✗ Player 1 did not draw 2 cards on its draw step (found $P1_DRAWS)${NC}"
    echo "$FIRST_DRAW_BLOCK" | head -10
    exit 1
fi

# (c) Player 2 (no Skullcap) drew only ONE card on its own draw step — the
#     trigger is ValidPlayer$ You (controller-only), so it must NOT fire on the
#     opponent's draw step.
SECOND_DRAW_BLOCK=$(awk '/--- Draw Step ---/{c++} c==2{print} c==3{exit}' "$LOG")
P2_DRAWS=$(echo "$SECOND_DRAW_BLOCK" | grep -cE "Player 2 draws " || true)
if [ "$P2_DRAWS" -eq 1 ]; then
    echo -e "${GREEN}✓ Player 2 drew only 1 card (trigger is controller-only)${NC}"
else
    echo -e "${RED}✗ Player 2 draw count wrong (expected 1, found $P2_DRAWS)${NC}"
    echo "$SECOND_DRAW_BLOCK" | head -10
    exit 1
fi

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "Full log: $LOG"
exit 0
