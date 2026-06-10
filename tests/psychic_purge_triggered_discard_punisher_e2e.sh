#!/usr/bin/env bash
# E2E test: Psychic Purge's punisher fires on an opponent TRIGGERED-ability
# forced discard (mtg-54n3b, follow-up to mtg-648 / mtg-534).
#
# The sibling test (psychic_purge_discard_punisher_e2e.sh) covers opponent
# discard SPELLS (Mind Rot). This one covers opponent TRIGGERED ABILITIES:
#
#   Hypnotic Specter (cardsfolder/h/hypnotic_specter.txt):
#     T:Mode$ DamageDone | ValidSource$ Card.Self | ValidTarget$ Opponent
#       | Execute$ TrigDiscard
#     SVar:TrigDiscard:DB$ Discard | Defined$ TriggeredTarget | NumCards$ 1 | Mode$ Random
#
# Such trigger-forced discards resolve through GameState::execute_effect
# (Effect::DiscardCards) from check_triggers_inner. mtg-54n3b extracted that
# discard logic into execute_discard_effect(effect, cause) and, at the trigger
# site, routes it with cause = the trigger's CONTROLLER. So when Hypnotic
# Specter's random discard hits Psychic Purge, the Specter's controller — an
# opponent of Psychic Purge's owner — loses 5 life.
#
# Scenario (seed 42): Player 1 (p0) controls a non-summoning-sick Hypnotic
# Specter; Player 2 (p1) holds ONLY Psychic Purge (so the "random" discard is
# deterministic). The Specter attacks unblocked, deals 2 combat damage, its
# trigger makes p1 discard Psychic Purge, and the Specter's controller (Player
# 1) loses 5 life (20 -> 15).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Psychic Purge triggered-ability discard-punisher E2E ==="
echo

cd "$WORKSPACE_ROOT"

LOG=/tmp/psychic_purge_triggered_discard_punish_e2e.txt
if run_mtg_with_timeout 60 tui \
    --start-state "$WORKSPACE_ROOT/test_puzzles/psychic_purge_triggered_discard_punish_e2e.pzl" \
    --p1=heuristic --p2=heuristic \
    --json --seed 42 --verbosity 3 \
    > "$LOG" 2>&1; then
    echo -e "${GREEN}✓ Game completed${NC}"
else
    echo -e "${RED}✗ Game failed${NC}"
    head -80 "$LOG"
    exit 1
fi

# The Specter must connect for combat damage (firing its discard trigger).
if ! grep -qE "Hypnotic Specter .* deals 2 damage to Player 2" "$LOG"; then
    echo -e "${RED}✗ Hypnotic Specter did not deal combat damage to Player 2${NC}"
    grep -E "Hypnotic Specter|deals .* damage" "$LOG" || true
    exit 1
fi
echo -e "${GREEN}✓ Hypnotic Specter dealt combat damage (firing its discard trigger)${NC}"

# The trigger must force the discard of Psychic Purge.
if ! grep -qE "Player 2 discards Psychic Purge" "$LOG"; then
    echo -e "${RED}✗ Hypnotic Specter's trigger did not discard Psychic Purge${NC}"
    grep -E "discards" "$LOG" || true
    exit 1
fi
echo -e "${GREEN}✓ Specter's trigger forced Player 2 to discard Psychic Purge${NC}"

# The CAUSE controller (the Specter's controller, Player 1) must lose 5 life.
if ! grep -qE "Player 1 loses 5 life \(life: 15\)" "$LOG"; then
    echo -e "${RED}✗ Punisher did NOT fire on the triggered-ability discard (caster should lose 5 -> 15)${NC}"
    grep -E "loses .* life|life:" "$LOG" || echo "(no life-loss line — cause not threaded through trigger path?)"
    exit 1
fi
echo -e "${GREEN}✓ Punisher fired: Hypnotic Specter's controller lost 5 life (20 -> 15)${NC}"

echo
echo -e "${GREEN}=== Psychic Purge triggered-ability discard-punisher E2E PASSED ===${NC}"
