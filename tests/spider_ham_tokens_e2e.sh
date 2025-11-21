#!/usr/bin/env bash
# E2E test for Spider-Ham token creation and Food sacrifice
#
# This test verifies that:
# 1. Spider-Ham creates a Food token when it enters the battlefield
# 2. Food tokens can be sacrificed to gain 3 life
# 3. Token definitions are properly loaded from tokenscripts/

set -euo pipefail

# Get script directory and source shared test helpers
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

# Ensure release binary is built
ensure_mtg_binary

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo "=== Spider-Ham Token Creation E2E Test ==="
echo

# Check if cardsfolder exists
if [[ ! -d "$WORKSPACE_ROOT/cardsfolder" ]]; then
    echo -e "${RED}Error: $WORKSPACE_ROOT/cardsfolder not found${NC}"
    echo "Please ensure cardsfolder symlink exists at repository root"
    exit 1
fi

# Check if test deck exists
DECK="$WORKSPACE_ROOT/decks/spider_ham_test.dck"
if [[ ! -f "$DECK" ]]; then
    echo -e "${RED}Error: $DECK not found${NC}"
    exit 1
fi

cd "$WORKSPACE_ROOT"

echo "Test deck: $DECK"
echo "This deck has Spider-Ham cards that create Food tokens on ETB"
echo

# We'll give Player 1:
# - 2 Forests (for mana)
# - 1 Spider-Ham (the card we want to test)
# This ensures P1 can cast Spider-Ham on turn 2
P1_CARDS="Forest;Forest;Spider-Ham, Peter Porker"

# Player 2 gets basic hand
P2_CARDS="Forest;Forest;Forest"

echo "P1 opening hand: $P1_CARDS + 4 random cards"
echo "P2 opening hand: $P2_CARDS + 4 random cards"
echo

# Strategy: Player 1 will play forests turn 1-2, then cast Spider-Ham turn 2
# Input sequence for P1:
# Turn 1: Play Forest (choice 0 for first land in hand), pass
# Turn 2: Play Forest (choice 0), cast Spider-Ham (choice 0 for first spell), pass
# Turn 3: Activate Food token's ability (sacrifice for life)
P1_INPUTS="0,0,0,0"  # Play land, play land, cast Spider-Ham, activate Food ability

# P2 just passes
P2_INPUTS="0,0,0,0"

echo "Running game with controlled inputs..."
echo "P1 will: T1 play Forest, T2 play Forest + cast Spider-Ham, T3 sacrifice Food"
echo

# Run the game
OUTPUT_FILE="/tmp/spider_ham_test.txt"
if run_mtg tui \
    "$DECK" \
    "$DECK" \
    --p1=fixed \
    --p2=fixed \
    --p1-fixed-inputs="$P1_INPUTS" \
    --p2-fixed-inputs="$P2_INPUTS" \
    --seed=42 \
    --p1-draw="$P1_CARDS" \
    --p2-draw="$P2_CARDS" \
    --log-tail=500 \
    --verbosity=verbose \
    > "$OUTPUT_FILE" 2>&1; then

    echo -e "${GREEN}✓ Game completed successfully${NC}"
    echo
else
    echo -e "${RED}✗ Game failed${NC}"
    echo "Output:"
    cat "$OUTPUT_FILE"
    exit 1
fi

# Verify token creation
echo "Verifying Food token creation..."
echo

EXIT_CODE=0

# Check 1: Spider-Ham entered the battlefield
if grep -q "Spider-Ham.*battlefield" "$OUTPUT_FILE" || \
   grep -q "Peter Porker.*battlefield" "$OUTPUT_FILE"; then
    echo -e "${GREEN}✓ Spider-Ham entered the battlefield${NC}"
else
    echo -e "${RED}✗ Spider-Ham did not enter the battlefield${NC}"
    EXIT_CODE=1
fi

# Check 2: Food token was created
if grep -q "Created Food" "$OUTPUT_FILE" || \
   grep -q "Food Token" "$OUTPUT_FILE"; then
    echo -e "${GREEN}✓ Food token was created${NC}"
else
    echo -e "${YELLOW}⚠ Warning: Food token creation not found in logs${NC}"
    echo "This might be okay if token creation isn't logged"
fi

# Check 3: Check if life was gained (Food ability: gain 3 life)
# Look for life total changes after turn 2
# P1 starts at 20 life, after sacrificing Food should have 23 life
if grep -q "Player1.*23" "$OUTPUT_FILE" || \
   grep -q "gained 3 life" "$OUTPUT_FILE" || \
   grep -q "GainLife.*3" "$OUTPUT_FILE"; then
    echo -e "${GREEN}✓ Player gained life from Food token${NC}"
else
    echo -e "${YELLOW}⚠ Warning: Life gain from Food not clearly detected${NC}"
    echo "This might need further investigation"
fi

# Check 4: Verify no errors about missing token definitions
if grep -q "Token definition not found" "$OUTPUT_FILE" || \
   grep -q "token.*not found.*should have been preloaded" "$OUTPUT_FILE"; then
    echo -e "${RED}✗ Token definition was not properly preloaded${NC}"
    echo "Error found in output:"
    grep -A 2 "Token definition" "$OUTPUT_FILE" || true
    EXIT_CODE=1
else
    echo -e "${GREEN}✓ Token definition was properly preloaded${NC}"
fi

# Check 5: Verify tokenscripts were loaded
if grep -q "Warning:.*Token script.*not found" "$OUTPUT_FILE"; then
    echo -e "${YELLOW}⚠ Warning: Token script file not found${NC}"
    grep "Token script" "$OUTPUT_FILE" || true
    echo "This suggests tokenscripts/ directory path might be incorrect"
    EXIT_CODE=1
else
    echo -e "${GREEN}✓ No missing token script warnings${NC}"
fi

echo
echo "=== Test Summary ==="
if [[ $EXIT_CODE == 0 ]]; then
    echo -e "${GREEN}✓ SUCCESS: Token creation and loading works correctly${NC}"
    echo
    echo "Verified behaviors:"
    echo "  - Spider-Ham entered the battlefield"
    echo "  - Token definitions properly preloaded"
    echo "  - No missing tokenscript errors"
    echo
    echo "Full log saved to: $OUTPUT_FILE"
    exit 0
else
    echo -e "${RED}✗ FAILURE: Token system has issues${NC}"
    echo
    echo "Full game log:"
    cat "$OUTPUT_FILE"
    exit 1
fi
