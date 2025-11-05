#!/usr/bin/env bash
# E2E test for puzzle loading via CLI
#
# This test verifies that the `mtg tui --start-state` command correctly
# loads puzzle files and runs games from specific board states.
#
# Test scenarios:
# 1. Load Grizzly Bears puzzle and verify game runs without immediate decking
# 2. Load Royal Assassin puzzle and verify game completes

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo "=== Puzzle Loading E2E Test ==="
echo

echo "Will use: cargo run --bin mtg"
echo

# Check if cardsfolder exists
if [[ ! -d "cardsfolder" ]]; then
    echo -e "${YELLOW}Warning: cardsfolder not found, skipping test${NC}"
    exit 0
fi

# Check if puzzle files exist
if [[ ! -f "test_puzzles/grizzly_bears_should_attack.pzl" ]]; then
    echo -e "${RED}Error: test_puzzles/grizzly_bears_should_attack.pzl not found${NC}"
    exit 1
fi

if [[ ! -f "test_puzzles/royal_assassin_kills_attacker.pzl" ]]; then
    echo -e "${RED}Error: test_puzzles/royal_assassin_kills_attacker.pzl not found${NC}"
    exit 1
fi

EXIT_CODE=0

# Test 1: Grizzly Bears puzzle
echo "=== Test 1: Grizzly Bears Attack Puzzle ==="
echo "Loading puzzle: test_puzzles/grizzly_bears_should_attack.pzl"
echo "Controllers: Heuristic vs Heuristic"
echo "Seed: 12345 (deterministic)"
echo

# Since we call mtg twice total in this script, use cargo run each time
if timeout 30s cargo run --bin mtg -- tui \
    --start-state test_puzzles/grizzly_bears_should_attack.pzl \
    --p1 heuristic \
    --p2 heuristic \
    --seed 12345 \
    --verbosity verbose \
    > /tmp/puzzle_grizzly_bears_test.txt 2>&1; then

    echo -e "${GREEN}✓ Game completed successfully${NC}"
    echo

    # Verify game didn't end by immediate decking
    if grep -qi "decking" /tmp/puzzle_grizzly_bears_test.txt; then
        # Check if it was immediate (turn 5 or 6)
        if grep -E "Turn: [5-6]" /tmp/puzzle_grizzly_bears_test.txt | grep -qi "decking"; then
            echo -e "${RED}✗ FAILURE: Game ended by immediate decking${NC}"
            echo "This suggests puzzle libraries are empty or too small"
            EXIT_CODE=1
        else
            echo -e "${GREEN}✓ Game ended by decking after several turns (expected)${NC}"
        fi
    else
        echo -e "${GREEN}✓ Game ended by other means (PlayerDeath expected)${NC}"
    fi

    # Verify puzzle was loaded
    if grep -qi "puzzle:" /tmp/puzzle_grizzly_bears_test.txt; then
        echo -e "${GREEN}✓ Puzzle metadata loaded${NC}"
    fi

    # Verify Grizzly Bears was on battlefield
    if grep -qi "grizzly bears" /tmp/puzzle_grizzly_bears_test.txt; then
        echo -e "${GREEN}✓ Grizzly Bears found in game${NC}"
    else
        echo -e "${RED}✗ Grizzly Bears not found - puzzle may not have loaded${NC}"
        EXIT_CODE=1
    fi

else
    EXIT_STATUS=$?
    if [[ $EXIT_STATUS == 124 ]]; then
        echo -e "${RED}✗ Test timed out after 30 seconds${NC}"
    else
        echo -e "${RED}✗ Game failed with exit code $EXIT_STATUS${NC}"
    fi
    echo
    echo "Output (first 50 lines):"
    head -50 /tmp/puzzle_grizzly_bears_test.txt
    EXIT_CODE=1
fi

echo
echo "Full log: /tmp/puzzle_grizzly_bears_test.txt"
echo

# Test 2: Royal Assassin puzzle
echo "=== Test 2: Royal Assassin Puzzle ==="
echo "Loading puzzle: test_puzzles/royal_assassin_kills_attacker.pzl"
echo "Controllers: Heuristic vs Heuristic"
echo "Seed: 42 (deterministic)"
echo

if timeout 30s cargo run --bin mtg -- tui \
    --start-state test_puzzles/royal_assassin_kills_attacker.pzl \
    --p1 heuristic \
    --p2 heuristic \
    --seed 42 \
    --verbosity verbose \
    > /tmp/puzzle_royal_assassin_test.txt 2>&1; then

    echo -e "${GREEN}✓ Game completed successfully${NC}"
    echo

    # Verify game didn't end by immediate decking
    if grep -qi "decking" /tmp/puzzle_royal_assassin_test.txt; then
        if grep -E "Turn: [3-4]" /tmp/puzzle_royal_assassin_test.txt | grep -qi "decking"; then
            echo -e "${RED}✗ FAILURE: Game ended by immediate decking${NC}"
            EXIT_CODE=1
        else
            echo -e "${GREEN}✓ Game ended by decking after several turns (acceptable)${NC}"
        fi
    else
        echo -e "${GREEN}✓ Game ended by other means${NC}"
    fi

    # Verify Royal Assassin was on battlefield
    if grep -qi "royal assassin" /tmp/puzzle_royal_assassin_test.txt; then
        echo -e "${GREEN}✓ Royal Assassin found in game${NC}"
    else
        echo -e "${RED}✗ Royal Assassin not found - puzzle may not have loaded${NC}"
        EXIT_CODE=1
    fi

    # Verify puzzle starting turn
    if grep -qi "turn 3\|turn: 3" /tmp/puzzle_royal_assassin_test.txt | head -1; then
        echo -e "${GREEN}✓ Game started at turn 3 as expected${NC}"
    fi

else
    EXIT_STATUS=$?
    if [[ $EXIT_STATUS == 124 ]]; then
        echo -e "${RED}✗ Test timed out after 30 seconds${NC}"
    else
        echo -e "${RED}✗ Game failed with exit code $EXIT_STATUS${NC}"
    fi
    echo
    echo "Output (first 50 lines):"
    head -50 /tmp/puzzle_royal_assassin_test.txt
    EXIT_CODE=1
fi

echo
echo "Full log: /tmp/puzzle_royal_assassin_test.txt"
echo

# Summary
echo "=== Test Summary ==="
if [[ $EXIT_CODE == 0 ]]; then
    echo -e "${GREEN}✓ All puzzle loading tests passed${NC}"
else
    echo -e "${RED}✗ Some tests failed${NC}"
fi

exit $EXIT_CODE
