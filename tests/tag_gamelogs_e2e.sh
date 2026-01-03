#!/usr/bin/env bash
# E2E test for --tag-gamelogs flag
#
# This test verifies that the --tag-gamelogs flag correctly prefixes
# official game actions with [GAMELOG TurnN STEP] tags.
#
# The tags enable comparing game logs between local and network modes
# to ensure game logic is identical.

set -euo pipefail

# Get script directory and source shared test helpers
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

# Ensure release binary is built
ensure_mtg_binary

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m' # No Color

echo "=== --tag-gamelogs E2E Test ==="
echo
echo "This test verifies that --tag-gamelogs correctly tags game actions."
echo

cd "$WORKSPACE_ROOT"

OUTPUT_FILE="/tmp/tag_gamelogs_test.txt"

# Run a short game with --tag-gamelogs enabled
echo "Running game with --tag-gamelogs..."
if run_mtg_with_timeout 30 tui \
    "$WORKSPACE_ROOT/decks/booster_draft/spiderman/julian_spiderman_draft.dck" \
    --p1 random \
    --p2 random \
    --seed 42 \
    --seed-p1 1 \
    --seed-p2 2 \
    --tag-gamelogs \
    --stop-on-choice 20 \
    --verbosity normal \
    > "$OUTPUT_FILE" 2>&1; then

    echo -e "${GREEN}✓ Game completed successfully${NC}"
    echo

    # Verify GAMELOG tags are present
    GAMELOG_COUNT=$(grep -c '\[GAMELOG' "$OUTPUT_FILE" 2>/dev/null || echo "0")
    if [[ "$GAMELOG_COUNT" -gt 0 ]]; then
        echo -e "${GREEN}✓ Found $GAMELOG_COUNT GAMELOG entries${NC}"
    else
        echo -e "${RED}✗ No GAMELOG entries found${NC}"
        echo
        echo "Output (first 50 lines):"
        head -50 "$OUTPUT_FILE"
        exit 1
    fi

    # Verify turn numbers are present
    if grep -q '\[GAMELOG Turn[0-9]' "$OUTPUT_FILE"; then
        echo -e "${GREEN}✓ Turn numbers present in tags${NC}"
    else
        echo -e "${RED}✗ Turn numbers missing from tags${NC}"
        exit 1
    fi

    # Verify step abbreviations are present (M1, M2, DR, etc.)
    if grep -qE '\[GAMELOG Turn[0-9]+ (M1|M2|DR|UP|UK|BC|DA|DB|CD|EC|ET|CL)\]' "$OUTPUT_FILE"; then
        echo -e "${GREEN}✓ Step abbreviations present in tags${NC}"
    else
        echo -e "${RED}✗ Step abbreviations missing from tags${NC}"
        exit 1
    fi

    # Show some example tags
    echo
    echo "Sample GAMELOG entries:"
    grep '\[GAMELOG' "$OUTPUT_FILE" | head -5

    echo
    echo -e "${GREEN}=== Test PASSED ===${NC}"
    EXIT_CODE=0
else
    EXIT_STATUS=$?
    if [[ $EXIT_STATUS == 124 ]]; then
        echo -e "${RED}✗ Test timed out after 30 seconds${NC}"
    else
        echo -e "${RED}✗ Game failed with exit code $EXIT_STATUS${NC}"
    fi
    echo
    echo "Output (first 50 lines):"
    head -50 "$OUTPUT_FILE"
    EXIT_CODE=1
fi

echo
echo "Full log: $OUTPUT_FILE"
exit $EXIT_CODE
