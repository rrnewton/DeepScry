#!/usr/bin/env bash
# E2E test: Fires of Invention enforces 2-spell-per-turn cap (NumLimitEachTurn$ 2).
#
# Card compat (mtg-901, 2020 World Championship wave 8):
#   S:Mode$ CantBeCast | NumLimitEachTurn$ 2 | Caster$ You | Secondary$ True
#
# Fires of Invention grants free casts for spells with CMC ≤ land count, but
# limits the player to at most 2 spells per turn. This test verifies that once 2
# spells have been cast the 3rd spell's free-cast option is no longer offered.
#
# Test scenario (fires_two_spell_cap.pzl):
#   P1 hand: Lightning Bolt, Ancestral Recall, Ponder (CMC 1 each)
#   P1 battlefield: 5 lands + Fires of Invention (CMC limit = 5, so all 3 qualify)

set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"
ensure_mtg_binary

GREEN='\033[0;32m'; RED='\033[0;31m'; NC='\033[0m'
echo "=== Fires of Invention: 2-Spell-Per-Turn Cap E2E Test ==="
echo

cd "$WORKSPACE_ROOT"
PUZZLE="$WORKSPACE_ROOT/test_puzzles/fires_two_spell_cap.pzl"
LOG=/tmp/fires_two_spell_cap_e2e.txt

# ── Phase 1: Before any spell — all 3 are offered as free casts ──
if run_mtg_with_timeout 30 tui --start-state "$PUZZLE" \
    --p1=zero --p2=zero \
    --stop-on-choice=1 --verbosity 3 --seed 42 > "$LOG" 2>&1; then
    echo -e "${GREEN}✓ Phase 1 game completed${NC}"
else
    echo -e "${RED}✗ Phase 1 game failed${NC}"; head -60 "$LOG"; exit 1
fi

for SPELL in "Lightning Bolt" "Ancestral Recall" "Ponder"; do
    # The free-cast label is "cast <Name> for " (trailing space from empty ManaCost).
    if grep -qE "\[[0-9]+\] cast $SPELL for " "$LOG"; then
        echo -e "${GREEN}✓ '$SPELL' offered as free cast (before first spell)${NC}"
    else
        echo -e "${RED}✗ '$SPELL' NOT offered as free cast before any spells${NC}"
        grep -A 15 "available actions" "$LOG" | head -20
        exit 1
    fi
done

# ── Phase 2: Cast 2 free spells, then verify the 3rd is NOT free after ──
LOG2=/tmp/fires_two_spell_cap_e2e_phase2.txt
# Numeric indices: 2=Lightning Bolt free, pass, 4=Ponder free (after Bolt on stack
# the list is: 1=Ancestral normal, 2=Ancestral free, 3=Ponder normal, 4=Ponder free)
if run_mtg_with_timeout 30 tui --start-state "$PUZZLE" \
    --p1=fixed --p2=zero \
    --p1-fixed-inputs="2;pass;4;pass" \
    --stop-on-choice=10 --verbosity 3 --seed 42 > "$LOG2" 2>&1; then
    echo -e "${GREEN}✓ Phase 2 game completed${NC}"
else
    EXIT_STATUS=$?
    echo -e "${RED}✗ Phase 2 game failed (exit $EXIT_STATUS)${NC}"
    head -80 "$LOG2"; exit 1
fi

# Verify ≥2 free-cast spells happened (exclude WARN replay lines which repeat entries).
ALT_CAST_COUNT=$(grep -v "^\[WARN\|^\[INFO" "$LOG2" | grep -cE "Player 1 casts .* \(alternative cost\)" || true)
if [ "$ALT_CAST_COUNT" -ge 2 ]; then
    echo -e "${GREEN}✓ $ALT_CAST_COUNT free-cast (alternative cost) spells cast${NC}"
else
    echo -e "${RED}✗ Expected ≥2 free-cast spells, found $ALT_CAST_COUNT${NC}"
    grep -E "casts" "$LOG2" | head -10
    exit 1
fi

# After both spells resolve, the next available-actions block (Your_Main1 with
# an empty stack) must NOT contain any "for" (free-cast) option.
# We extract lines from after the 2nd spell resolves.
# The marker is the SECOND "alternative cost" cast. We then look at what
# follows the next "available actions" block for free-cast offers.
AFTER_TWO_CASTS=$(awk '
    /Player 1 casts .* \(alternative cost\)/ { seen++ }
    seen >= 2 && /available actions/ { capturing=1; block=0; next }
    seen >= 2 && capturing && /^\[Your_/ { capturing=0 }
    capturing { print }
' "$LOG2")

if echo "$AFTER_TWO_CASTS" | grep -qE "cast .* for "; then
    echo -e "${RED}✗ Free-cast 'for' option appeared AFTER 2nd Fires spell resolved — cap not enforced${NC}"
    echo "Actions after 2nd cast:"
    echo "$AFTER_TWO_CASTS" | head -10
    exit 1
else
    echo -e "${GREEN}✓ No free-cast options after 2 spells — Fires 2-spell cap enforced${NC}"
fi

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "Full log (phase 1): $LOG"
echo "Full log (phase 2): $LOG2"
exit 0
