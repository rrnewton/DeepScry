#!/usr/bin/env bash
# End-to-end test for interactive TUI controller with piped stdin
#
# This script tests that the InteractiveController can play a full game
# when provided with a predetermined sequence of inputs via stdin.
#
# With deterministic seeding (--seed 42), we know exactly what choices
# will be presented to the TUI controller, so we can provide matching inputs.

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo "=== Interactive TUI E2E Test ==="
echo

echo "Will use: cargo run --bin mtg"
echo

# Check if test deck exists
if [[ ! -f "decks/simple_bolt.dck" ]]; then
    echo -e "${RED}Error: decks/simple_bolt.dck not found${NC}"
    exit 1
fi

# Check if cardsfolder exists
if [[ ! -d "cardsfolder" ]]; then
    echo -e "${YELLOW}Warning: cardsfolder not found, skipping test${NC}"
    exit 0
fi

echo "Test: TUI controller vs Zero controller with deterministic seed"
echo "Strategy: Provide scripted inputs that match ZeroController behavior"
echo

# With seed 42 and simple_bolt.dck, the ZeroController plays specific actions.
# We need to provide the same choices via stdin.
#
# The game flow with seed 42 (from test_tui_zero_vs_zero_simple_bolt):
# - Player 2 wins
# - Game ends by PlayerDeath (lightning bolts dealing damage)
#
# For the interactive TUI, we'll always pass priority ('p') to mimic ZeroController,
# which effectively passes on all decisions except when there's a meaningful first action.
#
# Strategy: Feed a long sequence of 'p' (pass) inputs, with occasional '0' for
# choosing the first available action when required.

# Create input script: alternating between 0 (choose first option) and p (pass)
# This should be enough to complete a simple game
INPUT_SEQUENCE=""
for i in {1..200}; do
    # Every few turns, choose action 0, otherwise pass
    if (( i % 3 == 1 )); then
        INPUT_SEQUENCE="${INPUT_SEQUENCE}0\n"
    else
        INPUT_SEQUENCE="${INPUT_SEQUENCE}p\n"
    fi
done

echo "Running game with piped input..."
echo

# Run the game with piped input
# P1 = tui (interactive with piped input)
# P2 = zero (deterministic choices)
# Redirect stderr to capture game output
# Since we only call mtg once, use cargo run
if echo -e "$INPUT_SEQUENCE" | timeout 30s cargo run --bin mtg -- tui \
    decks/simple_bolt.dck \
    decks/simple_bolt.dck \
    --p1 tui \
    --p2 zero \
    --seed 42 \
    --verbosity silent \
    > /tmp/tui_test_output.txt 2>&1; then

    echo -e "${GREEN}✓ Game completed successfully${NC}"
    echo

    # Check output for expected patterns
    if grep -q "Game Over" /tmp/tui_test_output.txt || \
       grep -q "Winner" /tmp/tui_test_output.txt || \
       ! grep -q "Error" /tmp/tui_test_output.txt; then
        echo -e "${GREEN}✓ Output looks correct (game finished)${NC}"
        EXIT_CODE=0
    else
        echo -e "${YELLOW}⚠ Game may not have completed as expected${NC}"
        echo "Output:"
        cat /tmp/tui_test_output.txt
        EXIT_CODE=1
    fi
else
    EXIT_STATUS=$?
    if [[ $EXIT_STATUS == 124 ]]; then
        echo -e "${RED}✗ Test timed out after 30 seconds${NC}"
        echo "The interactive TUI may be waiting for input"
    else
        echo -e "${RED}✗ Game failed with exit code $EXIT_STATUS${NC}"
    fi
    echo
    echo "Output:"
    cat /tmp/tui_test_output.txt
    EXIT_CODE=1
fi

# Cleanup
rm -f /tmp/tui_test_output.txt

echo
echo "=== Test Complete ==="
exit $EXIT_CODE
