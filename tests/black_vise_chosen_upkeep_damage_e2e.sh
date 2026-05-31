#!/usr/bin/env bash
# E2E test: Black Vise deals damage to the CHOSEN player at that player's upkeep
# equal to max(0, cards-in-hand - 4).
#
# Regression test for mtg-cuf0e (Card Compatibility: Black Vise).
#
# Black Vise's script:
#   K:ETBReplacement:Other:ChooseP
#   SVar:ChooseP:DB$ ChoosePlayer | Defined$ You | Choices$ Player.Opponent | AILogic$ MostCardsInHand
#   T:Mode$ Phase | Phase$ Upkeep | ValidPlayer$ Player.Chosen | TriggerZones$ Battlefield | Execute$ TrigDamage
#   SVar:TrigDamage:DB$ DealDamage | Defined$ ChosenPlayer | NumDmg$ X
#   SVar:X:Count$ValidHand Card.ChosenCtrl/Minus.4
#
# Before the fix two engine gaps made Black Vise a 0-damage no-op:
#   1. Count$ValidHand <selector>/Minus.N was unparsed (-> Fixed(0)).
#   2. The ETB "choose a player" replacement was unmodeled, so no chosen player
#      was recorded and ValidPlayer$ Player.Chosen could not gate the trigger.
#
# After the fix:
#   - CountExpression::CardsInHand{..} counts the chosen player's hand SIZE and
#     applies /Minus.4 (information-independent: only the public count is read).
#   - The ETB replacement records Card::chosen_player (deterministic public-state
#     pick), and Trigger::chosen_player_turn_only gates firing to the chosen
#     player's upkeep (CR 603 triggered ability, CR 614 as-enters replacement,
#     CR 119 damage).
#
# Scenario (test_puzzles/black_vise_chosen_upkeep_damage.pzl):
# - P1 (Player 2) controls Black Vise; the single opponent P0 (Player 1) was
#   chosen at ETB (resolved by the puzzle loader's ETB ChoosePlayer pass).
# - P0 holds 6 cards, so 6 - 4 = 2 damage at P0's upkeep.
# - Puzzle starts in P1's MAIN1 (turn 1, active=p1); the first upkeep reached is
#   P0's upkeep on turn 2: 2 damage to Player 1 (20 -> 18). The trigger does NOT
#   fire on P1's own upkeep (P1 was not chosen).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Black Vise Chosen-Upkeep Damage E2E ==="
echo

cd "$WORKSPACE_ROOT"

PUZZLE="$WORKSPACE_ROOT/test_puzzles/black_vise_chosen_upkeep_damage.pzl"
LOG=/tmp/black_vise_chosen_upkeep_damage_e2e.txt

if run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE" \
    --p1=zero --p2=zero \
    --stop-on-choice=12 --json --seed 42 --verbosity 3 \
    > "$LOG" 2>&1; then
    echo -e "${GREEN}✓ Game completed${NC}"
else
    EXIT_STATUS=$?
    echo -e "${RED}✗ Game failed (exit $EXIT_STATUS)${NC}"
    head -80 "$LOG"
    exit 1
fi

# NOTE on the ETB "chose" log line: a puzzle places battlefield permanents
# directly (they never enter via set_card_zone, where the ChoosePlayer
# replacement normally logs "<card> - chose <player>"). The puzzle loader's ETB
# ChoosePlayer pass records Card::chosen_player silently. The "chose <player>"
# log line IS exercised by the live-deck native-vs-WASM leg
# (decks/old_school2/black_vise_punisher.dck, seed 3) in `make
# validate-wasm-e2e-step`; here we prove the downstream effect (the chosen
# player's upkeep takes max(0, hand-4) damage and the NON-chosen player does
# not), which can only happen if a chosen player was recorded.

# Required: Black Vise deals 2 damage to the chosen player (hand 6 -> 6-4=2).
if grep -qE "Black Vise deals 2 damage to Player 1" "$LOG"; then
    echo -e "${GREEN}✓ Black Vise dealt 2 damage to Player 1 (hand of 6, minus 4)${NC}"
else
    echo -e "${RED}✗ Black Vise did NOT deal 2 damage to Player 1${NC}"
    grep -E "Black Vise deals|deals .* damage" "$LOG" || echo "(no Black Vise damage line — silent drop?)"
    exit 1
fi

# Required: it does NOT fire on the NON-chosen player's (P1/Player 2's) upkeep.
if grep -qE "Black Vise deals .* damage to Player 2" "$LOG"; then
    echo -e "${RED}✗ Black Vise fired on the non-chosen player's upkeep (ValidPlayer\$ Player.Chosen gate broken)${NC}"
    exit 1
fi

# Required: no silent "deals 0 damage" / fizzle (CR 120.8 — a 0-damage hit must
# be skipped, not logged).
if grep -qE "Black Vise deals 0 damage" "$LOG"; then
    echo -e "${RED}✗ Black Vise logged a 0-damage hit${NC}"
    exit 1
fi

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "Full log: $LOG"
exit 0
