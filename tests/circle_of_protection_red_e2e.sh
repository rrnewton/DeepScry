#!/usr/bin/env bash
# E2E test: Circle of Protection: Red ({1}: prevent the next damage a chosen
# red source would deal to you this turn). Regression test for "Card
# Compatibility: Circle of Protection: Red" (mtg-490) and the general
# source-filtered damage-prevention construct (CR 615.1, 615.6).
#
# Card script:
#   A:AB$ ChooseSource | Cost$ 1 | Choices$ Card.RedSource | SubAbility$ DBEffect
#       | SpellDescription$ The next time a red source of your choice would deal
#         damage to you this turn, prevent that damage.
#
# The construct is modelled as Effect::PreventDamageFromSource, installing a
# DamagePreventionShield on the protected player. The shield is consulted on the
# damage-dealing path (combat attribution AND direct/burn damage) and prevents
# the matching source's next damage event entirely.
#
# Two cases:
#   1. POSITIVE — a red attacker (Ironclaw Orcs, 2/2 R) is chosen; its combat
#      damage is prevented and the CoP controller's life is unchanged (20).
#   2. NEGATIVE — a green attacker (Grizzly Bears, 2/2 G) is NOT a legal choice
#      for CoP:Red, so its damage is NOT prevented (controller drops to 18).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Circle of Protection: Red E2E Test ==="
echo

cd "$WORKSPACE_ROOT"

# ----- Case 1: red combat damage PREVENTED -----
PUZZLE_PREVENT="$WORKSPACE_ROOT/test_puzzles/circle_of_protection_red_prevents.pzl"
LOG_PREVENT=/tmp/circle_of_protection_red_prevents_e2e.txt

# P1 (= Player 2 in logs) controls CoP:Red; activate it choosing the red
# attacker. The single red source is auto-selected, so wildcards cover the
# choice/cost prompts.
# Stop after the first combat (turn 1) but before turn 3's combat, where the
# shield (cleared at end of turn 1, CR 514.2) would correctly NOT prevent a
# fresh attack — keeping this case's assertions focused on the single prevented
# event.
if run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE_PREVENT" \
    --p1=zero --p2=fixed \
    --p2-fixed-inputs="activate Circle of Protection: Red;*;*;*" \
    --stop-on-choice=8 --seed 42 --verbosity 3 \
    > "$LOG_PREVENT" 2>&1; then
    echo -e "${GREEN}✓ Prevention-case game completed${NC}"
else
    EXIT_STATUS=$?
    echo -e "${RED}✗ Prevention-case game failed (exit $EXIT_STATUS)${NC}"
    head -80 "$LOG_PREVENT"
    exit 1
fi

if grep -qE "Circle of Protection: Red activates ability" "$LOG_PREVENT" \
   && grep -qE "targeting Ironclaw Orcs" "$LOG_PREVENT" \
   && grep -qE "Prevented 2 damage to Player 2 from Ironclaw Orcs" "$LOG_PREVENT"; then
    echo -e "${GREEN}✓ Red attacker chosen and its combat damage prevented${NC}"
else
    echo -e "${RED}✗ Prevention did not fire as expected${NC}"
    grep -iE "circle|prevent|orcs|damage|target" "$LOG_PREVENT" | head -15
    exit 1
fi

# The CoP controller (Player 2) must still be at 20 life: assert no "deals N
# damage to Player 2" combat line reduced its life. The prevention line is the
# only damage-related output for Player 2.
if grep -qE "Ironclaw Orcs \([0-9]+\) deals [0-9]+ damage to Player 2" "$LOG_PREVENT"; then
    echo -e "${RED}✗ Red attacker still dealt unprevented damage to the CoP controller${NC}"
    grep -iE "deals .* to Player 2" "$LOG_PREVENT" | head
    exit 1
else
    echo -e "${GREEN}✓ No unprevented red combat damage reached the CoP controller${NC}"
fi

# ----- Case 2: green (non-red) combat damage NOT prevented -----
PUZZLE_NONRED="$WORKSPACE_ROOT/test_puzzles/circle_of_protection_red_nonred.pzl"
LOG_NONRED=/tmp/circle_of_protection_red_nonred_e2e.txt

if run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE_NONRED" \
    --p1=zero --p2=zero \
    --stop-on-choice=12 --seed 42 --verbosity 3 \
    > "$LOG_NONRED" 2>&1; then
    echo -e "${GREEN}✓ Non-red-case game completed${NC}"
else
    EXIT_STATUS=$?
    echo -e "${RED}✗ Non-red-case game failed (exit $EXIT_STATUS)${NC}"
    head -80 "$LOG_NONRED"
    exit 1
fi

if grep -qE "Grizzly Bears \([0-9]+\) deals 2 damage to Player 2 \(life: 18\)" "$LOG_NONRED" \
   && ! grep -qE "Prevented .* from Grizzly Bears" "$LOG_NONRED"; then
    echo -e "${GREEN}✓ Green attacker's damage NOT prevented by CoP:Red (life 20 -> 18)${NC}"
else
    echo -e "${RED}✗ Non-red damage handling incorrect (CoP:Red should not prevent green)${NC}"
    grep -iE "grizzly|prevent|damage to player 2|life" "$LOG_NONRED" | head -15
    exit 1
fi

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "Prevention-case log: $LOG_PREVENT"
echo "Non-red-case log:    $LOG_NONRED"
exit 0
