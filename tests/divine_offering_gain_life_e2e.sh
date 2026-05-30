#!/usr/bin/env bash
# E2E test: Divine Offering ({1}{W} Instant) destroys a target artifact; the
# caster gains life equal to the artifact's mana value (dynamic-amount life gain).
#
# Card compat mtg-500. Script:
#   A:SP$ Destroy | ValidTgts$ Artifact | SubAbility$ DBGainLife
#   SVar:DBGainLife:DB$ GainLife | Defined$ You | LifeAmount$ X
#   SVar:X:Targeted$CardManaCost
#
# Scenario (test_puzzles/divine_offering_gain_life.pzl):
# - P1 hand: Divine Offering. P1 board: Plains x2 (pays {1}{W}).
# - P2 board: Jalum Tome (a {3} artifact, mana value 3).
# - P1 casts Divine Offering on Jalum Tome: it is destroyed and THE CASTER (P1)
#   gains 3 life (= the artifact's mana value).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Divine Offering Gain Life E2E Test ==="
echo

cd "$WORKSPACE_ROOT"

PUZZLE="$WORKSPACE_ROOT/test_puzzles/divine_offering_gain_life.pzl"
LOG=/tmp/divine_offering_gain_life_e2e.txt

if run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE" \
    --p1=fixed --p2=zero \
    --p1-fixed-inputs="cast Divine Offering;*;*" \
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

# (a) Divine Offering targets the opponent's artifact
if grep -qE "targeting Jalum Tome" "$LOG"; then
    echo -e "${GREEN}✓ Divine Offering targeted Jalum Tome (artifact)${NC}"
else
    echo -e "${RED}✗ Divine Offering did not target the artifact${NC}"
    grep -iE "divine|target" "$LOG" | head -8
    exit 1
fi

# (b) The artifact is destroyed and moves to graveyard
if grep -qE "Jalum Tome \([0-9]+\) goes to graveyard" "$LOG"; then
    echo -e "${GREEN}✓ Jalum Tome destroyed (to graveyard)${NC}"
else
    echo -e "${RED}✗ Jalum Tome not destroyed${NC}"
    grep -iE "destroy|jalum|graveyard" "$LOG" | head -8
    exit 1
fi

# (c) The caster (Player 1) gains 3 life (= artifact's mana value)
if grep -qE "Player 1 gains 3 life \(life: 23\)" "$LOG"; then
    echo -e "${GREEN}✓ Caster gained 3 life (= mana value)${NC}"
else
    echo -e "${RED}✗ Caster did not gain 3 life${NC}"
    grep -iE "gains|life" "$LOG" | head -8
    exit 1
fi

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "Full log: $LOG"
exit 0
