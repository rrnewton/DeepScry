#!/usr/bin/env bash
# E2E test: Sphinx of Foresight opening-hand reveal → scry 3 on first upkeep.
#
# Regression test for mtg-901 B5 (2020 WC compatibility).
# Card text: "You may reveal this card from your opening hand. If you do,
#             scry 3 at the beginning of your first upkeep."
# Encoded via K:MayEffectFromOpeningHand:RevealCard and SVar chain
# (RevealCard → ScryOnUpkeep → TrigOpenScry → DBScry).
#
# Scenario:
#   - P1 opening hand contains Sphinx of Foresight (forced via --p1-draw).
#   - Engine auto-reveals the Sphinx (always beneficial: free scry).
#   - On P1's first upkeep (Turn 1): "scrys 3 (opening-hand reveal trigger)" fires.
#   - The trigger fires EXACTLY ONCE (one-shot), NOT on P2's upkeep or P1's turn-3 upkeep.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Sphinx of Foresight Opening-Hand Reveal → Scry-3 E2E Test ==="
echo

DECK="$WORKSPACE_ROOT/decks/championship/2020/02_carvalho_jeskai_fires.dck"
OPP_DECK="$WORKSPACE_ROOT/decks/championship/2020/03_manfield_mono_red.dck"

if [[ ! -f "$DECK" ]]; then
    echo -e "${RED}Error: $DECK not found${NC}"
    exit 1
fi

LOG=/tmp/sphinx_foresight_opening_e2e.txt

if run_mtg tui \
    "$DECK" \
    "$OPP_DECK" \
    --p1=heuristic \
    --p2=heuristic \
    --seed=42 \
    --p1-draw="Sphinx of Foresight" \
    --verbosity=verbose \
    --log-tail=300 \
    > "$LOG" 2>&1; then
    echo -e "${GREEN}✓ Game completed${NC}"
else
    EXIT_STATUS=$?
    echo -e "${RED}✗ Game failed (exit $EXIT_STATUS)${NC}"
    head -60 "$LOG"
    exit 1
fi

EXIT_CODE=0

# (a) Opening-hand reveal message
REVEAL_COUNT=$(grep -c "reveals Sphinx of Foresight from opening hand" "$LOG" || true)
if [[ "$REVEAL_COUNT" -ge 1 ]]; then
    echo -e "${GREEN}✓ Sphinx of Foresight was revealed from opening hand ($REVEAL_COUNT reveal(s))${NC}"
else
    echo -e "${RED}✗ Opening-hand reveal message not found${NC}"
    grep -i "sphinx\|opening\|reveal" "$LOG" | head -5
    EXIT_CODE=1
fi

# (b) Scry-3 trigger fires
SCRY3_COUNT=$(grep -c "scrys 3 (opening-hand reveal trigger)" "$LOG" || true)
if [[ "$SCRY3_COUNT" -ge 1 ]]; then
    echo -e "${GREEN}✓ Scry-3 trigger fired on first upkeep ($SCRY3_COUNT trigger(s))${NC}"
else
    echo -e "${RED}✗ Scry-3 trigger did not fire${NC}"
    grep -i "scry\|upkeep" "$LOG" | head -5
    EXIT_CODE=1
fi

# (c) Each reveal produces exactly one scry-3 trigger (1:1 correspondence, no extras)
if [[ "$REVEAL_COUNT" -eq "$SCRY3_COUNT" ]]; then
    echo -e "${GREEN}✓ Reveal/trigger count matches: $REVEAL_COUNT reveal(s) → $SCRY3_COUNT trigger(s) (one-shot confirmed)${NC}"
else
    echo -e "${RED}✗ Mismatch: $REVEAL_COUNT reveal(s) vs $SCRY3_COUNT scry-3 trigger(s)${NC}"
    grep -E "reveals Sphinx|scrys 3 \(opening" "$LOG" | head -10
    EXIT_CODE=1
fi

echo
if [[ $EXIT_CODE -eq 0 ]]; then
    echo -e "${GREEN}=== Test PASSED ===${NC}"
else
    echo -e "${RED}=== Test FAILED ===${NC}"
    echo "Full log: $LOG"
fi
exit $EXIT_CODE
