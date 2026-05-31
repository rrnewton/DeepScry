#!/usr/bin/env bash
# E2E test: Karma deals damage to each player at their upkeep equal to the
# number of Swamps THAT player controls.
#
# Regression test for mtg-516 (Card Compatibility: Karma).
#
# Karma's script:
#   T:Mode$ Phase | Phase$ Upkeep | ValidPlayer$ Player | Execute$ TrigDamage
#   SVar:TrigDamage:DB$ DealDamage | Defined$ TriggeredPlayer | NumDmg$ X
#   SVar:X:Count$Valid Swamp.ActivePlayerCtrl
#
# Before the fix the phase-trigger handler in loader/card.rs only recognised
# `DB$ DealDamage | Defined$ You | NumDmg$ <fixed>` (controller-only fixed
# damage). Karma's `Defined$ TriggeredPlayer | NumDmg$ X` produced NO effect at
# all, so the trigger fired (log line) but dealt 0 damage — a silent drop that
# made Karma a do-nothing enchantment.
#
# After the fix the loader emits Effect::DealDamageToTriggeredPlayer { count,
# target_self:false }, and check_triggers_for_controller resolves it against the
# ACTIVE player (whose upkeep fired), evaluating the Count$ expression against
# that same player (CR 603 triggered ability).
#
# Scenario (test_puzzles/karma_upkeep_swamp_damage.pzl):
# - P1 (Player 2) controls Karma + 3 Swamps; P0 (Player 1) controls 2 Swamps.
# - Both libraries are Plains so the Swamp counts stay fixed (P0=2, P1=3).
# - Puzzle starts in P0's MAIN1 (turn 1, past upkeep), so the first trigger
#   fires on P1's upkeep (turn 2): 3 damage to Player 2 (20 -> 17).
# - P0's first upkeep (turn 3): 2 damage to Player 1 (20 -> 18).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Karma Upkeep Swamp Damage E2E ==="
echo

cd "$WORKSPACE_ROOT"

PUZZLE="$WORKSPACE_ROOT/test_puzzles/karma_upkeep_swamp_damage.pzl"
LOG=/tmp/karma_upkeep_swamp_damage_e2e.txt

if run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE" \
    --p1=zero --p2=zero \
    --stop-on-choice=10 --json --seed 42 --verbosity 3 \
    > "$LOG" 2>&1; then
    echo -e "${GREEN}✓ Game completed${NC}"
else
    EXIT_STATUS=$?
    echo -e "${RED}✗ Game failed (exit $EXIT_STATUS)${NC}"
    head -80 "$LOG"
    exit 1
fi

# Required: Karma damages the active player by their own Swamp count.
if grep -qE "Karma deals 3 damage to Player 2" "$LOG"; then
    echo -e "${GREEN}✓ Karma dealt 3 damage to Player 2 (controls 3 Swamps)${NC}"
else
    echo -e "${RED}✗ Karma did NOT deal 3 damage to Player 2${NC}"
    grep -E "Karma deals|deals .* damage" "$LOG" || echo "(no Karma damage line — silent drop?)"
    exit 1
fi

if grep -qE "Karma deals 2 damage to Player 1" "$LOG"; then
    echo -e "${GREEN}✓ Karma dealt 2 damage to Player 1 (controls 2 Swamps)${NC}"
else
    echo -e "${RED}✗ Karma did NOT deal 2 damage to Player 1${NC}"
    grep -E "Karma deals|deals .* damage" "$LOG" || echo "(no Karma damage line — silent drop?)"
    exit 1
fi

# Required: the counts differ per player — proves the count is evaluated against
# the ACTIVE player, not a fixed value or the trigger source's controller.
if grep -qE "Karma deals 2 damage to Player 2" "$LOG"; then
    echo -e "${RED}✗ Karma dealt the wrong amount to Player 2 (counted the wrong player's Swamps)${NC}"
    exit 1
fi

# Required: no silent "deals 0 damage" / fizzle.
if grep -qE "Karma deals 0 damage" "$LOG"; then
    echo -e "${RED}✗ Karma logged a 0-damage hit (count evaluated against wrong player)${NC}"
    exit 1
fi

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "Full log: $LOG"
exit 0
