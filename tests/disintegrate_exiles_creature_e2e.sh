#!/usr/bin/env bash
# E2E test: Disintegrate (mtg-ioesm) — X damage to any target, with the
# "if the creature would die this turn, exile it instead" replacement
# (ReplaceDyingDefined$ ThisTargetedCard.Creature).
#
# Regression for TWO bugs found in the wave-17 X/burn sweep:
#
#  1. DealDamageXPaid display double-resolution: the post-resolution display
#     logger matched only Effect::DealDamage { target: None }, NOT the XPaid
#     variant that NumDmg$ X produces. So an X-burn spell aimed at a CREATURE
#     left the logged target as None, and log_effect_execution's None branch
#     invented a phantom "deals N damage to <opponent player>" line — even
#     though the real damage went to the creature. The opponent's life was
#     never actually changed (display-only), but the game log was wrong and
#     misleading. Fixed in priority.rs by giving DealDamageXPaid the same
#     chosen-target resolution as DealDamage.
#
#  2. ReplaceDyingDefined exile-instead-of-dying was unimplemented: the
#     lethally-damaged creature went to the graveyard instead of exile.
#     Implemented via Effect::ExileIfWouldDieThisTurn + a per-card flag that
#     death_destination_for_card honors (alongside the finality-counter rule).
#
# Scenario: P0 casts Disintegrate X=3 at a 2/2 Grizzly Bears.
#  - Grizzly Bears takes 3 (lethal) damage and is EXILED, not buried.
#  - The opponent PLAYER takes NO damage (no phantom line, life stays 20).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Disintegrate Exiles Creature E2E ==="
echo

cd "$WORKSPACE_ROOT"

PUZZLE="$WORKSPACE_ROOT/test_puzzles/disintegrate_exiles_creature.pzl"
LOG=/tmp/disintegrate_exiles_creature_e2e.txt

if run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE" \
    --p1=fixed --p2=zero \
    --p1-fixed-inputs="cast Disintegrate;3;Grizzly Bears" \
    --stop-on-choice=3 --json --seed 42 --verbosity 3 \
    > "$LOG" 2>&1; then
    echo -e "${GREEN}✓ Game completed${NC}"
else
    EXIT_STATUS=$?
    echo -e "${RED}✗ Game failed (exit $EXIT_STATUS)${NC}"
    head -80 "$LOG"
    exit 1
fi

# 1. The creature takes the 3 (lethal) damage.
if grep -qE "Grizzly Bears \([0-9]+\) takes 3 damage" "$LOG"; then
    echo -e "${GREEN}✓ Grizzly Bears took 3 damage${NC}"
else
    echo -e "${RED}✗ Grizzly Bears did not take 3 damage${NC}"
    grep -E "takes|deals" "$LOG" || echo "(no damage lines)"
    exit 1
fi

# 2. NO phantom "deals N damage to Player" line — this is the double-resolution
#    bug signature. The damage must be logged against the CREATURE only.
if grep -qE "Disintegrate \([0-9]+\) deals 3 damage to Player" "$LOG"; then
    echo -e "${RED}✗ PHANTOM player damage logged (DealDamageXPaid double-resolution regressed)${NC}"
    grep -E "deals 3 damage to" "$LOG"
    exit 1
else
    echo -e "${GREEN}✓ No phantom 'deals 3 damage to Player' line${NC}"
fi

# 3. The creature is EXILED, not sent to the graveyard.
if grep -qE "Grizzly Bears \([0-9]+\) exiled instead of dying" "$LOG"; then
    echo -e "${GREEN}✓ Grizzly Bears exiled instead of dying${NC}"
else
    echo -e "${RED}✗ Grizzly Bears was NOT exiled (ReplaceDyingDefined not applied)${NC}"
    grep -E "graveyard|exile|dies" "$LOG" || echo "(no death lines)"
    exit 1
fi

# 4. The creature must NOT be reported as dying to the graveyard.
if grep -qE "Grizzly Bears \([0-9]+\) dies from lethal damage" "$LOG"; then
    echo -e "${RED}✗ Grizzly Bears went to graveyard (should have been exiled)${NC}"
    exit 1
else
    echo -e "${GREEN}✓ Grizzly Bears did not die to the graveyard${NC}"
fi

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "Full log: $LOG"
exit 0
