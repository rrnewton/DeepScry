#!/usr/bin/env bash
# Continue an agent play session with one more choice for a specific player
# Usage: ./agentplay/continue_game.sh <choice> [--p1|--p2]
# If no player specified, defaults to --p1
# Examples:
#   ./agentplay/continue_game.sh "1"           # Add choice for P1
#   ./agentplay/continue_game.sh "pass" --p2   # Add choice for P2
#   ./agentplay/continue_game.sh "play swamp" --p1

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# Parse arguments
if [[ $# -lt 1 ]]; then
    echo "Error: At least one argument (choice) required"
    echo "Usage: $0 <choice> [--p1|--p2]"
    echo "Examples:"
    echo "  $0 \"1\""
    echo "  $0 \"pass\" --p2"
    echo "  $0 \"play swamp\""
    exit 1
fi

NEW_CHOICE="$1"
PLAYER="p1"  # Default to p1

# Check for optional player flag
if [[ $# -ge 2 ]]; then
    case $2 in
        --p1)
            PLAYER="p1"
            ;;
        --p2)
            PLAYER="p2"
            ;;
        *)
            echo "Error: Unknown player flag: $2"
            echo "Use --p1 or --p2"
            exit 1
            ;;
    esac
fi

# Check that we have a snapshot from a previous session
if [[ ! -f "$SCRIPT_DIR/game.snapshot" ]]; then
    echo "Error: No game.snapshot found. Run start_game.sh first."
    exit 1
fi

# Check that we have choice files
if [[ ! -f "$SCRIPT_DIR/p1_choices.txt" ]] || [[ ! -f "$SCRIPT_DIR/p2_choices.txt" ]]; then
    echo "Error: Choice files not found. Run start_game.sh first."
    exit 1
fi

# Append the new choice to the appropriate player's choices file
if [[ "$PLAYER" == "p1" ]]; then
    echo "$NEW_CHOICE" >> "$SCRIPT_DIR/p1_choices.txt"
else
    echo "$NEW_CHOICE" >> "$SCRIPT_DIR/p2_choices.txt"
fi

# Read all choices for each player and join with semicolons
P1_CHOICES=$(tr '\n' ';' < "$SCRIPT_DIR/p1_choices.txt" | sed 's/;$//')
P2_CHOICES=$(tr '\n' ';' < "$SCRIPT_DIR/p2_choices.txt" | sed 's/;$//')

# Build the mtg command - resume from snapshot with updated fixed inputs for both players
CMD=(
    cargo run --release --bin mtg -- resume
    "$SCRIPT_DIR/game.snapshot"
    --override-p1=fixed
    --override-p2=fixed
    --p1-fixed-inputs="$P1_CHOICES"
    --p2-fixed-inputs="$P2_CHOICES"
    --stop-when-fixed-exhausted
    --snapshot-output="$SCRIPT_DIR/game.snapshot"
    --json  # Use JSON format for better debuggability
    --log-tail=80
)

# Build reproducer command
REPRODUCER_CMD="mtg resume \"$SCRIPT_DIR/game.snapshot\" --override-p1=fixed --override-p2=fixed --p1-fixed-inputs=\"$P1_CHOICES\" --p2-fixed-inputs=\"$P2_CHOICES\" --json --log-tail=80"

P1_COUNT=$(wc -l < "$SCRIPT_DIR/p1_choices.txt")
P2_COUNT=$(wc -l < "$SCRIPT_DIR/p2_choices.txt")

echo "============================================"
echo "Continuing agent play session"
if [[ "$PLAYER" == "p1" ]]; then
    echo "P1 choice #$P1_COUNT: $NEW_CHOICE"
else
    echo "P2 choice #$P2_COUNT: $NEW_CHOICE"
fi
echo "Total: P1=$P1_COUNT choices, P2=$P2_COUNT choices"
echo "============================================"
echo ""
echo "REPRODUCER: $REPRODUCER_CMD"
echo ""
echo "============================================"

# Run the command
cd "$REPO_ROOT"
"${CMD[@]}"

echo ""
echo "Use ./agentplay/continue_game.sh <choice> [--p1|--p2] to add another choice."
