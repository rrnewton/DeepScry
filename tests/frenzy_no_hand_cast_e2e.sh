#!/usr/bin/env bash
# E2E test: Experimental Frenzy blocks casting spells from hand (Origin$ Hand).
#
# Card compat (mtg-901, 2020 World Championship wave 8):
#   S:Mode$ CantBeCast | ValidCard$ Card | Caster$ You | Origin$ Hand
#   S:Mode$ CantPlayLand | Player$ You | Origin$ Hand
#
# While Experimental Frenzy is on the battlefield, the controller cannot cast
# spells from their hand OR play lands from their hand. They may still cast
# the top card of their library (via the MayPlay grant) and activate Frenzy's
# {3}{R} self-destruct ability.
#
# Test scenario (frenzy_no_hand_cast.pzl):
#   P1 hand: Lightning Bolt (a castable spell — would normally be available)
#   P1 battlefield: 5 Mountains + Experimental Frenzy
#   P1 library top: Lightning Bolt (castable from library via Frenzy's MayPlay)
#   Expected first-choice available actions:
#     (a) "cast Lightning Bolt" (from hand) is NOT offered.
#     (b) "cast Lightning Bolt from top of library" IS offered.
#     (c) "activate Experimental Frenzy" ({3}{R} ability) IS offered.

set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"
ensure_mtg_binary

GREEN='\033[0;32m'; RED='\033[0;31m'; NC='\033[0m'
echo "=== Experimental Frenzy: Hand Cast Restriction E2E Test ==="
echo

cd "$WORKSPACE_ROOT"
PUZZLE="$WORKSPACE_ROOT/test_puzzles/frenzy_no_hand_cast.pzl"
LOG=/tmp/frenzy_no_hand_cast_e2e.txt

if run_mtg_with_timeout 30 tui --start-state "$PUZZLE" \
    --p1=zero --p2=zero \
    --stop-on-choice=1 --verbosity 3 --seed 42 > "$LOG" 2>&1; then
    echo -e "${GREEN}✓ Game completed${NC}"
else
    echo -e "${RED}✗ Game failed${NC}"; head -60 "$LOG"; exit 1
fi

# Extract only the FIRST available-actions block (before any action is taken).
# We stop reading at the second "Player 1 available actions" or at the first
# non-action log line after the first block.
FIRST_BLOCK=$(awk '
    /Player 1 available actions:/ { found++; if (found==1) { in_block=1; next } else { exit } }
    in_block && /^\[/ { in_block=0; next }
    in_block { print }
' "$LOG")

# (a) Hand Lightning Bolt must NOT appear in the first available-actions block.
# "cast Lightning Bolt" without a zone qualifier = from hand.
# "cast Lightning Bolt from top of library" = from library (OK).
if echo "$FIRST_BLOCK" | grep -qE "\[[0-9]+\] cast Lightning Bolt$"; then
    echo -e "${RED}✗ 'cast Lightning Bolt' (from hand) appeared in initial actions — Frenzy hand restriction NOT enforced${NC}"
    echo "First actions block:"
    echo "$FIRST_BLOCK"
    exit 1
else
    echo -e "${GREEN}✓ Hand 'cast Lightning Bolt' correctly suppressed by Experimental Frenzy${NC}"
fi

# (b) Library cast must appear.
if echo "$FIRST_BLOCK" | grep -qE "\[[0-9]+\] cast Lightning Bolt from top of library$"; then
    echo -e "${GREEN}✓ 'cast Lightning Bolt from top of library' correctly offered${NC}"
else
    echo -e "${RED}✗ Library-cast Lightning Bolt NOT offered — Frenzy MayPlay grant broken${NC}"
    echo "First actions block:"
    echo "$FIRST_BLOCK"
    exit 1
fi

# (c) Frenzy self-destruct activation must appear.
if echo "$FIRST_BLOCK" | grep -qE "\[[0-9]+\] activate Experimental Frenzy$"; then
    echo -e "${GREEN}✓ Experimental Frenzy activate ability offered${NC}"
else
    echo -e "${RED}✗ Experimental Frenzy activate ability NOT offered${NC}"
    echo "First actions block:"
    echo "$FIRST_BLOCK"
    exit 1
fi

# (d) No free-cast "for" option from hand should appear either.
if echo "$FIRST_BLOCK" | grep -qE "\[[0-9]+\] cast Lightning Bolt for "; then
    echo -e "${RED}✗ Free-cast 'Lightning Bolt for' appeared — Fires path not blocked by Frenzy restriction${NC}"
    exit 1
else
    echo -e "${GREEN}✓ No free-cast hand-spell variant offered${NC}"
fi

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "Full log: $LOG"
exit 0
