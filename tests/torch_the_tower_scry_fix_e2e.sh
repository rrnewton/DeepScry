#!/usr/bin/env bash
# E2E test: Torch the Tower — scry fires when bargained, NOT when unbargained.
#
# Regression for mtg-881 wave4: before the fix, two inline Scry handlers in
# priority.rs pattern-matched `Effect::Scry { player, count, .. }` (ignoring
# `only_if_bargained`), so the "Condition$ Bargain" Scry 1 rider on Torch the
# Tower fired even when Bargain was not paid. After the fix both handlers check
# `only_if_bargained` and skip the scry when the condition is not met.
#
# This test covers two scenarios:
#   (a) Unbargained cast: Torch deals 2 damage, NO scry fires, creature is
#       exiled (exile-if-dies replacement effect confirms base damage path).
#   (b) Bargained cast: Torch deals 3 damage, scry 1 fires (Condition met),
#       creature is exiled.
#
# Reproducer (a):
#   ./target/release/mtg tui \
#     --start-state test_puzzles/torch_the_tower_base_damage.pzl \
#     --p1=fixed --p2=zero \
#     --p1-fixed-inputs="cast Torch the Tower;*" \
#     --stop-on-choice=20 --seed 42 --verbosity 3
#
# Reproducer (b):
#   ./target/release/mtg tui \
#     --start-state test_puzzles/torch_the_tower_bargained.pzl \
#     --p1=fixed --p2=zero \
#     --p1-fixed-inputs="cast Torch the Tower;*" \
#     --stop-on-choice=20 --seed 42 --verbosity 3

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Torch the Tower: Scry fix (Condition\$ Bargain) E2E Test ==="
echo

cd "$WORKSPACE_ROOT"

fail() {
    echo -e "${RED}✗ $1${NC}"
    echo "--- relevant log lines ---"
    grep -iE "torch|scry|bargain|exile|damage|dies" "${2:-}" | head -30
    exit 1
}

# -----------------------------------------------------------------------
# Scenario (a): UNBARGAINED — deals 2 damage, NO scry, creature exiled
# -----------------------------------------------------------------------
echo "--- Scenario (a): unbargained cast ---"
LOG_A=/tmp/torch_unbargained_e2e.txt

if run_mtg_with_timeout 40 tui \
    --start-state "$WORKSPACE_ROOT/test_puzzles/torch_the_tower_base_damage.pzl" \
    --p1=fixed --p2=zero \
    --p1-fixed-inputs="cast Torch the Tower;*" \
    --p2-fixed-inputs="" \
    --stop-on-choice=20 --seed 42 --verbosity 3 \
    > "$LOG_A" 2>&1; then
    echo -e "${GREEN}✓ Game completed${NC}"
else
    EXIT_STATUS=$?
    echo -e "${RED}✗ Game failed (exit $EXIT_STATUS)${NC}"
    head -60 "$LOG_A"
    exit 1
fi

# Torch must deal exactly 2 damage (unbargained X=2, not 0)
grep -qE "takes 2 damage" "$LOG_A" \
    || fail "Torch did not deal 2 damage to target (unbargained baseline)" "$LOG_A"
echo -e "${GREEN}✓ Torch dealt 2 damage (unbargained)${NC}"

# Scry must NOT fire when not bargained
grep -qiE "causes.*to scry|scry 1" "$LOG_A" \
    && { echo -e "${RED}✗ Scry fired for unbargained cast — Condition\$ Bargain not respected${NC}"; cat "$LOG_A" | grep -i "scry"; exit 1; }
echo -e "${GREEN}✓ No scry for unbargained cast (Condition\$ Bargain respected)${NC}"

# Creature must be exiled (exile-if-dies replacement effect working)
grep -qE "exiled instead of dying|is exiled" "$LOG_A" \
    || fail "Damaged creature was not exiled (exile-if-dies replacement not working)" "$LOG_A"
echo -e "${GREEN}✓ Damaged creature exiled instead of dying${NC}"

echo

# -----------------------------------------------------------------------
# Scenario (b): BARGAINED — deals 3 damage, scry 1 fires
# -----------------------------------------------------------------------
echo "--- Scenario (b): bargained cast ---"
LOG_B=/tmp/torch_bargained_e2e.txt

if run_mtg_with_timeout 40 tui \
    --start-state "$WORKSPACE_ROOT/test_puzzles/torch_the_tower_bargained.pzl" \
    --p1=fixed --p2=zero \
    --p1-fixed-inputs="cast Torch the Tower;*" \
    --p2-fixed-inputs="" \
    --stop-on-choice=20 --seed 42 --verbosity 3 \
    > "$LOG_B" 2>&1; then
    echo -e "${GREEN}✓ Game completed${NC}"
else
    EXIT_STATUS=$?
    echo -e "${RED}✗ Game failed (exit $EXIT_STATUS)${NC}"
    head -60 "$LOG_B"
    exit 1
fi

# Torch must deal exactly 3 damage (bargained X=3)
grep -qE "takes 3 damage" "$LOG_B" \
    || fail "Torch did not deal 3 damage to target (bargained)" "$LOG_B"
echo -e "${GREEN}✓ Torch dealt 3 damage (bargained)${NC}"

# Scry MUST fire when bargained
grep -qE "causes.*to scry|Torch the Tower.*scry" "$LOG_B" \
    || fail "Scry did not fire for bargained cast — Condition\$ Bargain broken" "$LOG_B"
echo -e "${GREEN}✓ Scry fired for bargained cast${NC}"

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "Unbargained log: $LOG_A"
echo "Bargained log:   $LOG_B"
exit 0
