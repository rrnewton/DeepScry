#!/usr/bin/env bash
# Continue an agent play session with one more choice
# Usage: ./agentplay/continue_game.sh [--game-dir=path] <choice>
#        ./agentplay/continue_game.sh [--game-dir=path] --p1 <choice>
#        ./agentplay/continue_game.sh [--game-dir=path] --p2 <choice>
#
# The script auto-detects whose turn it is from the snapshot.
# You can optionally specify --p1 or --p2 for sanity checking.
#
# Examples:
#   ./agentplay/continue_game.sh "1"                # Auto-detect whose turn (uses current.game)
#   ./agentplay/continue_game.sh --p1 "1"           # Assert it's P1's turn
#   ./agentplay/continue_game.sh --p2 "0"           # Assert it's P2's turn
#   ./agentplay/continue_game.sh --p1 "play mountain"
#   ./agentplay/continue_game.sh --game-dir=042.game "1"  # Continue specific game

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# Parse arguments
GAME_DIR=""
PLAYER_OVERRIDE=""  # Set if user explicitly specifies --p1 or --p2
CHOICE=""

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
        --p1)
            PLAYER_OVERRIDE="p1"
            shift
            if [[ $# -eq 0 ]]; then
                echo "Error: --p1 requires a choice argument"
                exit 1
            fi
            CHOICE="$1"
            shift
            ;;
        --p2)
            PLAYER_OVERRIDE="p2"
            shift
            if [[ $# -eq 0 ]]; then
                echo "Error: --p2 requires a choice argument"
                exit 1
            fi
            CHOICE="$1"
            shift
            ;;
        *)
            # If no flag, treat as the choice (auto-detect player)
            if [[ -z "$CHOICE" ]]; then
                CHOICE="$1"
                shift
            else
                echo "Error: Unknown argument: $1"
                echo "Usage: $0 [--game-dir=path] <choice>  OR  $0 [--game-dir=path] --p1 <choice>  OR  $0 [--game-dir=path] --p2 <choice>"
                exit 1
            fi
            ;;
    esac
done

if [[ -z "$CHOICE" ]]; then
    echo "Error: Must provide a choice"
    echo "Usage: $0 [--game-dir=path] <choice>  OR  $0 [--game-dir=path] --p1 <choice>  OR  $0 [--game-dir=path] --p2 <choice>"
    echo "Examples:"
    echo "  $0 \"1\"                # Auto-detect whose turn (uses current.game)"
    echo "  $0 --p1 \"1\"           # Assert it's P1's turn"
    echo "  $0 --p2 \"0\"           # Assert it's P2's turn"
    echo "  $0 --game-dir=042.game \"1\"  # Continue specific game"
    exit 1
fi

# Default to current.game if --game-dir not specified
if [[ -z "$GAME_DIR" ]]; then
    GAME_DIR="$SCRIPT_DIR/current.game"
fi

# Check that we have a game directory
if [[ ! -d "$GAME_DIR" ]]; then
    echo "Error: Game directory not found: $GAME_DIR"
    if [[ "$GAME_DIR" == "$SCRIPT_DIR/current.game" ]]; then
        echo "Hint: Run start_game.sh first to create a game."
    fi
    exit 1
fi

# Check that we have initial args
if [[ ! -f "$GAME_DIR/initial_args.txt" ]]; then
    echo "Error: initial_args.txt not found in $GAME_DIR"
    echo "This doesn't appear to be a valid game directory."
    exit 1
fi

# Check that we have choice files
if [[ ! -f "$GAME_DIR/p1_choices.txt" ]] || [[ ! -f "$GAME_DIR/p2_choices.txt" ]]; then
    echo "Error: Choice files not found in $GAME_DIR"
    exit 1
fi

# Auto-detect whose turn it is from the snapshot (if it exists)
DETECTED_PLAYER=""
if [[ -f "$GAME_DIR/game.snapshot" ]]; then
    # Check if jq is available
    if ! command -v jq &> /dev/null; then
        echo "Warning: jq not found, cannot auto-detect turn. Please install jq or specify --p1/--p2 explicitly."
        if [[ -z "$PLAYER_OVERRIDE" ]]; then
            echo "Error: Cannot auto-detect turn without jq. Please specify --p1 or --p2."
            exit 1
        fi
    else
        # Extract active player ID from snapshot
        ACTIVE_PLAYER_ID=$(jq -r '.game_state.turn.active_player' "$GAME_DIR/game.snapshot" 2>/dev/null || echo "")

        if [[ "$ACTIVE_PLAYER_ID" == "0" ]]; then
            DETECTED_PLAYER="p1"
        elif [[ "$ACTIVE_PLAYER_ID" == "1" ]]; then
            DETECTED_PLAYER="p2"
        else
            echo "Warning: Could not determine active player from snapshot (got: $ACTIVE_PLAYER_ID)"
            if [[ -z "$PLAYER_OVERRIDE" ]]; then
                echo "Error: Cannot auto-detect turn. Please specify --p1 or --p2."
                exit 1
            fi
        fi
    fi
else
    # No snapshot yet (first move) - must be P1's turn
    DETECTED_PLAYER="p1"
fi

# Determine which player to use
if [[ -n "$PLAYER_OVERRIDE" ]]; then
    # User specified --p1 or --p2 explicitly
    # Validate it matches auto-detected player (if we detected one)
    if [[ -n "$DETECTED_PLAYER" ]] && [[ "$PLAYER_OVERRIDE" != "$DETECTED_PLAYER" ]]; then
        echo "Error: You specified --$PLAYER_OVERRIDE but the snapshot shows it's $DETECTED_PLAYER's turn!"
        echo "  Snapshot shows active player ID: $ACTIVE_PLAYER_ID (0=P1/Alice, 1=P2/Bob)"
        echo "  Either use auto-detection (omit --p1/--p2) or fix the player flag."
        exit 1
    fi
    PLAYER="$PLAYER_OVERRIDE"
    echo "Using explicitly specified player: $PLAYER (validated against snapshot)"
else
    # Auto-detect mode
    PLAYER="$DETECTED_PLAYER"
    echo "Auto-detected turn: $PLAYER"
fi

# Append the new choice to the appropriate player's choice file
if [[ "$PLAYER" == "p1" ]]; then
    echo "$CHOICE" >> "$GAME_DIR/p1_choices.txt"
else
    echo "$CHOICE" >> "$GAME_DIR/p2_choices.txt"
fi

# Count total choices made so far (for both players combined)
P1_COUNT=$(wc -l < "$GAME_DIR/p1_choices.txt" | tr -d ' ')
P2_COUNT=$(wc -l < "$GAME_DIR/p2_choices.txt" | tr -d ' ')
TOTAL_CHOICES=$((P1_COUNT + P2_COUNT))
NEXT_STOP=$((TOTAL_CHOICES + 1))

# Read all choices and join with semicolons
if [[ -s "$GAME_DIR/p1_choices.txt" ]]; then
    P1_CHOICES=$(tr '\n' ';' < "$GAME_DIR/p1_choices.txt" | sed 's/;$//')
else
    P1_CHOICES=""
fi

if [[ -s "$GAME_DIR/p2_choices.txt" ]]; then
    P2_CHOICES=$(tr '\n' ';' < "$GAME_DIR/p2_choices.txt" | sed 's/;$//')
else
    P2_CHOICES=""
fi

# Read initial game arguments - handle multiline format
mapfile -t INITIAL_ARGS < "$GAME_DIR/initial_args.txt"

# Build the mtg command - start from scratch with all choices accumulated so far
CMD=(
    cargo run --release --bin mtg -- tui
    "${INITIAL_ARGS[@]}"
    --p1=fixed
    --p2=fixed
    --p1-fixed-inputs="$P1_CHOICES"
    --p2-fixed-inputs="$P2_CHOICES"
    --stop-on-choice="$NEXT_STOP"
    --snapshot-output="$GAME_DIR/game.snapshot"
    --json  # Use JSON format for better debuggability
    --log-tail=100
    --seed=42  # Deterministic seed for reproducibility
)

# Build reproducer command (without cargo run for cleaner output)
REPRODUCER_CMD="mtg tui ${INITIAL_ARGS[*]} --p1=fixed --p2=fixed --p1-fixed-inputs=\"$P1_CHOICES\" --p2-fixed-inputs=\"$P2_CHOICES\" --stop-on-choice=\"$NEXT_STOP\" --seed=42 --json --log-tail=100"

# Update the reproduce_game.sh script with current state
cat > "$GAME_DIR/reproduce_game.sh" <<EOF
#!/usr/bin/env bash
# Reproducer for this game session
# Generated: $(date)
# P1 choices: $P1_COUNT | P2 choices: $P2_COUNT | Total: $TOTAL_CHOICES
set -euo pipefail

REPO_ROOT="\$(cd "\$(dirname "\${BASH_SOURCE[0]}")/../.." && pwd)"
cd "\$REPO_ROOT"

cargo run --release --bin mtg -- tui ${INITIAL_ARGS[*]} \\
    --p1=fixed \\
    --p2=fixed \\
    --p1-fixed-inputs="$P1_CHOICES" \\
    --p2-fixed-inputs="$P2_CHOICES" \\
    --stop-on-choice=$NEXT_STOP \\
    --seed=42 \\
    --json \\
    --log-tail=100
EOF
chmod +x "$GAME_DIR/reproduce_game.sh"

echo "============================================"
echo "Continuing agent play session"
echo "Game directory: $(basename "$GAME_DIR")"
echo "Choice for $PLAYER: $CHOICE"
echo "P1 choices so far: $P1_COUNT | P2 choices so far: $P2_COUNT"
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
echo "Use ./agentplay/continue_game.sh <choice> to add another choice (auto-detects turn)."
echo "Or use --p1/--p2 flags for explicit turn specification."
echo "Or run $(basename "$GAME_DIR")/reproduce_game.sh to replay the full session."
