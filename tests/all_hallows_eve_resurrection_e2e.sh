#!/usr/bin/env bash
# E2E test: All Hallow's Eve self-exile, upkeep counter-tick, mass resurrection.
#
# Regression test for mtg-464870 (compat) / mtg-393 (bug). Three fixes are
# exercised end-to-end:
#
# 1. ChangeZone Origin$ Stack | Destination$ Exile | RememberChanged$ True —
#    the sorcery resolves *into exile* (not the graveyard) and the chained
#    `DB$ PutCounter | Defined$ Remembered` lands 2 SCREAM counters on it.
#    (Effect::SelfExileFromStack + remembered_cards; pre-existing.)
#
# 2. Exile-resident upkeep trigger — `T:Mode$ Phase | Phase$ Upkeep |
#    TriggerZones$ Exile | IsPresent$ Card.Self+counters_GE1_SCREAM`. Phase
#    triggers now scan the exile zone and honour the intervening-if counter
#    condition (CR 603.4 / 603.6e), so the trigger fires from exile each of the
#    controller's upkeeps and removes one SCREAM counter.
#    (Trigger::trigger_zones + Trigger::present_self_condition.)
#
# 3. Counter-gated self-move + mass return — when the last counter is removed
#    (counters_EQ0_SCREAM) the card moves itself exile→graveyard
#    (Effect::MoveSelfBetweenZones) and each player returns all creature cards
#    from their graveyard to the battlefield (Effect::ChangeZoneAll). Both are
#    gated by Effect::ConditionalSelfCounter.
#
# Starting state (test_puzzles/all_hallows_eve_resurrection.pzl): p0 has AHE in
# hand, Sengir Vampire + Sedge Troll in graveyard; p1 has Grizzly Bears +
# Lightning Bolt in graveyard. After casting and two of p0's upkeeps, all three
# creatures (but NOT Lightning Bolt) return to the battlefield.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== All Hallow's Eve Resurrection E2E ==="
echo

cd "$WORKSPACE_ROOT"

PUZZLE="$WORKSPACE_ROOT/test_puzzles/all_hallows_eve_resurrection.pzl"
LOG=/tmp/all_hallows_eve_resurrection_e2e.txt

if [ ! -f "$PUZZLE" ]; then
    echo -e "${RED}✗ Missing puzzle: $PUZZLE${NC}"
    exit 1
fi

if run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE" \
    --p1=fixed --p2=zero \
    --p1-fixed-inputs="cast All Hallow's Eve" \
    --p2-fixed-inputs="" \
    --stop-on-choice=6 --json --seed 42 --verbosity 3 \
    > "$LOG" 2>&1; then
    echo -e "${GREEN}✓ Game completed${NC}"
else
    EXIT_STATUS=$?
    echo -e "${RED}✗ Game failed (exit $EXIT_STATUS)${NC}"
    head -120 "$LOG"
    exit 1
fi

# (a) Self-exile with 2 SCREAM counters (bugs 1 + 2)
if grep -qE "All Hallow's Eve \([0-9]+\) is exiled \(remembered\)" "$LOG"; then
    echo -e "${GREEN}✓ All Hallow's Eve self-exiled${NC}"
else
    echo -e "${RED}✗ Self-exile did not happen${NC}"
    grep -iE "hallow|exile" "$LOG" | head -10
    exit 1
fi

if grep -qE "puts 2 Scream counter\(s\) on All Hallow's Eve" "$LOG"; then
    echo -e "${GREEN}✓ Two SCREAM counters placed on the exiled card${NC}"
else
    echo -e "${RED}✗ SCREAM counters not placed on All Hallow's Eve${NC}"
    grep -iE "scream|counter" "$LOG" | head -10
    exit 1
fi

# (b) Exile-resident upkeep trigger fires (bug 3a). It must fire at least twice
#     (one tick per SCREAM counter, on the controller's upkeeps only).
TRIGGER_COUNT=$(grep -cE "Trigger: All Hallow's Eve" "$LOG" || true)
if [ "$TRIGGER_COUNT" -ge 2 ]; then
    echo -e "${GREEN}✓ Upkeep trigger fired from exile $TRIGGER_COUNT times${NC}"
else
    echo -e "${RED}✗ Upkeep trigger fired $TRIGGER_COUNT times (expected >= 2)${NC}"
    grep -iE "upkeep|trigger" "$LOG" | head -10
    exit 1
fi

# (c) Mass resurrection: all three creatures return to the battlefield (bug 3b).
for creature in "Sengir Vampire" "Sedge Troll" "Grizzly Bears"; do
    if grep -qE "$creature \([0-9]+\) - [0-9]+/[0-9]+" "$LOG"; then
        echo -e "${GREEN}✓ $creature returned to the battlefield${NC}"
    else
        echo -e "${RED}✗ $creature did NOT return to the battlefield${NC}"
        grep -iE "sengir|sedge|grizzly|battlefield" "$LOG" | head -15
        exit 1
    fi
done

# (d) Lightning Bolt (a non-creature) must NOT be resurrected — ChangeType$
#     Creature filter correctness.
if grep -qE "Lightning Bolt \([0-9]+\) - " "$LOG"; then
    echo -e "${RED}✗ Lightning Bolt was incorrectly returned (ChangeType\$ Creature filter broken)${NC}"
    exit 1
else
    echo -e "${GREEN}✓ Lightning Bolt correctly stayed in the graveyard${NC}"
fi

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo
echo "Full log: $LOG"
exit 0
