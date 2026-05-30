#!/usr/bin/env bash
# E2E test: Concordant Crossroads grants haste to ALL creatures.
#
# Regression test for the "Card Compatibility: Concordant Crossroads" beads
# issue (mtg-492). Concordant Crossroads is a {G} World Enchantment with the
# static ability "All creatures have haste."
# (S:Mode$ Continuous | Affected$ Creature | AddKeyword$ Haste).
#
# Haste lets a creature attack the turn it comes under a player's control,
# ignoring summoning sickness (CR 702.10b, CR 302.6).
#
# Scenario (test_puzzles/concordant_crossroads_haste.pzl):
# - P1 board: Concordant Crossroads + 2 Forest. P1 hand: Grizzly Bears.
# - P1 (heuristic) casts Grizzly Bears on Turn 1 and, because of the granted
#   haste, immediately declares it as an attacker that SAME Turn 1.
#
# Without Concordant Crossroads the bears would be summoning-sick and could
# not attack until Turn 3 (P1's next turn) — see the per-card beads issue for
# the control reproducer.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Concordant Crossroads Haste E2E Test ==="
echo

cd "$WORKSPACE_ROOT"

PUZZLE="$WORKSPACE_ROOT/test_puzzles/concordant_crossroads_haste.pzl"
LOG=/tmp/concordant_crossroads_haste_e2e.txt

if run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE" \
    --p1=heuristic --p2=zero \
    --seed 42 --verbosity 3 \
    > "$LOG" 2>&1; then
    echo -e "${GREEN}✓ Game completed${NC}"
else
    EXIT_STATUS=$?
    echo -e "${RED}✗ Game failed (exit $EXIT_STATUS)${NC}"
    head -80 "$LOG"
    exit 1
fi

# (a) Grizzly Bears is cast and enters the battlefield on Turn 1.
TURN1_LINE=$(grep -n "Turn 1 - Player 1's turn" "$LOG" | head -1 | cut -d: -f1)
TURN2_LINE=$(grep -n "Turn 2 - " "$LOG" | head -1 | cut -d: -f1)
if [ -z "$TURN1_LINE" ] || [ -z "$TURN2_LINE" ]; then
    echo -e "${RED}✗ Could not locate Turn 1 / Turn 2 boundaries${NC}"
    grep -nE "Turn [0-9]" "$LOG" | head
    exit 1
fi

ENTER_LINE=$(grep -n "Grizzly Bears.*enters the battlefield" "$LOG" | head -1 | cut -d: -f1)
if [ -z "$ENTER_LINE" ] || [ "$ENTER_LINE" -ge "$TURN2_LINE" ]; then
    echo -e "${RED}✗ Grizzly Bears did not enter on Turn 1${NC}"
    grep -iE "grizzly|Turn [0-9]" "$LOG" | head
    exit 1
fi
echo -e "${GREEN}✓ Grizzly Bears entered on Turn 1${NC}"

# (b) Grizzly Bears attacks on Turn 1 (granted haste defeats summoning sickness).
ATTACK_LINE=$(grep -n "declares Grizzly Bears .* as attacker" "$LOG" | head -1 | cut -d: -f1)
if [ -z "$ATTACK_LINE" ] || [ "$ATTACK_LINE" -ge "$TURN2_LINE" ]; then
    echo -e "${RED}✗ Grizzly Bears did NOT attack on Turn 1 (haste not granted)${NC}"
    grep -iE "grizzly|attacker|Turn [0-9]" "$LOG" | head
    exit 1
fi
echo -e "${GREEN}✓ Grizzly Bears attacked on Turn 1 — haste granted by Concordant Crossroads${NC}"

# (c) The attack dealt combat damage to the defending player.
if grep -qE "Grizzly Bears \([0-9]+\) deals [0-9]+ damage to Player 2" "$LOG"; then
    echo -e "${GREEN}✓ Grizzly Bears dealt combat damage to Player 2${NC}"
else
    echo -e "${RED}✗ Grizzly Bears did not damage the player${NC}"
    grep -iE "grizzly.*deals" "$LOG" | head
    exit 1
fi

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "Full log: $LOG"
exit 0
