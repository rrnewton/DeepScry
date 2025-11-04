#!/bin/bash
set -euo pipefail

# Random Decks Tournament Script
# Selects 50 random decks from the full deck list and runs a 10-second tournament
# with Random vs Heuristic controllers

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

# Configuration
NUM_DECKS=50
TOURNAMENT_SECONDS=10
DECK_LIST_FILE="full_deck_list.txt"

# Colors for output
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo -e "${BLUE}=== Random Decks Tournament ===${NC}"
echo ""

# Ensure deck list exists
if [ ! -f "$DECK_LIST_FILE" ]; then
    echo -e "${YELLOW}Building deck list...${NC}"
    make "$DECK_LIST_FILE"
fi

# Get total number of decks
TOTAL_DECKS=$(wc -l < "$DECK_LIST_FILE")
echo -e "${BLUE}Total decks available:${NC} $TOTAL_DECKS"
echo -e "${BLUE}Selecting:${NC} $NUM_DECKS random decks"
echo ""

# Select 50 random deck paths using shuf (shuffle and take first N lines)
# shuf is part of GNU coreutils and should be available on most systems
if ! command -v shuf &> /dev/null; then
    echo "Error: 'shuf' command not found. Please install GNU coreutils."
    exit 1
fi

# Create temporary file for selected decks
TEMP_DECKS=$(mktemp)
trap "rm -f $TEMP_DECKS" EXIT

# Select random decks
shuf -n "$NUM_DECKS" "$DECK_LIST_FILE" > "$TEMP_DECKS"

echo -e "${GREEN}Selected decks:${NC}"
cat "$TEMP_DECKS" | head -10
if [ "$NUM_DECKS" -gt 10 ]; then
    echo "... ($(($NUM_DECKS - 10)) more)"
fi
echo ""

# Build the binary if needed
echo -e "${YELLOW}Building release binary...${NC}"
cargo build --release --quiet

echo ""
echo -e "${BLUE}Running tournament for ${TOURNAMENT_SECONDS} seconds...${NC}"
echo -e "${BLUE}Controllers: P1=Random, P2=Heuristic${NC}"
echo ""

# Read deck paths into array and pass to tourney command
mapfile -t DECK_PATHS < "$TEMP_DECKS"

# Run tournament
./target/release/mtg tourney --seconds="$TOURNAMENT_SECONDS" --p1=random --p2=heuristic "${DECK_PATHS[@]}"

echo ""
echo -e "${GREEN}=== Tournament Complete ===${NC}"
