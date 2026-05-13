#!/usr/bin/env bash
# E2E test: Animate Dead is castable when a creature card exists in a graveyard
#
# Regression test for mtg-efb050 (Card Compatibility: Animate Dead).
#
# Two bugs fixed:
# 1. K:Enchant:Creature.inZoneGraveyard:creature card in a graveyard
#    — the parser at mtg-engine/src/loader/card.rs split on the FIRST colon
#    only, leaving the description ":creature card in a graveyard" appended
#    to the Subtype. Targeting code that split on ".inzone" then saw zone =
#    "graveyard:creature card in a graveyard" and never matched the
#    `Some("graveyard")` arm, so no graveyard targets were ever found.
# 2. The Aura-castability filter at mtg-engine/src/game/game_loop/actions.rs
#    only searched the BATTLEFIELD for valid Aura targets and only matched
#    bare types ("Creature", "Land", ...). Animate Dead's
#    `Creature.inZoneGraveyard` requirement therefore never produced a
#    castable spell offering even though graveyard targeting itself worked.
#
# After both fixes, "cast Animate Dead" appears in the available actions
# when a creature card sits in any graveyard, and casting it correctly
# targets that graveyard card.
#
# Test scenario:
# - P1 hand: Animate Dead; battlefield: 3 Swamps (enough for {1}{B})
# - P1 graveyard: Sengir Vampire
# - Verify (a) "cast Animate Dead" appears as an action, (b) targeting picks
#   the graveyard creature.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Animate Dead: Castable + Targets Graveyard E2E Test ==="
echo

cd "$WORKSPACE_ROOT"

PUZZLE="$WORKSPACE_ROOT/test_puzzles/animate_dead_reanimate.pzl"
LOG=/tmp/animate_dead_castable_e2e.txt

if run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE" \
    --p1=fixed --p2=zero \
    --p1-fixed-inputs="cast Animate Dead" \
    --p2-fixed-inputs="" \
    --stop-on-choice=4 --json --seed 42 --verbosity 3 \
    > "$LOG" 2>&1; then
    echo -e "${GREEN}✓ Game completed${NC}"
else
    EXIT_STATUS=$?
    echo -e "${RED}✗ Game failed (exit $EXIT_STATUS)${NC}"
    head -80 "$LOG"
    exit 1
fi

# (a) Animate Dead is castable
if grep -qE "\[[0-9]+\] cast Animate Dead" "$LOG"; then
    echo -e "${GREEN}✓ 'cast Animate Dead' appears in available actions${NC}"
else
    echo -e "${RED}✗ Animate Dead not offered as a castable spell${NC}"
    grep -E "available actions" "$LOG" | head -5
    exit 1
fi

# (b) Cast and resolve
if grep -qE "Player 1 casts Animate Dead" "$LOG"; then
    echo -e "${GREEN}✓ Animate Dead cast and put on stack${NC}"
else
    echo -e "${RED}✗ Animate Dead never cast${NC}"
    exit 1
fi

# (c) Target chosen is Sengir Vampire (the only creature in any graveyard)
if grep -qE "→ targeting Sengir Vampire" "$LOG"; then
    echo -e "${GREEN}✓ Targeted Sengir Vampire in graveyard${NC}"
else
    echo -e "${RED}✗ Did not target the graveyard creature${NC}"
    grep -iE "targeting" "$LOG" | head -5
    exit 1
fi

# (d) Spell resolved
if grep -qE "Animate Dead \([0-9]+\) resolves" "$LOG"; then
    echo -e "${GREEN}✓ Animate Dead resolved on the stack${NC}"
else
    echo -e "${RED}✗ Animate Dead never resolved${NC}"
    exit 1
fi

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo
echo "Full reanimation behaviour (target returns to battlefield, Aura attaches,"
echo "etbCounter + continuous -1/-0 resolve correctly) is verified separately"
echo "by tests/animate_dead_reanimate_triskelion_e2e.sh."
echo
echo "Full log: $LOG"
exit 0
