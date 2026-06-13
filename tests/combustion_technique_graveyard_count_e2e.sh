#!/usr/bin/env bash
# E2E test: Combustion Technique — graveyard-count X resolves correctly.
#
# Combustion Technique: SVar:X:Count$ValidGraveyard Lesson.YouOwn/Plus.2
# deals damage equal to 2 + (Lesson cards in your graveyard) to target creature.
# If that creature would die this turn, exile it instead.
#
# Before the fix (was: DealDamageXPaid path, dealt 0 damage):
#   - The spell resolved but dealt 0 damage (X resolved via DealDamageXPaid, which
#     returned x_paid from the mana cost, not the graveyard count).
# After the fix (DealDamageDynamic + ValidGraveyard CountExpression):
#   - X = 2 + count(Lesson cards in graveyard).
#   - With 2 Lessons in graveyard → X=4. A 3/3 Centaur Courser takes 4 damage
#     and is exiled instead of dying (ReplaceDyingDefined$ Targeted).
#
# Reproducer:
#   ./target/release/mtg tui \
#     --start-state test_puzzles/combustion_technique_graveyard_count.pzl \
#     --p1=fixed --p2=zero \
#     --p1-fixed-inputs="cast Combustion Technique;*" \
#     --stop-on-choice=20 --seed 42 --verbosity 3

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Combustion Technique: Graveyard-count X E2E Test ==="
echo

cd "$WORKSPACE_ROOT"

fail() {
    echo -e "${RED}✗ $1${NC}"
    echo "--- relevant log lines ---"
    grep -iE "combustion|damage|exile|dies|centaur|lesson" "${2:-}" | head -30
    exit 1
}

LOG=/tmp/combustion_technique_graveyard_e2e.txt

if run_mtg_with_timeout 40 tui \
    --start-state "$WORKSPACE_ROOT/test_puzzles/combustion_technique_graveyard_count.pzl" \
    --p1=fixed --p2=zero \
    --p1-fixed-inputs="cast Combustion Technique;*" \
    --p2-fixed-inputs="" \
    --stop-on-choice=20 --seed 42 --verbosity 3 \
    > "$LOG" 2>&1; then
    echo -e "${GREEN}✓ Game completed${NC}"
else
    EXIT_STATUS=$?
    echo -e "${RED}✗ Game failed (exit $EXIT_STATUS)${NC}"
    head -60 "$LOG"
    exit 1
fi

# Combustion Technique must deal 4 damage (2 + 2 Lesson cards in graveyard)
# X=0 would indicate the old DealDamageXPaid bug
grep -qE "takes 4 damage" "$LOG" \
    || fail "Combustion Technique did not deal 4 damage (expected 2+2=4 from graveyard count)" "$LOG"
echo -e "${GREEN}✓ Combustion Technique dealt 4 damage (2 + 2 graveyard Lessons)${NC}"

# Ensure it didn't deal 0 (the bug we're regression-testing)
grep -qE "takes 0 damage" "$LOG" \
    && { echo -e "${RED}✗ Combustion Technique dealt 0 damage — DealDamageXPaid bug regression!${NC}"; cat "$LOG" | grep -i damage; exit 1; }
echo -e "${GREEN}✓ No 0-damage bug (DealDamageXPaid path not taken)${NC}"

# Creature should be exiled (ReplaceDyingDefined$ Targeted replacement effect)
grep -qE "exiled instead of dying|is exiled" "$LOG" \
    || fail "Damaged creature was not exiled (ReplaceDyingDefined Targeted not working)" "$LOG"
echo -e "${GREEN}✓ Damaged creature exiled instead of dying (ReplaceDyingDefined Targeted works)${NC}"

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "Log: $LOG"
exit 0
