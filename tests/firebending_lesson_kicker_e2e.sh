#!/usr/bin/env bash
# E2E test: Firebending Lesson — kicker tracked and CountExpression::Kicked resolves.
#
# Firebending Lesson: K:Kicker:4 / SVar:X:Count$Kicked.5.2
# Deals 5 damage when kicked ({4} additional cost), 2 damage when not kicked.
#
# Before the wave-5 fix (kicker_paid tracking unimplemented):
#   - CountExpression::Kicked always evaluated to unkicked_value=2.
#   - The AI never set kicker_paid even with sufficient mana.
# After the fix:
#   - kicker_paid: bool field added to Card, serialized, undo-logged.
#   - Priority loop Step 2c.5 sets kicker_paid=true when mana allows.
#   - evaluate_count_with_source checks kicker_paid for CountExpression::Kicked.
#   - Firebending Lesson deals 5 damage (kicked) when AI has 5+ mana.
#
# Scenario (a): kicked — P0 has 6 lands (5 available after {R}), kicker {4} paid → deals 5.
#   Craw Wurm (6/4) takes 5 damage and dies (survives 2).
# Scenario (b): unkicked — P0 has only 1 Mountain (just {R}), kicker cannot be paid → deals 2.
#   Grizzly Bears (2/2) takes 2 damage and dies.
#
# Reproducer (a):
#   ./target/release/mtg tui \
#     --start-state test_puzzles/firebending_lesson_kicked.pzl \
#     --p1=fixed --p2=zero \
#     --p1-fixed-inputs="cast Firebending Lesson;*" \
#     --stop-on-choice=20 --seed 42 --verbosity 3
# Reproducer (b):
#   ./target/release/mtg tui \
#     --start-state test_puzzles/firebending_lesson_unkicked.pzl \
#     --p1=fixed --p2=zero \
#     --p1-fixed-inputs="cast Firebending Lesson;*" \
#     --stop-on-choice=20 --seed 42 --verbosity 3

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Firebending Lesson: Kicker Tracking E2E Test ==="
echo

cd "$WORKSPACE_ROOT"

fail() {
    echo -e "${RED}✗ $1${NC}"
    echo "--- relevant log lines ---"
    grep -iE "firebending|kicker|damage|takes|craw|grizzly" "${2:-}" | head -30
    exit 1
}

# -----------------------------------------------------------------------
# Scenario (a): KICKED — deals 5 damage, Craw Wurm dies
# -----------------------------------------------------------------------
echo "--- Scenario (a): kicked cast (sufficient mana) ---"
LOG_A=/tmp/firebending_kicked_e2e.txt

if run_mtg_with_timeout 40 tui \
    --start-state "$WORKSPACE_ROOT/test_puzzles/firebending_lesson_kicked.pzl" \
    --p1=fixed --p2=zero \
    --p1-fixed-inputs="cast Firebending Lesson;*" \
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

# Kicker must have been paid
grep -qE "Kicker paid" "$LOG_A" \
    || fail "Kicker was not paid even with sufficient mana" "$LOG_A"
echo -e "${GREEN}✓ Kicker paid (AI greedy kicker heuristic fired)${NC}"

# Firebending Lesson must deal 5 damage (kicked)
grep -qE "takes 5 damage" "$LOG_A" \
    || fail "Firebending Lesson did not deal 5 damage when kicked" "$LOG_A"
echo -e "${GREEN}✓ Firebending Lesson dealt 5 damage (kicked)${NC}"

# Craw Wurm must die (6/4 survives 2 but dies from 5)
grep -qE "Craw Wurm.*dies|Craw Wurm.*graveyard" "$LOG_A" \
    || fail "Craw Wurm should have died from kicked 5-damage but survived" "$LOG_A"
echo -e "${GREEN}✓ Craw Wurm (6/4) died from 5 damage (would survive unkicked 2)${NC}"

echo

# -----------------------------------------------------------------------
# Scenario (b): UNKICKED — insufficient mana, deals 2 damage
# -----------------------------------------------------------------------
echo "--- Scenario (b): unkicked cast (insufficient mana) ---"
LOG_B=/tmp/firebending_unkicked_e2e.txt

if run_mtg_with_timeout 40 tui \
    --start-state "$WORKSPACE_ROOT/test_puzzles/firebending_lesson_unkicked.pzl" \
    --p1=fixed --p2=zero \
    --p1-fixed-inputs="cast Firebending Lesson;*" \
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

# Kicker must NOT have been paid (not enough mana)
grep -qE "Kicker paid" "$LOG_B" \
    && { echo -e "${RED}✗ Kicker was paid without sufficient mana — heuristic bug${NC}"; cat "$LOG_B" | grep -i kicker; exit 1; }
echo -e "${GREEN}✓ Kicker not paid (insufficient mana, correct)${NC}"

# Firebending Lesson must deal 2 damage (unkicked)
grep -qE "takes 2 damage" "$LOG_B" \
    || fail "Firebending Lesson did not deal 2 damage when not kicked" "$LOG_B"
echo -e "${GREEN}✓ Firebending Lesson dealt 2 damage (unkicked)${NC}"

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "Kicked log:   $LOG_A"
echo "Unkicked log: $LOG_B"
exit 0
