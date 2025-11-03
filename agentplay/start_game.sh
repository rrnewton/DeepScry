#!/usr/bin/env bash
# Start a new agent play session
# Usage: ./agentplay/start_game.sh <args for mtg tui>
# Examples:
#   ./agentplay/start_game.sh decks/simple_bolt.dck decks/simple_bolt.dck
#   ./agentplay/start_game.sh decks/simple_bolt.dck --p1-draw="Mountain;Lightning Bolt"
#   ./agentplay/start_game.sh --start-state="foo.pzl"

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# Clean up any previous session
rm -f "$SCRIPT_DIR"/*.snapshot
rm -f "$SCRIPT_DIR"/*.log
rm -f "$SCRIPT_DIR"/choices.txt
rm -f "$SCRIPT_DIR"/p1_choices.txt
rm -f "$SCRIPT_DIR"/p2_choices.txt
rm -f "$SCRIPT_DIR"/last_command.txt

# Initialize empty choices file
touch "$SCRIPT_DIR/choices.txt"

# Build the mtg command - use --stop-on-choice=1 to stop after the first choice
# Use heuristic controllers for initialization, then continue_game.sh will switch to fixed
CMD=(
    cargo run --release --bin mtg -- tui
    "$@"  # Pass through all user arguments
    --p1=heuristic
    --p2=heuristic
    --stop-on-choice=1
    --snapshot-output="$SCRIPT_DIR/game.snapshot"
    --json  # Use JSON format for better debuggability
    --log-tail=100
    --seed=42  # Deterministic seed for reproducibility
)

# Build reproducer command
REPRODUCER_CMD="mtg tui $* --p1=heuristic --p2=heuristic --stop-on-choice=1 --seed=42 --json --log-tail=100"

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
echo "Session initialized. Use ./agentplay/continue_game.sh <choice> to add choices."
