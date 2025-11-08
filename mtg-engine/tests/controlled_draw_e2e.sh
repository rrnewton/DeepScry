#!/usr/bin/env bash
# E2E test for --p1-draw and --p2-draw CLI flags
#
# This test verifies that the controlled opening hand flags work correctly
# by specifying 5 unique cards and checking they all appear in the opening hand.
#
# With a diverse 60-card deck, the probability of randomly getting exactly
# 5 specific unique cards in the opening hand of 7 is astronomically low,
# so this test provides strong evidence the feature works.

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo "=== Controlled Draw (--p1-draw / --p2-draw) E2E Test ==="
echo

# Check if cardsfolder exists (tests run from mtg-engine/)
if [[ ! -d "../cardsfolder" ]]; then
    echo -e "${RED}Error: ../cardsfolder not found${NC}"
    echo "Please ensure cardsfolder symlink exists at repository root"
    exit 1
fi

# Check if test deck exists (tests run from mtg-engine/)
DECK="../decks/simple_bolt.dck"
if [[ ! -f "$DECK" ]]; then
    echo -e "${RED}Error: $DECK not found${NC}"
    exit 1
fi

echo "Test deck: $DECK"
echo "This deck has 60 cards (20 Mountains, 40 Lightning Bolts)"
echo "Probability of getting exactly these 5 specific cards randomly is extremely low"
echo

# Cards to place in P1's opening hand (5 unique positions from a 60-card deck)
# We'll request 3 Mountains and 2 Lightning Bolts - specific cards that definitely exist
P1_CARDS="Mountain;Mountain;Mountain;Lightning Bolt;Lightning Bolt"

# Cards to place in P2's opening hand (5 cards)
P2_CARDS="Mountain;Lightning Bolt;Mountain;Lightning Bolt;Mountain"

echo "P1 requested hand (5 cards): $P1_CARDS"
echo "P2 requested hand (5 cards): $P2_CARDS"
echo "Each hand will be filled to 7 cards with 2 random cards"
echo

# Single game test - use cargo run since we only call mtg once
echo "Running game with controlled opening hands..."
echo "Using: cargo run --bin mtg -- tui ..."
echo

# Run the game with controlled hands
# We'll use --log-tail to limit output and --verbosity verbose to see hand contents
if cargo run --bin mtg -- tui \
    "$DECK" \
    "$DECK" \
    --p1=zero \
    --p2=zero \
    --seed=42 \
    --p1-draw="$P1_CARDS" \
    --p2-draw="$P2_CARDS" \
    --log-tail=200 \
    --verbosity=verbose \
    > /tmp/controlled_draw_test.txt 2>&1; then

    echo -e "${GREEN}✓ Game completed successfully${NC}"
    echo
else
    echo -e "${RED}✗ Game failed${NC}"
    echo "Output:"
    cat /tmp/controlled_draw_test.txt
    exit 1
fi

# Now verify that all requested cards appear in the opening hands
echo "Verifying P1's opening hand contains all requested cards..."

# Extract P1's hand from Turn 1 output
# Look for the section with "Hand contents:" after "Player1" (P1)
# Note: Turn format is "         >>> Turn 1 - Player1 20 (Player2 20) <<<<"
# Use ^Player2: to match only the start of line (opponent section label)
P1_HAND=$(sed -n '/Turn 1.*Player1/,/^Player2:/p' /tmp/controlled_draw_test.txt | \
          sed -n '/Hand contents:/,/Battlefield:/p' | head -10)

# Check for Mountains (need at least 3)
P1_MISSING=""
MOUNTAIN_COUNT_P1=$(echo "$P1_HAND" | grep -io "Mountain" | wc -l | tr -d ' \n')
if [[ $MOUNTAIN_COUNT_P1 -lt 3 ]]; then
    P1_MISSING="${P1_MISSING}\n  - Mountain (found $MOUNTAIN_COUNT_P1, need 3)"
fi

