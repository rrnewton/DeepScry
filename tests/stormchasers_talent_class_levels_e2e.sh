#!/usr/bin/env bash
# E2E test: Stormchaser's Talent Class levels 1/2/3 (mtg-881 wave4 2025 WC compat).
#
# Stormchaser's Talent is a Class enchantment from the 2025 World Championship
# decks with three level-up abilities:
#
#   L1 (ETB): Create a 1/1 blue/red Otter token with prowess.
#   L2 ({3}{U}): ClassLevelGained trigger — return target instant or sorcery
#                from your graveyard to your hand.
#   L3 ({5}{U}): Install ongoing SpellCast trigger — whenever you cast an
#                instant or sorcery, create a 1/1 blue/red Otter token.
#
# This test covers scenarios (a) L2 ClassLevelGained trigger and (b) L3
# SpellCast ongoing trigger.  L1 ETB is handled by the standard Class ETB
# infrastructure tested elsewhere.
#
# Scenario (a): Talent starts at level 1 (pre-levelled via puzzle Counters:LEVEL=1
# + NoETBTrigs to avoid double-counting the ETB Otter). Activating L2 returns
# Lightning Bolt from graveyard to hand.
#
# Scenario (b): Talent starts at level 2 (Counters:LEVEL=2 + NoETBTrigs).
# Activating L3 (index 3 in 1-based action list) installs the SpellCast
# trigger. Casting Lightning Bolt immediately creates an Otter token.
#
# Reproducer (a):
#   ./target/release/mtg tui \
#     --start-state test_puzzles/stormchasers_talent_l2_return_instant.pzl \
#     --p1=fixed --p2=zero \
#     --p1-fixed-inputs="activate Stormchaser's Talent;*" \
#     --stop-on-choice=20 --seed 42 --verbosity 3
#
# Reproducer (b):
#   ./target/release/mtg tui \
#     --start-state test_puzzles/stormchasers_talent_l3_otter_on_instant.pzl \
#     --p1=fixed --p2=zero \
#     --p1-fixed-inputs="3;cast Lightning Bolt;*" \
#     --stop-on-choice=20 --seed 42 --verbosity 3

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Stormchaser's Talent: Class level 2 and level 3 E2E Test ==="
echo

cd "$WORKSPACE_ROOT"

fail() {
    echo -e "${RED}✗ $1${NC}"
    echo "--- relevant log lines ---"
    grep -iE "stormchaser|level|otter|return|graveyard|trigger|instant" "${2:-}" | head -30
    exit 1
}

# -----------------------------------------------------------------------
# Scenario (a): Level 2 ClassLevelGained — return instant from graveyard
# -----------------------------------------------------------------------
echo "--- Scenario (a): Level 2 ClassLevelGained trigger ---"
LOG_A=/tmp/stormchaser_l2_e2e.txt

if run_mtg_with_timeout 40 tui \
    --start-state "$WORKSPACE_ROOT/test_puzzles/stormchasers_talent_l2_return_instant.pzl" \
    --p1=fixed --p2=zero \
    --p1-fixed-inputs="activate Stormchaser's Talent;*" \
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

# Class must advance to level 2
grep -qE "Stormchaser's Talent advances to level 2" "$LOG_A" \
    || fail "Stormchaser's Talent did not advance to level 2" "$LOG_A"
echo -e "${GREEN}✓ Stormchaser's Talent advanced to level 2${NC}"

# ClassLevelGained trigger must return the instant from graveyard
grep -qE "returns Lightning Bolt from graveyard to hand" "$LOG_A" \
    || fail "Lightning Bolt was not returned from graveyard to hand by L2 trigger" "$LOG_A"
echo -e "${GREEN}✓ L2 trigger returned Lightning Bolt from graveyard to hand${NC}"

echo

# -----------------------------------------------------------------------
# Scenario (b): Level 3 SpellCast — Otter token on instant cast
# -----------------------------------------------------------------------
echo "--- Scenario (b): Level 3 SpellCast ongoing trigger ---"
LOG_B=/tmp/stormchaser_l3_e2e.txt

# Available actions: [1] cast Lightning Bolt, [2] activate L2 (fizzles), [3] activate L3
# Index 3 selects the Level 3 activation (1-based in fixed controller).
if run_mtg_with_timeout 40 tui \
    --start-state "$WORKSPACE_ROOT/test_puzzles/stormchasers_talent_l3_otter_on_instant.pzl" \
    --p1=fixed --p2=zero \
    --p1-fixed-inputs="3;cast Lightning Bolt;*" \
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

# Class must advance to level 3
grep -qE "Stormchaser's Talent advances to level 3" "$LOG_B" \
    || fail "Stormchaser's Talent did not advance to level 3" "$LOG_B"
echo -e "${GREEN}✓ Stormchaser's Talent advanced to level 3${NC}"

# The ongoing SpellCast trigger must be installed
grep -qE "gains ability: Whenever you cast an instant or sorcery spell, create a 1/1" "$LOG_B" \
    || fail "L3 SpellCast trigger was not installed as an ongoing ability" "$LOG_B"
echo -e "${GREEN}✓ L3 SpellCast trigger installed${NC}"

# Casting Lightning Bolt must trigger Otter token creation
grep -qE "Trigger: Stormchaser's Talent" "$LOG_B" \
    || fail "SpellCast trigger did not fire on Lightning Bolt cast" "$LOG_B"
echo -e "${GREEN}✓ SpellCast trigger fired on Lightning Bolt cast${NC}"

grep -qE "Created Otter Token under Player 1" "$LOG_B" \
    || fail "No Otter Token created by L3 SpellCast trigger" "$LOG_B"
echo -e "${GREEN}✓ Otter Token created by L3 SpellCast trigger${NC}"

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "L2 log: $LOG_A"
echo "L3 log: $LOG_B"
exit 0
