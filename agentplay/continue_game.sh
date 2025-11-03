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
GAME_DIR="$SCRIPT_DIR/current.game"

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

# Check that we have a game directory
if [[ ! -d "$GAME_DIR" ]]; then
    echo "Error: No current.game found. Run start_game.sh first."
    exit 1
fi

# Check that we have initial args
if [[ ! -f "$GAME_DIR/initial_args.txt" ]]; then
    echo "Error: initial_args.txt not found. Run start_game.sh first."
    exit 1
fi

# Check that we have a choices file
if [[ ! -f "$GAME_DIR/choices.txt" ]]; then
    echo "Error: choices.txt not found. Run start_game.sh first."
    exit 1
fi

# Append the new choice to the choices file
echo "$NEW_CHOICE" >> "$GAME_DIR/choices.txt"

# Count total choices made so far
CHOICE_COUNT=$(wc -l < "$GAME_DIR/choices.txt" | tr -d ' ')
NEXT_STOP=$((CHOICE_COUNT + 1))

# Read all choices and join with semicolons
if [[ -s "$GAME_DIR/choices.txt" ]]; then
    CHOICES=$(tr '\n' ';' < "$GAME_DIR/choices.txt" | sed 's/;$//')
else
    CHOICES=""
fi

# Read initial game arguments
INITIAL_ARGS=$(cat "$GAME_DIR/initial_args.txt")

# Build the mtg command - start from scratch with all choices accumulated so far
# Both players use the same choice script - the game engine will ask each player in turn
# Stop after one more choice (total choices + 1)
CMD=(
    cargo run --release --bin mtg -- tui
    $INITIAL_ARGS  # Note: no quotes to allow word splitting
    --p1=fixed
    --p2=fixed
    --p1-fixed-inputs="$CHOICES"
    --p2-fixed-inputs="$CHOICES"
    --stop-on-choice="$NEXT_STOP"
    --snapshot-output="$GAME_DIR/game.snapshot"
    --json  # Use JSON format for better debuggability
    --log-tail=100
    --seed=42  # Deterministic seed for reproducibility
)

# Build reproducer command (without cargo run for cleaner output)
REPRODUCER_CMD="mtg tui $INITIAL_ARGS --p1=fixed --p2=fixed --p1-fixed-inputs=\"$CHOICES\" --p2-fixed-inputs=\"$CHOICES\" --stop-on-choice=\"$NEXT_STOP\" --seed=42 --json --log-tail=100"

# Update the reproduce_game.sh script with current state
cat > "$GAME_DIR/reproduce_game.sh" <<EOF
#!/usr/bin/env bash
# Reproducer for this game session
# Generated: $(date)
# Choices made: $CHOICE_COUNT
set -euo pipefail

REPO_ROOT="\$(cd "\$(dirname "\${BASH_SOURCE[0]}")/../.." && pwd)"
cd "\$REPO_ROOT"

cargo run --release --bin mtg -- tui $INITIAL_ARGS \\
    --p1=fixed \\
    --p2=fixed \\
    --p1-fixed-inputs="$CHOICES" \\
    --p2-fixed-inputs="$CHOICES" \\
    --stop-on-choice=$NEXT_STOP \\
    --seed=42 \\
    --json \\
    --log-tail=100
EOF
chmod +x "$GAME_DIR/reproduce_game.sh"

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
echo "Or run ./agentplay/current.game/reproduce_game.sh to replay the full session."
