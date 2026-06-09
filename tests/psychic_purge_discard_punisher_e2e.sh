#!/usr/bin/env bash
# E2E test: Psychic Purge — primary 1-damage mode AND the self-discard punisher.
#
# Regression test for mtg-534 (Card Compatibility: Psychic Purge) / engine gap
# mtg-648 (TriggerEvent::Discarded — opponent-forced-discard punishers were
# silently dropped at load).
#
# Psychic Purge's script:
#   A:SP$ DealDamage | ValidTgts$ Any | NumDmg$ 1
#   T:Mode$ Discarded | ValidCard$ Card.Self | ValidCause$ SpellAbility.OppCtrl | Execute$ TrigLoseLife
#   SVar:TrigLoseLife:DB$ LoseLife | Defined$ TriggeredCauseController | LifeAmount$ 5
#
# Before the fix the loader routed every `Mode$ Discarded` trigger to
# TriggerEvent::CardDiscarded (the "you discarded a card" battlefield watcher),
# which only fires from a permanent on the battlefield watching its controller's
# discards — so Psychic Purge's self-discard punisher (carried on the card BEING
# discarded) never fired. The opponent-punisher did nothing.
#
# After the fix:
#   - `ValidCard$ Card.Self` Discarded triggers parse to TriggerEvent::Discarded
#     and fire from the discarded card's LKI as it moves Hand->Graveyard.
#   - `ValidCause$ SpellAbility.OppCtrl` gates it to OPPONENT-forced discards.
#   - `Defined$ TriggeredCauseController` resolves the 5-life loss to the
#     controller of the spell/ability that forced the discard.
#
# Three checks:
#   (1) PRIMARY mode: SP$ DealDamage 1 to any target still works.
#   (2) PUNISHER fires when an OPPONENT (Mind Rot's controller) forces the
#       discard: that opponent loses 5 life.
#   (3) PUNISHER does NOT fire on a SELF-discard (cleanup-step hand-size discard,
#       CR 514.1 — no spell/ability cause): neither player loses life.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Psychic Purge: 1-damage primary mode + self-discard punisher E2E ==="
echo

cd "$WORKSPACE_ROOT"

# -------------------------------------------------------------------------
# (1) PRIMARY mode: Psychic Purge deals 1 damage to any target.
# -------------------------------------------------------------------------
PRIMARY_PZL=/tmp/psychic_purge_primary_$$.pzl
cat > "$PRIMARY_PZL" <<'P'
[metadata]
Name:Psychic Purge deals 1
[state]
turn=2
activeplayer=p0
activephase=MAIN1
p0life=20
p0hand=Psychic Purge
p0library=Island; Island; Island; Island; Island
p0battlefield=Island; Island; Island
p1life=20
p1library=Plains; Plains; Plains; Plains; Plains
P

LOG_PRIMARY=/tmp/psychic_purge_primary_e2e.txt
if run_mtg_with_timeout 30 tui \
    --start-state "$PRIMARY_PZL" \
    --p1=fixed --p2=zero \
    --p1-fixed-inputs="cast Psychic Purge;pass" \
    --stop-on-choice=6 --json --seed 42 --verbosity 3 \
    > "$LOG_PRIMARY" 2>&1; then
    echo -e "${GREEN}✓ Primary-mode game completed${NC}"
else
    EXIT_STATUS=$?
    echo -e "${RED}✗ Primary-mode game failed (exit $EXIT_STATUS)${NC}"
    head -80 "$LOG_PRIMARY"
    exit 1
fi

if grep -qE "Psychic Purge \([0-9]+\) deals 1 damage to Player 2" "$LOG_PRIMARY"; then
    echo -e "${GREEN}✓ (1) Psychic Purge dealt 1 damage to a player${NC}"
else
    echo -e "${RED}✗ (1) Psychic Purge did NOT deal 1 damage${NC}"
    grep -E "deals .* damage|Psychic Purge" "$LOG_PRIMARY" || echo "(no damage line)"
    exit 1
fi

