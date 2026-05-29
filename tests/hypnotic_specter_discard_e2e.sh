#!/usr/bin/env bash
# E2E test: Hypnotic Specter's combat-damage trigger makes the DAMAGED player
# (the opponent), not its controller, discard a card at random.
#
# Regression test for the Defined$ TriggeredTarget discard bug found while
# bringing the 1994 Old School 'Mono Black Rogerbrand' deck (mtg-560) to
# WORKING. Hypnotic Specter's script is:
#
#   T:Mode$ DamageDone | ValidSource$ Card.Self | ValidTarget$ Opponent | Execute$ TrigDiscard
#   SVar:TrigDiscard:DB$ Discard | Defined$ TriggeredTarget | NumCards$ 1 | Mode$ Random
#
# Before the fix, the effect converter had no arm for `Defined$ TriggeredTarget`,
# so the discard player fell through to `placeholder()`. The trigger-path
# resolver (`resolve_effect_placeholder`) then mapped the placeholder to
# `ctx.controller` — i.e. the ATTACKER — so the player who controlled Hypnotic
# Specter discarded from their own hand instead of the player it just hit.
#
# After the fix the converter emits the `target_opponent` sentinel for
# `TriggeredTarget`/`TriggeredPlayer`, and the trigger-path resolver maps that
# sentinel to `ctx.opponent` (the damaged player in a 2-player game; CR 116.2c).
#
# Test scenario:
# - P0 has an untapped Hypnotic Specter on the battlefield and an empty hand.
# - P1 (opponent) has 3 Plains in hand and no blockers.
# - P0 attacks; Specter connects for 2; P1 (NOT P0) discards exactly one card.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Hypnotic Specter Damage Triggers Opponent Discard E2E ==="
echo

cd "$WORKSPACE_ROOT"

PUZZLE="$WORKSPACE_ROOT/test_puzzles/hypnotic_specter_discard.pzl"
LOG=/tmp/hypnotic_specter_discard_e2e.txt

if run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE" \
    --p1=fixed --p2=zero \
    --p1-fixed-inputs="attack Hypnotic Specter;pass;pass;pass;pass" \
    --p2-fixed-inputs="" \
    --stop-on-choice=8 --seed 42 --verbosity 3 \
    > "$LOG" 2>&1; then
    echo -e "${GREEN}✓ Game completed${NC}"
else
    EXIT_STATUS=$?
    echo -e "${RED}✗ Game failed (exit $EXIT_STATUS)${NC}"
    head -80 "$LOG"
    exit 1
fi

# Required: Specter dealt 2 damage to Player 2.
if grep -qE "Hypnotic Specter \([0-9]+\) deals 2 damage to Player 2" "$LOG"; then
    echo -e "${GREEN}✓ Hypnotic Specter dealt combat damage to opponent${NC}"
else
    echo -e "${RED}✗ Specter did not connect with opponent${NC}"
    grep -E "deals .* damage" "$LOG" || echo "(none)"
    exit 1
fi

# Required: Player 2 (opponent / damaged player) discards.
if grep -qE "^  Player 2 discards " "$LOG"; then
    echo -e "${GREEN}✓ Opponent (damaged player) discarded a card${NC}"
else
    echo -e "${RED}✗ Opponent did not discard${NC}"
    grep -E "discards " "$LOG" || echo "(none)"
    exit 1
fi

# Required: the ATTACKER (Player 1) must NOT discard (the original bug).
if grep -qE "^  Player 1 discards " "$LOG"; then
    echo -e "${RED}✗ Regression: attacker discarded instead of the damaged player${NC}"
    grep -E "^  Player 1 discards " "$LOG"
    exit 1
fi
echo -e "${GREEN}✓ Attacker did not discard${NC}"

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "Full log: $LOG"
exit 0
