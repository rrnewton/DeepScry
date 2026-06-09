#!/usr/bin/env bash
# E2E test: Animate Dead's leave-the-battlefield drawback (mtg-394 / mtg-400).
#
# Animate Dead ({1}{B} Enchant Creature in a graveyard) reanimates the enchanted
# creature, and:
#   SVar:DBDelay:DB$ DelayedTrigger | Mode$ ChangesZone | ValidCard$ Card.Self
#     | Origin$ Battlefield | Execute$ TrigSacrifice
#   SVar:TrigSacrifice:DB$ SacrificeAll | Defined$ DelayTriggerRememberedLKI
# "When Animate Dead leaves the battlefield, that creature's controller
#  sacrifices it."
#
# Scenario: P0 casts Animate Dead on Sengir Vampire (in P0's graveyard),
# reanimating it, THEN casts Disenchant destroying its own Animate Dead. The
# delayed leave-trigger must fire and sacrifice Sengir Vampire.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Animate Dead: sacrifice reanimated creature when the Aura leaves E2E ==="
echo

cd "$WORKSPACE_ROOT"

PUZZLE="$WORKSPACE_ROOT/test_puzzles/animate_dead_sacrifice_on_leave.pzl"

LOG=/tmp/animate_dead_sac.txt
# P0: cast Animate Dead (menu) -> target Sengir Vampire ([0]); then cast
# Disenchant (menu) -> target the Animate Dead ([0]); then pass out.
printf 'cast Animate Dead\n0\ncast Disenchant\n0\npass\npass\n' \
    | "$MTG_BIN" tui --start-state "$PUZZLE" --p1 tui --p2 zero \
        --seed 42 --verbosity 3 > "$LOG" 2>&1 || true

if grep -qE "Animate Dead reanimates Sengir Vampire" "$LOG"; then
    echo -e "${GREEN}✓ Animate Dead reanimated Sengir Vampire${NC}"
else
    echo -e "${RED}✗ Animate Dead did not reanimate Sengir Vampire${NC}"
    grep -iE "animate|reanimat|sengir" "$LOG" || echo "(none)"
    exit 1
fi

if grep -qE "Disenchant .* destroys Animate Dead|Animate Dead \([0-9]+\) goes to graveyard" "$LOG"; then
    echo -e "${GREEN}✓ Animate Dead was destroyed (left the battlefield)${NC}"
else
    echo -e "${RED}✗ Animate Dead was not destroyed${NC}"
    grep -iE "disenchant|animate dead.*graveyard|destroy" "$LOG" || echo "(none)"
    exit 1
fi

# The drawback: the reanimated Sengir Vampire must be sacrificed.
if grep -qE "Sengir Vampire \([0-9]+\) is sacrificed \(Animate Dead left\)" "$LOG"; then
    echo -e "${GREEN}✓ Reanimated Sengir Vampire was sacrificed when Animate Dead left${NC}"
else
    echo -e "${RED}✗ Reanimated creature was NOT sacrificed when Animate Dead left${NC}"
    grep -iE "sengir|sacrific" "$LOG" || echo "(none)"
    exit 1
fi

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
exit 0
