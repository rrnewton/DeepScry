#!/usr/bin/env bash
# E2E test: Swords to Plowshares ({W} Instant) exiles a creature; its
# controller gains life equal to the creature's power (dynamic-amount life gain).
#
# Card compat mtg-297 / mtg-547. Script:
#   A:SP$ ChangeZone | Origin$ Battlefield | Destination$ Exile | SubAbility$ DBGainLife
#   SVar:DBGainLife:DB$ GainLife | Defined$ TargetedController | LifeAmount$ X
#   SVar:X:Targeted$CardPower
#
# Scenario (test_puzzles/swords_to_plowshares_gain_life.pzl):
# - P1 hand: Swords to Plowshares. P1 board: Plains (pays {W}).
# - P2 board: Hill Giant (a 3/3 vanilla creature).
# - P1 casts Swords on Hill Giant: it is exiled and ITS CONTROLLER (P2) gains
#   3 life (CR 608.2g/2h: power captured via last-known information). The
#   Swords caster gains nothing.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Swords to Plowshares Gain Life E2E Test ==="
echo

cd "$WORKSPACE_ROOT"

PUZZLE="$WORKSPACE_ROOT/test_puzzles/swords_to_plowshares_gain_life.pzl"
LOG=/tmp/swords_to_plowshares_gain_life_e2e.txt

if run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE" \
    --p1=fixed --p2=zero \
    --p1-fixed-inputs="cast Swords to Plowshares;*;*" \
    --p2-fixed-inputs="" \
    --stop-on-choice=6 --seed 42 --verbosity 3 --no-color-logs \
    > "$LOG" 2>&1; then
    echo -e "${GREEN}✓ Game completed${NC}"
else
    EXIT_STATUS=$?
    echo -e "${RED}✗ Game failed (exit $EXIT_STATUS)${NC}"
    head -80 "$LOG"
    exit 1
fi

# (a) Swords targets the opponent's creature
if grep -qE "targeting Hill Giant" "$LOG"; then
    echo -e "${GREEN}✓ Swords targeted Hill Giant${NC}"
else
    echo -e "${RED}✗ Swords did not target the creature${NC}"
    grep -iE "swords|target" "$LOG" | head -8
    exit 1
fi

# (b) The creature is exiled
if grep -qE "Hill Giant \([0-9]+\) is exiled" "$LOG"; then
    echo -e "${GREEN}✓ Hill Giant exiled by Swords${NC}"
else
    echo -e "${RED}✗ Hill Giant not exiled${NC}"
    grep -iE "exile|hill giant" "$LOG" | head -8
    exit 1
fi

# (c) The creature's controller (Player 2) gains 3 life (= its power)
if grep -qE "Player 2 gains 3 life \(life: 23\)" "$LOG"; then
    echo -e "${GREEN}✓ Hill Giant's controller gained 3 life (= power)${NC}"
else
    echo -e "${RED}✗ Controller did not gain 3 life${NC}"
    grep -iE "gains|life" "$LOG" | head -8
    exit 1
fi

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "Full log: $LOG"
exit 0
