#!/usr/bin/env bash
# Start a new agent play session
# Usage: ./agentplay/start_game.sh [--game-dir=path] <args for mtg tui>
# Examples:
#   ./agentplay/start_game.sh decks/simple_bolt.dck decks/simple_bolt.dck
#   ./agentplay/start_game.sh decks/simple_bolt.dck --p1-draw="Mountain;Lightning Bolt"
#   ./agentplay/start_game.sh --start-state="foo.pzl"
#   ./agentplay/start_game.sh --game-dir=my_test.game decks/simple_bolt.dck decks/simple_bolt.dck

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# Parse --game-dir flag if present
GAME_DIR=""
MTG_ARGS=()

while [[ $# -gt 0 ]]; do
    case "$1" in
        --game-dir=*)
            GAME_DIR="${1#*=}"
            # Convert relative path to absolute
            if [[ ! "$GAME_DIR" = /* ]]; then
                GAME_DIR="$SCRIPT_DIR/$GAME_DIR"
            fi
            shift
            ;;
        --game-dir)
            if [[ $# -lt 2 ]]; then
                echo "Error: --game-dir requires a value"
                exit 1
            fi
            GAME_DIR="$2"
            # Convert relative path to absolute
            if [[ ! "$GAME_DIR" = /* ]]; then
                GAME_DIR="$SCRIPT_DIR/$GAME_DIR"
            fi
            shift 2
            ;;
        *)
            MTG_ARGS+=("$1")
            shift
            ;;
    esac
done

# If --game-dir was specified, use it and don't touch current.game symlink
UPDATE_SYMLINK=true
if [[ -n "$GAME_DIR" ]]; then
    echo "Using explicit game directory: $GAME_DIR"
    UPDATE_SYMLINK=false

    # Check if directory already exists
    if [[ -d "$GAME_DIR" ]]; then
        echo "Error: Game directory already exists: $GAME_DIR"
        echo "Please choose a different directory or remove the existing one."
        exit 1
    fi
else
    # Auto-numbered game mode: find next available number
    GAME_NUM=1
    while [[ -d "$SCRIPT_DIR/$(printf "%03d" $GAME_NUM).game" ]]; do
        GAME_NUM=$((GAME_NUM + 1))
    done

    GAME_DIR="$SCRIPT_DIR/$(printf "%03d" $GAME_NUM).game"
    echo "Creating new numbered game: $(basename "$GAME_DIR")"
fi

# Create fresh game directory
mkdir -p "$GAME_DIR"

# Initialize empty choice files for each player
touch "$GAME_DIR/p1_choices.txt"
touch "$GAME_DIR/p2_choices.txt"

# Store the initial game arguments for reproducers
printf '%s\n' "${MTG_ARGS[@]}" > "$GAME_DIR/initial_args.txt"

# Build the mtg command - use --stop-on-choice=1 to stop after the first choice
CMD=(
    cargo run --release --bin mtg -- tui
    "${MTG_ARGS[@]}"  # Pass through all user arguments
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
REPRODUCER_CMD="mtg tui ${MTG_ARGS[*]} --p1=fixed --p2=fixed --p1-fixed-inputs=\"\" --p2-fixed-inputs=\"\" --stop-on-choice=1 --seed=42 --json --log-tail=100"

# Save reproducer script
cat > "$GAME_DIR/reproduce_game.sh" <<EOF
#!/usr/bin/env bash
# Reproducer for this game session
# Generated: $(date)
set -euo pipefail

REPO_ROOT="\$(cd "\$(dirname "\${BASH_SOURCE[0]}")/../.." && pwd)"
cd "\$REPO_ROOT"

cargo run --release --bin mtg -- tui ${MTG_ARGS[*]} \\
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

# Update current.game symlink if not using explicit --game-dir
if [[ "$UPDATE_SYMLINK" == true ]]; then
    SYMLINK_PATH="$SCRIPT_DIR/current.game"

    # Remove old symlink/directory if it exists
    if [[ -L "$SYMLINK_PATH" ]]; then
        rm "$SYMLINK_PATH"
    elif [[ -d "$SYMLINK_PATH" ]]; then
        # Old-style directory - archive it
        ARCHIVE_NUM=1
        while [[ -d "$SCRIPT_DIR/$(printf "%03d" $ARCHIVE_NUM).game" ]]; do
            ARCHIVE_NUM=$((ARCHIVE_NUM + 1))
        done
        ARCHIVE_DIR="$SCRIPT_DIR/$(printf "%03d" $ARCHIVE_NUM).game"
        mv "$SYMLINK_PATH" "$ARCHIVE_DIR"
        echo "Archived old current.game directory to: $(basename "$ARCHIVE_DIR")"
    fi

    # Create new symlink
    ln -s "$(basename "$GAME_DIR")" "$SYMLINK_PATH"
    echo "Created symlink: current.game -> $(basename "$GAME_DIR")"
fi

echo "============================================"
echo "Starting new agent play session"
echo "Game directory: $(basename "$GAME_DIR")"
echo "============================================"
echo ""
echo "REPRODUCER: $REPRODUCER_CMD"
echo ""
echo "============================================"

# Run the command
cd "$REPO_ROOT"
"${CMD[@]}"

echo ""
if [[ "$UPDATE_SYMLINK" == true ]]; then
    echo "Session initialized. Use ./agentplay/continue_game.sh <choice> to continue."
else
    echo "Session initialized. Use ./agentplay/continue_game.sh --game-dir=\"$GAME_DIR\" <choice> to continue."
fi
