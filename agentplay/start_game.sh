#!/usr/bin/env bash
# Start a new agent play session
# Usage: ./agentplay/start_game.sh <args for mtg tui>
# Examples:
#   ./agentplay/start_game.sh deck1.dck deck2.dck
#   ./agentplay/start_game.sh deck1.dck --p1-draw="Mountain;Lightning Bolt"
#   ./agentplay/start_game.sh --start-state="foo.pzl"

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# Clean up any previous session
rm -f "$SCRIPT_DIR"/*.snapshot
rm -f "$SCRIPT_DIR"/*.log
rm -f "$SCRIPT_DIR"/p1_choices.txt
rm -f "$SCRIPT_DIR"/p2_choices.txt
rm -f "$SCRIPT_DIR"/last_command.txt

# Initialize choices files with first choice "1" for each player
echo "1" > "$SCRIPT_DIR/p1_choices.txt"
echo "1" > "$SCRIPT_DIR/p2_choices.txt"

# Build the mtg command - pass all args through to mtg tui
CMD=(
    cargo run --release --bin mtg -- tui
    "$@"  # Pass through all user arguments
    --p1=fixed
    --p2=fixed
    --stop-when-fixed-exhausted
    --snapshot-output="$SCRIPT_DIR/game.snapshot"
    --json  # Use JSON format for better debuggability
    --log-tail=80
    --seed=42  # Deterministic seed for reproducibility
    --p1-fixed-inputs="1"
    --p2-fixed-inputs="1"
)

# Build reproducer command
REPRODUCER_CMD="mtg tui $* --p1=fixed --p2=fixed --seed=42 --json --log-tail=80 --p1-fixed-inputs=\"1\" --p2-fixed-inputs=\"1\""

echo "============================================"
echo "Starting new agent play session"
echo "============================================"
echo ""
echo "REPRODUCER: $REPRODUCER_CMD"
echo ""
echo "============================================"

# Run the command
cd "$REPO_ROOT"
"${CMD[@]}"

echo ""
echo "Session initialized. Use ./agentplay/continue_game.sh <choice> to continue."
