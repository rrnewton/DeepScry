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
GAME_DIR="$SCRIPT_DIR/current.game"

# Archive existing session if present
if [[ -d "$GAME_DIR" ]]; then
    echo "============================================"
    echo "WARNING: Existing session found at current.game"
    echo "============================================"

    # Find next available archive number
    ARCHIVE_NUM=1
    while [[ -d "$SCRIPT_DIR/$(printf "%03d" $ARCHIVE_NUM).game" ]]; do
        ARCHIVE_NUM=$((ARCHIVE_NUM + 1))
    done

    ARCHIVE_DIR="$SCRIPT_DIR/$(printf "%03d" $ARCHIVE_NUM).game"
    mv "$GAME_DIR" "$ARCHIVE_DIR"
    echo "Archived previous session to: $(basename "$ARCHIVE_DIR")"
    echo ""
fi

# Create fresh game directory
mkdir -p "$GAME_DIR"

# Initialize empty choice files for each player
touch "$GAME_DIR/p1_choices.txt"
touch "$GAME_DIR/p2_choices.txt"

# Store the initial game arguments for reproducers
echo "$@" > "$GAME_DIR/initial_args.txt"

# Build the mtg command - use --stop-on-choice=1 to stop after the first choice
CMD=(
    cargo run --release --bin mtg -- tui
    "$@"  # Pass through all user arguments
    --p1=fixed
    --p2=fixed
    --p1-fixed-inputs=""
    --p2-fixed-inputs=""
    --stop-on-choice=1
    --snapshot-output="$GAME_DIR/game.snapshot"
    --json  # Use JSON format for better debuggability
    --log-tail=100
    --seed=42  # Deterministic seed for reproducibility
)

# Build reproducer command (without cargo run for cleaner output)
REPRODUCER_CMD="mtg tui $* --p1=fixed --p2=fixed --p1-fixed-inputs=\"\" --p2-fixed-inputs=\"\" --stop-on-choice=1 --seed=42 --json --log-tail=100"

# Save reproducer script
cat > "$GAME_DIR/reproduce_game.sh" <<EOF
#!/usr/bin/env bash
# Reproducer for this game session
# Generated: $(date)
set -euo pipefail

REPO_ROOT="\$(cd "\$(dirname "\${BASH_SOURCE[0]}")/../.." && pwd)"
cd "\$REPO_ROOT"

cargo run --release --bin mtg -- tui $* \\
    --p1=fixed \\
    --p2=fixed \\
    --p1-fixed-inputs="" \\
    --p2-fixed-inputs="" \\
    --stop-on-choice=1 \\
    --seed=42 \\
    --json \\
    --log-tail=100
EOF
chmod +x "$GAME_DIR/reproduce_game.sh"

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
echo "Session initialized. Use ./agentplay/continue_game.sh --p1 <choice> or --p2 <choice> to add choices."
