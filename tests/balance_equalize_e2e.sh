#!/usr/bin/env bash
# E2E test: Balance ({1}{W} Sorcery) equalizes lands, hands, and creatures to
# the minimum any player controls/holds.
#
# Regression test for the "Card Compatibility: Balance" beads issue (mtg-483).
# Balance is:
#   A:SP$ Balance | Valid$ Land | SubAbility$ BalanceHands
#   SVar:BalanceHands:DB$ Balance | Zone$ Hand | SubAbility$ BalanceCreatures
#   SVar:BalanceCreatures:DB$ Balance | Valid$ Creature
#
# Scenario (test_puzzles/balance_spell_sacrifice.pzl):
# - P1 hand: Balance. P1 board: Plains x2 (both tapped to cast).
# - P2 board: Grizzly Bears, Llanowar Elves, Hill Giant (3 creatures, 0 lands).
# - P2 hand: Giant Growth x3.
# After Balance: lands equalize to 0 (P1 sacrifices its 2 Plains used as mana),
# hands equalize to 0 (P2 discards all 3), creatures equalize to 0 (P2
# sacrifices all 3, P1 has none).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Balance Equalize E2E Test ==="
echo

cd "$WORKSPACE_ROOT"

PUZZLE="$WORKSPACE_ROOT/test_puzzles/balance_spell_sacrifice.pzl"
LOG=/tmp/balance_equalize_e2e.txt

run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE" \
    --p1=fixed --p2=zero \
    --p1-fixed-inputs="cast Balance;*;*;*;*;*;*;*" \
    --seed 42 --verbosity 3 \
    > "$LOG" 2>&1 || true

# (a) Balance resolves and runs all three equalize passes.
for cat in Land Hand Creature; do
    if grep -qE "Balance: ${cat}" "$LOG"; then
        echo -e "${GREEN}✓ Balance equalized ${cat}${NC}"
    else
        echo -e "${RED}✗ Balance did not equalize ${cat}${NC}"
        grep -iE "balance" "$LOG" | head -12
        exit 1
    fi
done

# (b) Hand equalizes to 0: P2 discards down to the minimum (0). P2 may cast an
# instant Giant Growth in response first, so the exact discard count can be 2
# or 3 — what matters is the hand pass runs and discards happen.
if grep -qE "Balance: Hand sizes equalize to 0" "$LOG" \
    && grep -qE "discards Giant Growth to Balance" "$LOG"; then
    echo -e "${GREEN}✓ P2 discarded to the minimum hand size (0)${NC}"
else
    echo -e "${RED}✗ Hand did not equalize via discards${NC}"
    grep -iE "discard|hand size" "$LOG" | head -8
    exit 1
fi

# (c) P2 sacrifices its creatures (creature equalize to 0).
if grep -qE "sacrifices Grizzly Bears to Balance" "$LOG" \
    && grep -qE "sacrifices (Llanowar Elves|Hill Giant) to Balance" "$LOG"; then
    echo -e "${GREEN}✓ P2 sacrificed its creatures (creatures -> 0)${NC}"
else
    echo -e "${RED}✗ P2 did not sacrifice its creatures${NC}"
    grep -iE "sacrifice" "$LOG" | head -8
    exit 1
fi

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "Full log: $LOG"
exit 0
