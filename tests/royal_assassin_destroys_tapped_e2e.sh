#!/usr/bin/env bash
# E2E test: Royal Assassin taps to destroy a tapped creature.
#
# Card compat (mtg-537, 1994 Old School 'Mono Black Rogerbrand' deck mtg-560):
#   A:AB$ Destroy | Cost$ T | ValidTgts$ Creature.tapped
#
# Scenario: P0 has an untapped Royal Assassin; P1 has a tapped Serra Angel.
# Royal Assassin activates (tap cost), targets the tapped Serra Angel, and it
# is destroyed (moved to the graveyard).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Royal Assassin Destroys Tapped Creature E2E ==="
echo

cd "$WORKSPACE_ROOT"

PUZZLE="$WORKSPACE_ROOT/test_puzzles/royal_assassin_destroys_tapped.pzl"
LOG=/tmp/royal_assassin_destroys_tapped_e2e.txt

if run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE" \
    --p1=fixed --p2=zero \
    --p1-fixed-inputs="activate Royal Assassin;0;pass;pass;pass" \
    --p2-fixed-inputs="" \
    --stop-on-choice=6 --seed 42 --verbosity 3 \
    > "$LOG" 2>&1; then
    echo -e "${GREEN}✓ Game completed${NC}"
else
    EXIT_STATUS=$?
    echo -e "${RED}✗ Game failed (exit $EXIT_STATUS)${NC}"
    head -80 "$LOG"
    exit 1
fi

# Required: Royal Assassin's destroy ability resolved against Serra Angel.
if grep -qE "Royal Assassin activates ability: Destroy target tapped creature" "$LOG"; then
    echo -e "${GREEN}✓ Royal Assassin activated its destroy ability${NC}"
else
    echo -e "${RED}✗ Royal Assassin did not activate its destroy ability${NC}"
    grep -E "Royal Assassin|Destroy" "$LOG" || echo "(none)"
    exit 1
fi

# Required: Serra Angel went to the graveyard.
if grep -qE "Serra Angel \([0-9]+\) goes to graveyard" "$LOG"; then
    echo -e "${GREEN}✓ Tapped Serra Angel was destroyed${NC}"
else
    echo -e "${RED}✗ Serra Angel was not destroyed${NC}"
    grep -E "Serra Angel" "$LOG" || echo "(none)"
    exit 1
fi

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "Full log: $LOG"
exit 0
