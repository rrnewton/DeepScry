#!/usr/bin/env bash
# E2E test: Cranial Extraction (3B Sorcery - Arcane) exiles all copies of the
# named card from the target player's graveyard, hand, and library.
#
# Compatibility test for the 2005 World Championship deck card (compat-2005-wave6).
#
# Card text: "Choose a nonland card name. Search target player's graveyard,
# hand, and library for all cards with that name and exile them. Then that
# player shuffles."
#
# Script structure:
#   A:SP$ NameCard | ... | SubAbility$ ExileYard
#   SVar:ExileYard:DB$ ChangeZoneAll | Origin$ Graveyard | ChangeType$ Card.NamedCard
#   SVar:ExileHand:DB$ ChangeZone | Origin$ Hand | ChangeType$ Card.NamedCard
#   SVar:ExileLib:DB$ ChangeZone | Origin$ Library | ChangeType$ Card.NamedCard | Shuffle$ True
#
# Scenario (test_puzzles/cranial_extraction_exile_all_zones.pzl):
#   P1 hand: Cranial Extraction. P1 board: 4 Swamps (pays {3}{B}).
#   P2 graveyard: 2x Grizzly Bears. P2 hand: 1x Grizzly Bears.
#     P2 library: Grizzly Bears; Plains; Plains; Plains.
#
# Expected: AI names "Grizzly Bears" (most common in P2's graveyard).
#   All 4 Grizzly Bears (2 from graveyard, 1 from hand, 1 from library)
#   are exiled. P2's library is shuffled.
#
# Regressions:
#   - matches_with_name() must NOT delegate back to matches() directly since
#     matches() short-circuits to false when requires_named_card=true.
#   - ExileHand/ExileLib DB$ ChangeZone sub-abilities must convert to
#     ChangeZoneAll (not MoveSelfBetweenZones) when ChangeType$=Card.NamedCard.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Cranial Extraction Exile All Zones E2E ==="
echo

cd "$WORKSPACE_ROOT"

PUZZLE="$WORKSPACE_ROOT/test_puzzles/cranial_extraction_exile_all_zones.pzl"
LOG=/tmp/cranial_extraction_exile_all_zones_e2e.txt

if run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE" \
    --p1=fixed --p2=zero \
    --p1-fixed-inputs="cast Cranial Extraction;*;*" \
    --stop-on-choice=4 --seed 42 --verbosity 3 \
    > "$LOG" 2>&1; then
    echo -e "${GREEN}✓ Game completed${NC}"
else
    EXIT_STATUS=$?
    echo -e "${RED}✗ Game failed (exit $EXIT_STATUS)${NC}"
    head -80 "$LOG"
    exit 1
fi

# 1. Cranial Extraction resolved.
if grep -qE "Cranial Extraction \([0-9]+\) resolves" "$LOG"; then
    echo -e "${GREEN}✓ Cranial Extraction resolved${NC}"
else
    echo -e "${RED}✗ Cranial Extraction did not resolve${NC}"
    grep -iE "cranial|resolves" "$LOG" | head -8
    exit 1
fi

# 2. AI named "Grizzly Bears" (the most common card in P2's graveyard).
if grep -qE 'names a card: "Grizzly Bears"' "$LOG"; then
    echo -e "${GREEN}✓ AI correctly named Grizzly Bears${NC}"
else
    echo -e "${RED}✗ AI did not name Grizzly Bears${NC}"
    grep -iE "names a card" "$LOG" | head -4
    exit 1
fi

# 3. Graveyard exiled (2 copies).
if grep -qE "Cranial Extraction \([0-9]+\) moves all cards from Graveyard to Exile" "$LOG"; then
    echo -e "${GREEN}✓ Graveyard sweep logged${NC}"
else
    echo -e "${RED}✗ Graveyard sweep not logged${NC}"
    grep -iE "graveyard|exile" "$LOG" | head -8
    exit 1
fi

# 4. Hand exiled (1 copy).
if grep -qE "Cranial Extraction \([0-9]+\) moves all cards from Hand to Exile" "$LOG"; then
    echo -e "${GREEN}✓ Hand sweep logged${NC}"
else
    echo -e "${RED}✗ Hand sweep not logged${NC}"
    grep -iE "hand|exile" "$LOG" | head -8
    exit 1
fi

# 5. Library exiled (1 copy) with shuffle.
if grep -qE "Cranial Extraction \([0-9]+\) moves all cards from Library to Exile" "$LOG"; then
    echo -e "${GREEN}✓ Library sweep logged${NC}"
else
    echo -e "${RED}✗ Library sweep not logged${NC}"
    grep -iE "library|exile" "$LOG" | head -8
    exit 1
fi

# 6. After resolution P2 must have 4 cards in exile (all Grizzly Bears)
#    and 0 in graveyard. We read from the first status dump after P2's turn 2.
#    The log format is "  Exile: N" under each player block.
# Extract P2's exile count from first status block after turn 2 starts.
P2_EXILE=$(awk '/Turn 2 - Player 2/{found=1} found && /Player 2 \(active\)/{p2=1} p2 && /Exile:/{print; exit}' "$LOG" | grep -oE 'Exile: [0-9]+' | grep -oE '[0-9]+' || echo "")
P2_GRAVEYARD=$(awk '/Turn 2 - Player 2/{found=1} found && /Player 2 \(active\)/{p2=1} p2 && /Graveyard:/{print; exit}' "$LOG" | grep -oE 'Graveyard: [0-9]+' | grep -oE '[0-9]+' || echo "")

if [ "$P2_EXILE" = "4" ]; then
    echo -e "${GREEN}✓ P2 has 4 cards in exile (all Grizzly Bears exiled)${NC}"
else
    echo -e "${RED}✗ P2 exile count wrong: expected 4, got '${P2_EXILE}'${NC}"
    grep -E "Exile:|Graveyard:|Hand:|Library:" "$LOG" | head -20
    exit 1
fi

if [ "$P2_GRAVEYARD" = "0" ]; then
    echo -e "${GREEN}✓ P2 graveyard is empty (Grizzly Bears removed)${NC}"
else
    echo -e "${RED}✗ P2 graveyard not empty: expected 0, got '${P2_GRAVEYARD}'${NC}"
    grep -E "Graveyard:" "$LOG" | head -10
    exit 1
fi

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "Full log: $LOG"
exit 0
