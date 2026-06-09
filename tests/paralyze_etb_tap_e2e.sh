#!/usr/bin/env bash
# E2E test: Paralyze's ETB trigger taps the enchanted creature (mtg-529).
#
# Paralyze ({B} Enchant Creature):
#   T:Mode$ ChangesZone | Destination$ Battlefield | ValidCard$ Card.Self | Execute$ TrigTap
#   SVar:TrigTap:DB$ Tap | Defined$ Enchanted
# "When Paralyze enters, tap enchanted creature."
#
# This exercises the `Defined$ Enchanted` resolution: the Aura's ETB trigger
# resolves `DB$ Tap | Defined$ Enchanted` to the permanent it just attached to
# (Card::attached_to, threaded via TriggerContext::enchanted) and taps it.
#
# Scenario: P0 casts Paralyze on P1's UNTAPPED Grizzly Bears; the Bears must end
# up tapped (CR 303.4 — Aura attaches on resolution; its ETB trigger then fires).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Paralyze: ETB taps the enchanted creature E2E Test ==="
echo

cd "$WORKSPACE_ROOT"

PUZZLE="$WORKSPACE_ROOT/test_puzzles/paralyze_etb_tap.pzl"

LOG=/tmp/paralyze_etb_tap.txt
# P0 casts Paralyze (menu [1]); the only legal Enchant Creature target is P1's
# Grizzly Bears ([0]); then pass out.
printf 'cast Paralyze\n0\npass\npass\n' \
    | "$MTG_BIN" tui --start-state "$PUZZLE" --p1 tui --p2 zero \
        --seed 42 --verbosity 3 > "$LOG" 2>&1 || true

if grep -qE "Paralyze .* enchants Grizzly Bears|Paralyze enchants Grizzly Bears" "$LOG"; then
    echo -e "${GREEN}✓ Paralyze attached to Grizzly Bears${NC}"
else
    echo -e "${RED}✗ Paralyze did not enchant Grizzly Bears${NC}"
    grep -iE "paralyze|enchant" "$LOG" || echo "(none)"
    exit 1
fi

# The ETB trigger must tap the enchanted creature (Defined$ Enchanted).
if grep -qE "Grizzly Bears \([0-9]+\) becomes tapped" "$LOG"; then
    echo -e "${GREEN}✓ ETB trigger tapped the enchanted Grizzly Bears (Defined\$ Enchanted)${NC}"
else
    echo -e "${RED}✗ Paralyze's ETB trigger did NOT tap the enchanted creature${NC}"
    grep -iE "tap|enchant|grizzly" "$LOG" || echo "(no tap line)"
    exit 1
fi

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
exit 0
