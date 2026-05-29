#!/usr/bin/env bash
# E2E test: Shivan Dragon's Flying keyword is honored in combat.
#
# Regression test for the "Card Compatibility: Shivan Dragon" beads issue
# (mtg-541). Shivan Dragon is a 5/5 Flying Dragon with firebreathing
# ({R}: +1/+0). This test focuses on the FLYING aspect: a non-flying,
# non-reach blocker (Grizzly Bears) may NOT block it (CR 509.1b / 702.9b),
# so Shivan's combat damage must hit the defending player.
#
# (Firebreathing / pump are already covered by puzzle_e2e.rs:
#  test_shivan_dragon_firebreathing_combat and test_shivan_dragon_pump_ability.)
#
# Scenario (test_puzzles/shivan_dragon_flying_block.pzl):
# - P1 board: Shivan Dragon + Mountains. P2 board: Grizzly Bears (2/2, no
#   flying/reach).
# - P1 (heuristic) attacks with Shivan; P2 cannot block; damage hits P2.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Shivan Dragon Flying Block E2E Test ==="
echo

cd "$WORKSPACE_ROOT"

PUZZLE="$WORKSPACE_ROOT/test_puzzles/shivan_dragon_flying_block.pzl"
LOG=/tmp/shivan_dragon_flying_block_e2e.txt

if run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE" \
    --p1=heuristic --p2=heuristic \
    --seed 5 --verbosity 2 \
    > "$LOG" 2>&1; then
    echo -e "${GREEN}✓ Game completed${NC}"
else
    EXIT_STATUS=$?
    echo -e "${RED}✗ Game failed (exit $EXIT_STATUS)${NC}"
    head -80 "$LOG"
    exit 1
fi

# (a) Shivan Dragon is declared as an attacker
if grep -qE "declares Shivan Dragon \([0-9]+\) \([0-9]+/[0-9]+\) as attacker" "$LOG"; then
    echo -e "${GREEN}✓ Shivan Dragon attacked${NC}"
else
    echo -e "${RED}✗ Shivan Dragon never attacked${NC}"
    grep -iE "shivan|attacker" "$LOG" | head -8
    exit 1
fi

# (b) Grizzly Bears is NEVER declared as a blocker for Shivan (flying)
if grep -qE "declares Grizzly Bears .* as blocker for Shivan Dragon" "$LOG"; then
    echo -e "${RED}✗ Grizzly Bears illegally blocked the flyer${NC}"
    grep -iE "blocker" "$LOG" | head -8
    exit 1
else
    echo -e "${GREEN}✓ Grizzly Bears did not (could not) block the flyer${NC}"
fi

# (c) Shivan's combat damage hit the defending PLAYER (not a blocker)
if grep -qE "Shivan Dragon \([0-9]+\) deals [0-9]+ damage to Player 2" "$LOG"; then
    echo -e "${GREEN}✓ Shivan Dragon dealt combat damage to Player 2${NC}"
else
    echo -e "${RED}✗ Shivan Dragon did not damage the player${NC}"
    grep -iE "shivan.*deals" "$LOG" | head -8
    exit 1
fi

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "Full log: $LOG"
exit 0
