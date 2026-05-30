#!/usr/bin/env bash
# E2E test: Blue Elemental Blast ({U} Instant Charm — counter red spell
# OR destroy red permanent). Regression test for "Card Compatibility:
# Blue Elemental Blast" (mtg-487). Card script is
#
#   A:SP$ Charm | Choices$ DBCounter,DBDestroy
#   SVar:DBCounter:DB$ Counter | TargetType$ Spell | ValidTgts$ Card.Red
#   SVar:DBDestroy:DB$ Destroy | ValidTgts$ Permanent.Red
#
# Exercises both modes via puzzle + fixed-input scripts.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Blue Elemental Blast E2E Test ==="
echo

cd "$WORKSPACE_ROOT"

# ----- Mode 1: counter target red spell -----
PUZZLE_COUNTER="$WORKSPACE_ROOT/test_puzzles/blue_elemental_blast_counter.pzl"
LOG_COUNTER=/tmp/blue_elemental_blast_counter_e2e.txt

if run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE_COUNTER" \
    --p1=fixed --p2=fixed \
    --p1-fixed-inputs="cast Lightning Bolt;*;*" \
    --p2-fixed-inputs="cast Blue Elemental Blast;0;*;*" \
    --stop-on-choice=15 --seed 42 --verbosity 3 \
    > "$LOG_COUNTER" 2>&1; then
    echo -e "${GREEN}✓ Counter-mode game completed${NC}"
else
    EXIT_STATUS=$?
    echo -e "${RED}✗ Counter-mode game failed (exit $EXIT_STATUS)${NC}"
    head -80 "$LOG_COUNTER"
    exit 1
fi

if grep -qE "Player 2 chooses mode: Counter target red spell" "$LOG_COUNTER" \
   && grep -qE "Blue Elemental Blast \([0-9]+\) counters Lightning Bolt" "$LOG_COUNTER"; then
    echo -e "${GREEN}✓ Counter mode selected and Lightning Bolt countered${NC}"
else
    echo -e "${RED}✗ Counter mode did not fire${NC}"
    grep -iE "blue elemental|counter|mode|target" "$LOG_COUNTER" | head -10
    exit 1
fi

# ----- Mode 2: destroy target red permanent -----
PUZZLE_DESTROY="$WORKSPACE_ROOT/test_puzzles/blue_elemental_blast_destroy.pzl"
LOG_DESTROY=/tmp/blue_elemental_blast_destroy_e2e.txt

if run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE_DESTROY" \
    --p1=zero --p2=fixed \
    --p2-fixed-inputs="cast Blue Elemental Blast;1;*;*" \
    --stop-on-choice=15 --seed 42 --verbosity 3 \
    > "$LOG_DESTROY" 2>&1; then
    echo -e "${GREEN}✓ Destroy-mode game completed${NC}"
else
    EXIT_STATUS=$?
    echo -e "${RED}✗ Destroy-mode game failed (exit $EXIT_STATUS)${NC}"
    head -80 "$LOG_DESTROY"
    exit 1
fi

if grep -qE "Player 2 chooses mode: Destroy target red permanent" "$LOG_DESTROY" \
   && grep -qE "Blue Elemental Blast \([0-9]+\) destroys Mons's Goblin Raiders" "$LOG_DESTROY" \
   && grep -qE "Mons's Goblin Raiders \([0-9]+\) goes to graveyard" "$LOG_DESTROY"; then
    echo -e "${GREEN}✓ Destroy mode selected and red creature destroyed/graveyarded${NC}"
else
    echo -e "${RED}✗ Destroy mode did not fire${NC}"
    grep -iE "blue elemental|destroy|mode|target|graveyard|goblin" "$LOG_DESTROY" | head -10
    exit 1
fi

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "Counter-mode log: $LOG_COUNTER"
echo "Destroy-mode log: $LOG_DESTROY"
exit 0
