#!/usr/bin/env bash
# E2E regression test: Swords to Plowshares life gain uses LAST-KNOWN
# INFORMATION (CR 608.2h) including continuous static buffs.
#
# Card compat mtg-297. The life amount must reflect the exiled creature's power
# AS IT LAST EXISTED on the battlefield — counting continuous static buffs that
# the leave-the-battlefield event then strips. This guards the pre-resolution
# CR-613-layer snapshot on BOTH resolution paths (the choice-routing
# collect-effects path is exercised here via the `fixed` controller).
#
# Scenario (test_puzzles/swords_lki_static_buff.pzl):
# - P2 board: Sedge Troll (2/2 base) + Swamp -> Troll is a 3/3 via its static
#   "+1/+1 while you control a Swamp".
# - P1 casts Swords on the Troll: it is exiled and P2 gains 3 life (NOT 2).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Swords LKI Static Buff E2E Test ==="
echo

cd "$WORKSPACE_ROOT"

PUZZLE="$WORKSPACE_ROOT/test_puzzles/swords_lki_static_buff.pzl"
LOG=/tmp/swords_lki_static_buff_e2e.txt

if run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE" \
    --p1=fixed --p2=zero \
    --p1-fixed-inputs="cast Swords to Plowshares;*;*" \
    --p2-fixed-inputs="" \
    --stop-on-choice=8 --seed 42 --verbosity 3 --no-color-logs \
    > "$LOG" 2>&1; then
    echo -e "${GREEN}✓ Game completed${NC}"
else
    EXIT_STATUS=$?
    echo -e "${RED}✗ Game failed (exit $EXIT_STATUS)${NC}"
    head -80 "$LOG"
    exit 1
fi

# (a) Swords exiles the buffed creature
if grep -qE "Sedge Troll \([0-9]+\) is exiled" "$LOG"; then
    echo -e "${GREEN}✓ Sedge Troll exiled by Swords${NC}"
else
    echo -e "${RED}✗ Sedge Troll not exiled${NC}"
    grep -iE "sedge|exile" "$LOG" | head -8
    exit 1
fi

# (b) The controller gains 3 life (LKI power = 3 with Swamp buff), NOT 2
if grep -qE "Player 2 gains 3 life \(life: 23\)" "$LOG"; then
    echo -e "${GREEN}✓ Gained 3 life (LKI power incl. static buff)${NC}"
else
    echo -e "${RED}✗ Did not gain 3 life — LKI / static buff not counted${NC}"
    grep -iE "gains|life" "$LOG" | head -8
    exit 1
fi
# Guard against the naive post-exile read (would be 2).
if grep -qE "Player 2 gains 2 life" "$LOG"; then
    echo -e "${RED}✗ Gained 2 life — power read AFTER the buff was stripped (CR 608.2h violation)${NC}"
    exit 1
fi

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "Full log: $LOG"
exit 0
