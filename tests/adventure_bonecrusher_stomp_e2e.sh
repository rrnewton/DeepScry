#!/usr/bin/env bash
# E2E test: the Adventure mechanic (CR 715), Bonecrusher Giant / Stomp.
#
# Regression test for mtg-902 B2 (2020 World Championship compat backlog):
# Adventure was previously unimplemented — the loader stopped parsing at the
# `ALTERNATE` separator, so only the creature face was ever seen and the
# Adventure (instant/sorcery) half was never offered.
#
# This test proves the full Adventure flow end-to-end:
#   1. The Adventure half (Stomp) is OFFERED as a cast from hand, distinct from
#      the creature half (Bonecrusher Giant).
#   2. Casting Stomp resolves its spell effect (2 damage to a target).
#   3. On resolution the card is EXILED "on an adventure" (CR 715.3d) instead of
#      going to the graveyard, and the creature half becomes castable from exile
#      for its PRINTED mana cost ({2}{R}, NOT Stomp's {1}{R}).
#   4. Casting Bonecrusher Giant from exile puts the 4/3 creature onto the
#      battlefield under its own name.
#
# Reproducer:
#   ./target/release/mtg tui \
#     --start-state test_puzzles/adventure_bonecrusher_stomp.pzl \
#     --p1=fixed --p2=zero \
#     --p1-fixed-inputs="cast Stomp (Adventure);cast Bonecrusher Giant" \
#     --stop-on-choice=30 --json --seed 42 --verbosity 3

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Adventure: Bonecrusher Giant / Stomp E2E Test ==="
echo

cd "$WORKSPACE_ROOT"

PUZZLE="$WORKSPACE_ROOT/test_puzzles/adventure_bonecrusher_stomp.pzl"
LOG=/tmp/adventure_bonecrusher_stomp_e2e.txt

if run_mtg_with_timeout 40 tui \
    --start-state "$PUZZLE" \
    --p1=fixed --p2=zero \
    --p1-fixed-inputs="cast Stomp (Adventure);cast Bonecrusher Giant" \
    --p2-fixed-inputs="" \
    --stop-on-choice=30 --json --seed 42 --verbosity 3 \
    > "$LOG" 2>&1; then
    echo -e "${GREEN}✓ Game completed${NC}"
else
    EXIT_STATUS=$?
    echo -e "${RED}✗ Game failed (exit $EXIT_STATUS)${NC}"
    head -80 "$LOG"
    exit 1
fi

fail() {
    echo -e "${RED}✗ $1${NC}"
    echo "--- relevant log lines ---"
    grep -iE "stomp|bonecrusher|adventure|exile|cast" "$LOG" | head -30
    exit 1
}

# (a) The Adventure half is offered as a distinct cast.
grep -qE "\[[0-9]+\] cast Stomp \(Adventure\)" "$LOG" \
    || fail "Stomp (Adventure) not offered as a castable spell"
echo -e "${GREEN}✓ Adventure half 'cast Stomp (Adventure)' offered from hand${NC}"

# (b) Stomp is cast and resolves, dealing 2 damage.
grep -qE "Player 1 casts Stomp" "$LOG" || fail "Stomp never cast"
grep -qE "Stomp \([0-9]+\) deals 2 damage to Grizzly Bears" "$LOG" \
    || fail "Stomp did not deal its 2 damage to the target"
echo -e "${GREEN}✓ Stomp resolved and dealt 2 damage${NC}"

# (c) The card is exiled "on an adventure" (NOT graveyarded).
grep -qE "Bonecrusher Giant goes on an adventure \(exiled" "$LOG" \
    || fail "Card was not exiled on an adventure"
echo -e "${GREEN}✓ Bonecrusher Giant exiled on an adventure${NC}"

# (d) The creature half is offered from exile for its PRINTED cost {2}{R}.
grep -qE "Cast from exile: Bonecrusher Giant \(for 2R\)" "$LOG" \
    || fail "Creature half not offered from exile at its printed {2}{R} cost"
echo -e "${GREEN}✓ Creature half castable from exile for printed {2}{R}${NC}"

# (e) Casting from exile puts the 4/3 creature onto the battlefield.
grep -qE "Player 1 casts Bonecrusher Giant from exile" "$LOG" \
    || fail "Creature half never cast from exile"
grep -qE "Bonecrusher Giant \([0-9]+\) enters the battlefield as a 4/3 creature" "$LOG" \
    || fail "Bonecrusher Giant did not enter the battlefield as a 4/3"
echo -e "${GREEN}✓ Bonecrusher Giant entered the battlefield as a 4/3 from exile${NC}"

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "Full log: $LOG"
exit 0
