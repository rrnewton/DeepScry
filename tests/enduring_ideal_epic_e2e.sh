#!/usr/bin/env bash
# E2E test: Enduring Ideal — Epic cast lock + repeating upkeep search.
#
# Regression for mtg-910 (2005 WC compatibility wave 7 — B9 Epic verification).
#
# Enduring Ideal (5WW) card script:
#   K:Epic
#   A:SP$ ChangeZone | Origin$ Library | Destination$ Battlefield
#     | ChangeType$ Enchantment | ChangeNum$ 1
#     | SpellDescription$ Search your library for an enchantment card,
#       put it onto the battlefield, then shuffle.
#
# Epic (CR 702.88a-c):
#   (a) When you cast a spell with Epic, exile it as it resolves instead of going
#       to the graveyard.
#   (b) For the rest of the game, you can't cast spells (CR 702.88b).
#   (c) At the beginning of each of your upkeeps, copy the Epic spell except for
#       its Epic ability; the copy has the same effects but no Epic itself.
#
# Engine implementation (wave-5, claude/compat-2005-wave3 commit 92e672a7c):
#   - Player::cant_cast_spells bool + GameAction::SetCantCastSpells undo entry.
#   - DelayedTrigger::repeating: bool — re-registers after firing for each upkeep.
#   - resolve_spell_finalize Epic block sets cast lock + registers repeating trigger.
#   - Epic upkeep copy: fire_delayed_trigger re-registers; ExecuteEffect
#     SearchLibrary arm handles the copy.
#
# Setup (test_puzzles/enduring_ideal_epic.pzl):
#   P0 has Enduring Ideal in hand; 6 tapped lands (cannot pay mana immediately).
#   Turn 3 is P0's first untap with fresh mana — heuristic AI casts Enduring Ideal.
#   Library: Ghostly Prison, Honden of Cleansing Fire, Solitary Confinement, Plains, Plains.
#
# Expected sequence:
#   1. Turn 3: Enduring Ideal resolves — first enchantment (Honden of Cleansing Fire)
#      enters the battlefield from library.
#   2. Turn 5: Epic trigger fires (P0's upkeep) — second enchantment (Ghostly Prison)
#      enters the battlefield. Log: "copies Epic effect (search library → Battlefield)".
#   3. Turn 7: Epic trigger fires again (P0's next upkeep) — Solitary Confinement
#      enters the battlefield.
#   4. After casting Enduring Ideal, P0 may ONLY play lands (not cast spells).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Enduring Ideal — Epic E2E ==="
echo

cd "$WORKSPACE_ROOT"

PUZZLE="$WORKSPACE_ROOT/test_puzzles/enduring_ideal_epic.pzl"
LOG=/tmp/enduring_ideal_epic_e2e.txt

if run_mtg_with_timeout 60 tui \
    --start-state "$PUZZLE" \
    --p1=heuristic --p2=heuristic \
    --seed 42 --verbosity 3 \
    > "$LOG" 2>&1; then
    echo -e "${GREEN}✓ Game completed${NC}"
else
    EXIT_STATUS=$?
    echo -e "${RED}✗ Game failed (exit $EXIT_STATUS)${NC}"
    head -80 "$LOG"
    exit 1
fi

# Check 1: Enduring Ideal was cast.
if grep -qE "Player 1 casts Enduring Ideal" "$LOG"; then
    echo -e "${GREEN}✓ Enduring Ideal was cast${NC}"
else
    echo -e "${RED}✗ Enduring Ideal was never cast${NC}"
    grep -E "cast|Enduring" "$LOG" | head -10 || echo "(no cast lines)"
    exit 1
fi

# Check 2: First enchantment search — library search succeeded and put an enchantment
# onto the battlefield ("searches Player 1's library for a Enchantment card and puts
# it into Battlefield").
if grep -qE "searches Player 1.*library for a Enchantment card" "$LOG"; then
    echo -e "${GREEN}✓ Enduring Ideal searched library for enchantment (first cast)${NC}"
else
    echo -e "${RED}✗ Library search for enchantment did not fire on cast${NC}"
    grep -E "search|Enchantment|library|Battlefield" "$LOG" | head -15 || echo "(no search lines)"
    exit 1
fi

# Check 3: Epic repeating upkeep trigger fires at least once.
# Log line: "Player 1 copies Epic effect (search library → Battlefield)".
if grep -qE "copies Epic effect" "$LOG"; then
    echo -e "${GREEN}✓ Epic upkeep trigger fired (repeating copy)${NC}"
else
    echo -e "${RED}✗ Epic upkeep trigger never fired — repeating trigger not registered${NC}"
    grep -E "trigger|upkeep|Epic|Upkeep" "$LOG" | head -20 || echo "(no trigger lines)"
    exit 1
fi

# Check 4: Epic trigger fires at least TWICE (the trigger must re-register after firing).
EPIC_COUNT=$(grep -c "copies Epic effect" "$LOG" || true)
if [ "$EPIC_COUNT" -ge 2 ]; then
    echo -e "${GREEN}✓ Epic upkeep trigger fired ${EPIC_COUNT}x (repeating correctly)${NC}"
else
    echo -e "${RED}✗ Epic trigger fired only ${EPIC_COUNT}x — expected ≥2 (not re-registering)${NC}"
    grep -n "copies Epic effect" "$LOG" | head -10 || echo "(no epic lines)"
    exit 1
fi

# Check 5: Multiple distinct enchantments appeared on P0's battlefield.
# Verify both Ghostly Prison and Honden appear on P0's battlefield at some point.
if grep -qE "Honden of Cleansing Fire" "$LOG"; then
    echo -e "${GREEN}✓ Honden of Cleansing Fire found on battlefield${NC}"
else
    echo -e "${RED}✗ Honden of Cleansing Fire never appeared${NC}"
    exit 1
fi

if grep -qE "Ghostly Prison" "$LOG"; then
    echo -e "${GREEN}✓ Ghostly Prison found on battlefield (from Epic copy)${NC}"
else
    echo -e "${RED}✗ Ghostly Prison never appeared — Epic copy search failed${NC}"
    exit 1
fi

# Check 6: After Epic, Player 1 may only play lands (not cast non-land spells).
# Verify no additional non-land spells were cast by P1 after Enduring Ideal.
# We accept "Player 1 plays" (land) but reject "Player 1 casts <non-Enduring>" lines.
EXTRA_CASTS=$(grep -E "Player 1 casts" "$LOG" | grep -v "Enduring Ideal" | wc -l || true)
if [ "$EXTRA_CASTS" -eq 0 ]; then
    echo -e "${GREEN}✓ Epic cast lock active — P1 cast no spells after Enduring Ideal${NC}"
else
    echo -e "${RED}✗ Epic cast lock broken — P1 cast ${EXTRA_CASTS} spell(s) after Enduring Ideal:${NC}"
    grep -E "Player 1 casts" "$LOG" | grep -v "Enduring Ideal" | head -10
    exit 1
fi

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "Full log: $LOG"
exit 0
