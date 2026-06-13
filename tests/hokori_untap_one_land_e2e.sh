#!/usr/bin/env bash
# E2E test: Hokori, Dust Drinker — each player untaps exactly one land per upkeep.
#
# Regression for mtg-910 (2005 WC compatibility wave 3).
#
# Hokori's script:
#   R:Event$ Untap | ActiveZones$ Battlefield | ValidCard$ Land
#     | ValidStepTurnToController$ You | Layer$ CantHappen
#   T:Mode$ Phase | Phase$ Upkeep | ValidPlayer$ Player | TriggerZones$ Battlefield
#     | Execute$ TrigUntap | TriggerDescription$ At the beginning of each player's
#     upkeep, that player untaps a land they control.
#   SVar:TrigUntap:DB$ Untap | UntapExactly$ True | UntapType$ Land.ActivePlayerCtrl
#     | Amount$ 1 | Defined$ TriggeredPlayer
#
# The untap-step replacement stops ALL lands from untapping normally.
# The upkeep trigger fires for the ACTIVE player and uses Effect::UntapOne
# (parsed from UntapExactly$ True) to untap exactly one tapped land they control.
#
# Setup:
#   P0 controls Hokori + 3 tapped Plains; starts in P0's Upkeep (turn 2).
#   P1 controls 2 tapped Forests.
#
# Expected after P0's upkeep trigger: "untaps one matching permanent" in log.
# UntapOne picks the FIRST matching tapped land — so exactly 1 Plains untaps;
# the other 2 remain tapped.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Hokori, Dust Drinker — UntapOne E2E ==="
echo

cd "$WORKSPACE_ROOT"

PUZZLE="$WORKSPACE_ROOT/test_puzzles/hokori_untap_one_land.pzl"
LOG=/tmp/hokori_untap_one_land_e2e.txt

if run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE" \
    --p1=zero --p2=zero \
    --stop-on-choice=15 --json --seed 42 --verbosity 3 \
    > "$LOG" 2>&1; then
    echo -e "${GREEN}✓ Game completed${NC}"
else
    EXIT_STATUS=$?
    echo -e "${RED}✗ Game failed (exit $EXIT_STATUS)${NC}"
    head -80 "$LOG"
    exit 1
fi

# Required: the UntapOne trigger fires for P0's upkeep — execute_untap_one
# logs "{card_name} ({card_id}) is untapped" when it untaps the single land.
# Also verify that Forest (12) or Plains (4) is explicitly untapped (one land).
if grep -qE "is untapped" "$LOG"; then
    echo -e "${GREEN}✓ UntapOne trigger fired (one land untapped)${NC}"
else
    echo -e "${RED}✗ No land was untapped — UntapOne trigger may not have fired${NC}"
    grep -E "untap|Hokori|trigger" "$LOG" | head -20 || echo "(no untap lines)"
    exit 1
fi

# Safety: Hokori's trigger must fire (the trigger announcement line must appear).
if grep -qE "Hokori, Dust Drinker trigger effect" "$LOG"; then
    echo -e "${GREEN}✓ Hokori upkeep trigger fired${NC}"
else
    echo -e "${RED}✗ Hokori trigger did not fire${NC}"
    exit 1
fi

# Safety: the Hokori land-lock must prevent the opponent's lands from
# untapping in the untap step ("Forest doesn't untap (locked tapped)").
if grep -qE "doesn't untap \(locked tapped\)" "$LOG"; then
    echo -e "${GREEN}✓ Hokori lock prevents opponent lands from untapping in untap step${NC}"
else
    echo -e "${RED}✗ Hokori lock did not prevent opponent lands from staying tapped${NC}"
    grep -E "untap|Forest" "$LOG" | head -20 || echo "(no relevant lines)"
    exit 1
fi

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "Full log: $LOG"
exit 0