# -------------------------------------------------------------------------
# (2) PUNISHER fires when an opponent forces the discard.
# -------------------------------------------------------------------------
PUZZLE_OPP="$WORKSPACE_ROOT/test_puzzles/psychic_purge_discarded_by_opponent.pzl"
LOG_OPP=/tmp/psychic_purge_opp_discard_e2e.txt
if run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE_OPP" \
    --p1=zero --p2=fixed \
    --p2-fixed-inputs="cast Mind Rot;target Player 1;pass;pass" \
    --stop-on-choice=10 --json --seed 42 --verbosity 3 \
    > "$LOG_OPP" 2>&1; then
    echo -e "${GREEN}✓ Opponent-discard game completed${NC}"
else
    EXIT_STATUS=$?
    echo -e "${RED}✗ Opponent-discard game failed (exit $EXIT_STATUS)${NC}"
    head -80 "$LOG_OPP"
    exit 1
fi

# Psychic Purge must actually be discarded (precondition for the trigger).
if ! grep -qE "discards Psychic Purge" "$LOG_OPP"; then
    echo -e "${RED}✗ (2) precondition failed: Psychic Purge was not discarded${NC}"
    grep -iE "discard" "$LOG_OPP" | head
    exit 1
fi

# The trigger fires and Mind Rot's controller (Player 2) loses 5 life (20 -> 15).
if grep -qE "Player 2 loses 5 life" "$LOG_OPP"; then
    echo -e "${GREEN}✓ (2) Opponent (Mind Rot's controller) lost 5 life from the discard punisher${NC}"
else
    echo -e "${RED}✗ (2) The Discarded punisher did NOT fire (no 5-life loss for the cause's controller)${NC}"
    grep -iE "loses|trigger|discard" "$LOG_OPP" | head
    exit 1
fi

# The punisher must NOT hit the discarding player (Player 1) themselves.
if grep -qE "Player 1 loses 5 life" "$LOG_OPP"; then
    echo -e "${RED}✗ (2) Punisher hit the DISCARDING player instead of the cause's controller${NC}"
    exit 1
fi

# -------------------------------------------------------------------------
# (3) PUNISHER does NOT fire on a self-discard (cleanup-step, no cause).
# -------------------------------------------------------------------------
PUZZLE_SELF="$WORKSPACE_ROOT/test_puzzles/psychic_purge_self_discard_no_punish.pzl"
LOG_SELF=/tmp/psychic_purge_self_discard_e2e.txt
if run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE_SELF" \
    --p1=fixed --p2=zero \
    --p1-fixed-inputs="pass;pass;pass;pass;pass;pass;pass;pass" \
    --stop-on-choice=14 --json --seed 42 --verbosity 3 \
    > "$LOG_SELF" 2>&1; then
    echo -e "${GREEN}✓ Self-discard game completed${NC}"
else
    EXIT_STATUS=$?
    echo -e "${RED}✗ Self-discard game failed (exit $EXIT_STATUS)${NC}"
    head -80 "$LOG_SELF"
    exit 1
fi

# Precondition: Psychic Purge was discarded at cleanup (a self-discard).
if ! grep -qE "discards Psychic Purge" "$LOG_SELF"; then
    echo -e "${RED}✗ (3) precondition failed: Psychic Purge was not discarded at cleanup${NC}"
    grep -iE "discard" "$LOG_SELF" | head
    exit 1
fi

# No player may lose 5 life — a self-discard has no opponent cause (CR 701.8).
if grep -qE "loses 5 life" "$LOG_SELF"; then
    echo -e "${RED}✗ (3) Self-discard wrongly fired the opponent-punisher (5-life loss seen)${NC}"
    grep -iE "loses|trigger" "$LOG_SELF" | head
    exit 1
else
    echo -e "${GREEN}✓ (3) Self-discard did NOT punish (no spurious 5-life loss)${NC}"
fi

rm -f "$PRIMARY_PZL"

echo
echo -e "${GREEN}=== Psychic Purge E2E PASSED (primary damage + opponent-punisher + self-discard no-op) ===${NC}"
