#!/usr/bin/env bash
# E2E test: Valley Floodcaller PumpAll trigger on noncreature spell cast.
#
# Verifies that Valley Floodcaller's SpellCast trigger correctly fires when
# a noncreature spell is cast while valid Otter/Bird/Frog/Rat targets are
# in play (mtg-881 wave4 2025 WC compat verification).
#
# Previous deep-verify runs showed a "fizzled: unresolved target" warning when
# no valid creature-type targets were present. This test confirms the trigger
# fires and applies +1/+1 correctly when Otter targets ARE present.
#
# Test scenario:
#   Valley Floodcaller (2/2 Otter Wizard) + Otter-Penguin (1/1 Otter) on
#   battlefield. P0 casts Lightning Bolt (noncreature instant). The SpellCast
#   trigger pumps both Otters +1/+1 until end of turn.
#
# Reproducer:
#   ./target/release/mtg tui \
#     --start-state test_puzzles/valley_floodcaller_pumpall_on_spell.pzl \
#     --p1=fixed --p2=zero \
#     --p1-fixed-inputs="cast Lightning Bolt;*" \
#     --stop-on-choice=20 --seed 42 --verbosity 3

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Valley Floodcaller: PumpAll trigger on noncreature spell E2E Test ==="
echo

cd "$WORKSPACE_ROOT"

LOG=/tmp/valley_floodcaller_pumpall_e2e.txt

if run_mtg_with_timeout 40 tui \
    --start-state "$WORKSPACE_ROOT/test_puzzles/valley_floodcaller_pumpall_on_spell.pzl" \
    --p1=fixed --p2=zero \
    --p1-fixed-inputs="cast Lightning Bolt;*" \
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

fail() {
    echo -e "${RED}✗ $1${NC}"
    echo "--- relevant log lines ---"
    grep -iE "valley|floodcaller|pumpall|otter|trigger|pump|\+1" "$LOG" | head -30
    exit 1
}

# The SpellCast trigger must fire when Lightning Bolt is cast
grep -qE "Trigger: Valley Floodcaller" "$LOG" \
    || fail "Valley Floodcaller SpellCast trigger did not fire"
echo -e "${GREEN}✓ Valley Floodcaller SpellCast trigger fired${NC}"

# Both Otters must receive the +1/+1 pump
grep -qE "Valley Floodcaller gets \+1/\+1 until end of turn" "$LOG" \
    || fail "Valley Floodcaller itself was not pumped +1/+1"
echo -e "${GREEN}✓ Valley Floodcaller pumped +1/+1${NC}"

grep -qE "Otter-Penguin gets \+1/\+1 until end of turn" "$LOG" \
    || fail "Otter-Penguin was not pumped +1/+1 by Valley Floodcaller trigger"
echo -e "${GREEN}✓ Otter-Penguin pumped +1/+1 by Valley Floodcaller trigger${NC}"

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "Full log: $LOG"
exit 0
