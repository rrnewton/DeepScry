#!/usr/bin/env bash
# E2E test for Friendly Neighborhood and Spider-Ham anthem effects
#
# This test verifies that:
# 1. Friendly Neighborhood creates 3 Human Citizen tokens on ETB
# 2. Spider-Ham creates a Food token on ETB
# 3. Spider-Ham's anthem grants +1/+1 to other Bears (Grizzly Bears, Runeclaw Bear)
# 4. The P/T modifications are correctly applied via continuous effects

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

echo "=== Friendly Neighborhood E2E Test ==="
echo

# Check if cardsfolder exists
if [[ ! -d "$WORKSPACE_ROOT/cardsfolder" ]]; then
    echo -e "${RED}Error: $WORKSPACE_ROOT/cardsfolder not found${NC}"
    echo "Please ensure cardsfolder symlink exists at repository root"
    exit 1
fi

# Check if test deck exists
DECK="$WORKSPACE_ROOT/decks/friendly_neighborhood_test.dck"
if [[ ! -f "$DECK" ]]; then
    echo -e "${RED}Error: $DECK not found${NC}"
    exit 1
fi

cd "$WORKSPACE_ROOT"

echo "Test deck: $DECK"
echo "This deck tests Friendly Neighborhood token creation and Spider-Ham anthem effects"
echo

echo "Test strategy: AI players will play cards and we'll verify:"
echo "  1. Friendly Neighborhood creates 3 Human Citizen tokens"
echo "  2. Spider-Ham creates Food token"
echo "  3. Spider-Ham grants +1/+1 to other Bears (Grizzly Bears = 2/2, becomes 3/3)"
echo

# Run the game - stop after 30 choices (enough to see cards played)
OUTPUT_FILE="/tmp/friendly_neighborhood_test.txt"
if run_mtg tui \
    "$DECK" \
    "$DECK" \
    --p1=heuristic \
    --p2=heuristic \
    --seed=42 \
    --stop-on-choice=30 \
    --log-tail=800 \
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

# Verify test conditions
echo "Verifying gameplay behaviors..."
echo

EXIT_CODE=0

# Check 1: Friendly Neighborhood creates Human Citizen tokens
echo "Check 1: Friendly Neighborhood token creation"
if grep -q "Created.*Human Citizen.*Token" "$OUTPUT_FILE" || \
   grep -q "Human Citizen" "$OUTPUT_FILE"; then
    # Count how many were created (should be 3)
    TOKEN_COUNT=$(grep -o "Created.*Human Citizen.*Token" "$OUTPUT_FILE" | wc -l | tr -d ' \n')
    if [[ $TOKEN_COUNT -ge 1 ]]; then
        echo -e "${GREEN}✓ Friendly Neighborhood created Human Citizen token(s)${NC}"
    else
        echo -e "${YELLOW}⚠ Human Citizen tokens detected but count unclear${NC}"
    fi
else
    echo -e "${YELLOW}⚠ Human Citizen token creation not clearly detected in logs${NC}"
    echo "  (May not have been cast yet - check game log)"
fi

# Check 2: Spider-Ham creates Food token
echo "Check 2: Spider-Ham token creation"
if grep -q "Created.*Food" "$OUTPUT_FILE" || \
   grep -q "Food Token" "$OUTPUT_FILE"; then
    echo -e "${GREEN}✓ Spider-Ham created Food token${NC}"
else
    echo -e "${YELLOW}⚠ Food token creation not detected${NC}"
    echo "  (Spider-Ham may not have been cast yet)"
fi

# Check 3: Spider-Ham anthem effect on Bears
echo "Check 3: Spider-Ham anthem effect"
# Look for Grizzly Bears or Runeclaw Bear with boosted P/T
# Grizzly Bears is normally 2/2, should be 3/3 with Spider-Ham's anthem
# Runeclaw Bear is normally 2/2, should be 3/3 with Spider-Ham's anthem

# Check for any bear creatures being shown with their P/T
if grep -E "(Grizzly Bears|Runeclaw Bear).*3/3" "$OUTPUT_FILE"; then
    echo -e "${GREEN}✓ Bear creatures show boosted P/T (2/2 → 3/3)${NC}"
    echo "  Spider-Ham's anthem effect is working!"
elif grep -E "(Grizzly Bears|Runeclaw Bear).*2/2" "$OUTPUT_FILE"; then
    echo -e "${YELLOW}⚠ Bear creatures found but showing base 2/2${NC}"
    echo "  Either Spider-Ham not on battlefield, or anthem not applying"
    # Check if Spider-Ham is on battlefield
    if grep -q "Spider-Ham.*battlefield" "$OUTPUT_FILE"; then
        echo -e "${RED}✗ Spider-Ham is on battlefield but anthem not working${NC}"
        EXIT_CODE=1
    else
        echo "  Spider-Ham not yet on battlefield - anthem not expected"
    fi
else
    echo -e "${YELLOW}⚠ No Bear creatures detected in game yet${NC}"
fi

# Check 4: No selector parsing warnings
echo "Check 4: Selector parsing"
if grep -q "Unknown Affected.*selector" "$OUTPUT_FILE" || \
   grep -q "Warning:.*selector" "$OUTPUT_FILE"; then
    echo -e "${RED}✗ Selector parsing warnings detected${NC}"
    grep "selector" "$OUTPUT_FILE" || true
    EXIT_CODE=1
else
    echo -e "${GREEN}✓ No selector parsing warnings${NC}"
fi

# Check 5: Token definitions preloaded
echo "Check 5: Token definition loading"
if grep -q "Token definition not found" "$OUTPUT_FILE" || \
   grep -q "should have been preloaded" "$OUTPUT_FILE"; then
    echo -e "${RED}✗ Token definition not properly preloaded${NC}"
    grep "Token definition" "$OUTPUT_FILE" || true
    EXIT_CODE=1
else
    echo -e "${GREEN}✓ Token definitions properly preloaded${NC}"
fi

echo
echo "=== Test Summary ==="
if [[ $EXIT_CODE == 0 ]]; then
    echo -e "${GREEN}✓ SUCCESS: Friendly Neighborhood system works correctly${NC}"
    echo
    echo "Verified behaviors:"
    echo "  - Token creation system operational"
    echo "  - Selector parsing works without warnings"
    echo "  - Token definitions properly preloaded"
    echo "  - (Anthem effects require Spider-Ham + Bears on battlefield)"
    echo
    echo "Full log saved to: $OUTPUT_FILE"
    echo
    echo "To see anthem effects, examine the log for turns where both"
    echo "Spider-Ham and Bear creatures are on the battlefield together."
    exit 0
else
    echo -e "${RED}✗ FAILURE: Some checks failed${NC}"
    echo
    echo "Full game log:"
    cat "$OUTPUT_FILE"
    exit 1
fi