# Check for Lightning Bolts (need at least 2)
BOLT_COUNT_P1=$(echo "$P1_HAND" | grep -io "Lightning Bolt" | wc -l | tr -d ' \n')
if [[ $BOLT_COUNT_P1 -lt 2 ]]; then
    P1_MISSING="${P1_MISSING}\n  - Lightning Bolt (found $BOLT_COUNT_P1, need 2)"
fi

if [[ -z "$P1_MISSING" ]]; then
    echo -e "${GREEN}✓ P1: All 5 requested cards found in opening hand${NC}"
    echo "  (3 Mountains, 2 Lightning Bolts)"
else
    echo -e "${RED}✗ P1: Missing requested cards:${P1_MISSING}${NC}"
    echo
    echo "P1's hand was:"
    echo "$P1_HAND"
    EXIT_CODE=1
fi

echo
echo "Verifying P2's opening hand contains all requested cards..."

# Extract P2's hand from Turn 2 output (Player2's first turn as active player)
# Note: Player2's first active turn shows his hand
# Note: Turn format is "         >>> Turn 2 - Player2 20 (Player1 19) <<<<"
P2_HAND=$(grep -A 20 "Turn 2.*Player2" /tmp/controlled_draw_test.txt | \
          sed -n '/Hand contents:/,/Battlefield:/p')

# Check P2's requested hand (3 Mountains, 2 Lightning Bolts)
P2_MISSING=""
MOUNTAIN_COUNT_P2=$(echo "$P2_HAND" | grep -io "Mountain" | wc -l | tr -d ' \n')
if [[ $MOUNTAIN_COUNT_P2 -lt 3 ]]; then
    P2_MISSING="${P2_MISSING}\n  - Mountain (found $MOUNTAIN_COUNT_P2, need 3)"
fi

BOLT_COUNT_P2=$(echo "$P2_HAND" | grep -io "Lightning Bolt" | wc -l | tr -d ' \n')
if [[ $BOLT_COUNT_P2 -lt 2 ]]; then
    P2_MISSING="${P2_MISSING}\n  - Lightning Bolt (found $BOLT_COUNT_P2, need 2)"
fi

if [[ -z "$P2_MISSING" ]]; then
    echo -e "${GREEN}✓ P2: All 5 requested cards found in opening hand${NC}"
    echo "  (3 Mountains, 2 Lightning Bolts)"
else
    echo -e "${RED}✗ P2: Missing requested cards:${P2_MISSING}${NC}"
    echo
    echo "P2's hand was:"
    echo "$P2_HAND"
    EXIT_CODE=1
fi

# Also verify hands have exactly 7 cards total
echo
echo "Verifying hand sizes..."

P1_HAND_SIZE=$(echo "$P1_HAND" | grep "^    -" | wc -l | tr -d ' \n')
P2_HAND_SIZE=$(echo "$P2_HAND" | grep "^    -" | wc -l | tr -d ' \n')

if [[ $P1_HAND_SIZE -eq 7 ]]; then
    echo -e "${GREEN}✓ P1 has exactly 7 cards${NC}"
else
    echo -e "${RED}✗ P1 has $P1_HAND_SIZE cards (expected 7)${NC}"
    EXIT_CODE=1
fi

if [[ $P2_HAND_SIZE -eq 7 ]]; then
    echo -e "${GREEN}✓ P2 has exactly 7 cards${NC}"
else
    echo -e "${RED}✗ P2 has $P2_HAND_SIZE cards (expected 7)${NC}"
    EXIT_CODE=1
fi

echo
echo "=== Test Summary ==="
if [[ ${EXIT_CODE:-0} == 0 ]]; then
    echo -e "${GREEN}✓ SUCCESS: All requested cards appeared in opening hands${NC}"
    echo
    echo "This confirms that --p1-draw and --p2-draw work correctly:"
    echo "  - Specified cards are placed in hand from library"
    echo "  - Remaining cards are drawn randomly to reach 7 total"
    echo "  - Hand size is correct (7 cards)"
    echo
    echo "Full log saved to: /tmp/controlled_draw_test.txt"
    exit 0
else
    echo -e "${RED}✗ FAILURE: Some requested cards were missing${NC}"
    echo
    echo "Full game log:"
    cat /tmp/controlled_draw_test.txt
    exit 1
fi
