#!/usr/bin/env bash
# Continue an agent play session with one more choice
# Usage: ./agentplay/continue_game.sh <choice>
# The game state determines which player's choice this is
# Examples:
#   ./agentplay/continue_game.sh "0"
#   ./agentplay/continue_game.sh "pass"
#   ./agentplay/continue_game.sh "play swamp"

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# Parse arguments
if [[ $# -lt 1 ]]; then
    echo "Error: One argument (choice) required"
    echo "Usage: $0 <choice>"
    echo "Examples:"
    echo "  $0 \"0\""
    echo "  $0 \"pass\""
    echo "  $0 \"play swamp\""
    exit 1
fi

NEW_CHOICE="$1"

# Check that we have a snapshot from a previous session
if [[ ! -f "$SCRIPT_DIR/game.snapshot" ]]; then
    echo "Error: No game.snapshot found. Run start_game.sh first."
    exit 1
fi

# Check that we have a choices file
if [[ ! -f "$SCRIPT_DIR/choices.txt" ]]; then
    echo "Error: choices.txt not found. Run start_game.sh first."
    exit 1
fi

# Append the new choice to the choices file
echo "$NEW_CHOICE" >> "$SCRIPT_DIR/choices.txt"

# Count total choices made so far
CHOICE_COUNT=$(wc -l < "$SCRIPT_DIR/choices.txt" | tr -d ' ')
NEXT_STOP=$((CHOICE_COUNT + 1))

# Read all choices and join with semicolons
if [[ -s "$SCRIPT_DIR/choices.txt" ]]; then
    CHOICES=$(tr '\n' ';' < "$SCRIPT_DIR/choices.txt" | sed 's/;$//')
else
    CHOICES=""
fi

# Build the mtg command - resume from snapshot with fixed controllers for both players
# Both players use the same choice script - the game engine will ask each player in turn
# Stop after one more choice (total choices + 1)
CMD=(
    cargo run --release --bin mtg -- resume
    "$SCRIPT_DIR/game.snapshot"
    --override-p1=fixed
    --override-p2=fixed
    --p1-fixed-inputs="$CHOICES"
    --p2-fixed-inputs="$CHOICES"
    --stop-on-choice="$NEXT_STOP"
    --snapshot-output="$SCRIPT_DIR/game.snapshot"
    --json  # Use JSON format for better debuggability
    --log-tail=100
)

# Build reproducer command
REPRODUCER_CMD="mtg resume \"$SCRIPT_DIR/game.snapshot\" --override-p1=fixed --override-p2=fixed --p1-fixed-inputs=\"$CHOICES\" --p2-fixed-inputs=\"$CHOICES\" --stop-on-choice=\"$NEXT_STOP\" --json --log-tail=100"

echo "============================================"
echo "Continuing agent play session"
echo "Choice #$CHOICE_COUNT: $NEW_CHOICE"
echo "Will stop after choice #$NEXT_STOP"
echo "============================================"
echo ""
echo "REPRODUCER: $REPRODUCER_CMD"
echo ""
echo "============================================"

# Run the command
cd "$REPO_ROOT"
"${CMD[@]}"

echo ""
echo "Use ./agentplay/continue_game.sh <choice> to add another choice."
