#!/usr/bin/env bash
# Continue an agent play session with one more choice
# Usage: ./agentplay/continue_game.sh --p1 <choice>  OR  ./agentplay/continue_game.sh --p2 <choice>
# Examples:
#   ./agentplay/continue_game.sh --p1 "1"
#   ./agentplay/continue_game.sh --p2 "0"
#   ./agentplay/continue_game.sh --p1 "play mountain"

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
GAME_DIR="$SCRIPT_DIR/current.game"

# Parse arguments
PLAYER=""
CHOICE=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        --p1)
            PLAYER="p1"
            shift
            if [[ $# -eq 0 ]]; then
                echo "Error: --p1 requires a choice argument"
                exit 1
            fi
            CHOICE="$1"
            shift
            ;;
        --p2)
            PLAYER="p2"
            shift
            if [[ $# -eq 0 ]]; then
                echo "Error: --p2 requires a choice argument"
                exit 1
            fi
            CHOICE="$1"
            shift
            ;;
        *)
            echo "Error: Unknown argument: $1"
            echo "Usage: $0 --p1 <choice>  OR  $0 --p2 <choice>"
            exit 1
            ;;
    esac
done

if [[ -z "$PLAYER" ]]; then
    echo "Error: Must specify --p1 or --p2"
    echo "Usage: $0 --p1 <choice>  OR  $0 --p2 <choice>"
    echo "Examples:"
    echo "  $0 --p1 \"1\""
    echo "  $0 --p2 \"0\""
    echo "  $0 --p1 \"play mountain\""
    exit 1
fi

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

# Check that we have choice files
if [[ ! -f "$GAME_DIR/p1_choices.txt" ]] || [[ ! -f "$GAME_DIR/p2_choices.txt" ]]; then
    echo "Error: Choice files not found. Run start_game.sh first."
    exit 1
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

# Read initial game arguments
INITIAL_ARGS=$(cat "$GAME_DIR/initial_args.txt")

# Build the mtg command - start from scratch with all choices accumulated so far
CMD=(
    cargo run --release --bin mtg -- tui
    $INITIAL_ARGS  # Note: no quotes to allow word splitting
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
REPRODUCER_CMD="mtg tui $INITIAL_ARGS --p1=fixed --p2=fixed --p1-fixed-inputs=\"$P1_CHOICES\" --p2-fixed-inputs=\"$P2_CHOICES\" --stop-on-choice=\"$NEXT_STOP\" --seed=42 --json --log-tail=100"

# Update the reproduce_game.sh script with current state
cat > "$GAME_DIR/reproduce_game.sh" <<EOF
#!/usr/bin/env bash
# Reproducer for this game session
# Generated: $(date)
# P1 choices: $P1_COUNT | P2 choices: $P2_COUNT | Total: $TOTAL_CHOICES
set -euo pipefail

REPO_ROOT="\$(cd "\$(dirname "\${BASH_SOURCE[0]}")/../.." && pwd)"
cd "\$REPO_ROOT"

cargo run --release --bin mtg -- tui $INITIAL_ARGS \\
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
echo "Use ./agentplay/continue_game.sh --p1 <choice> or --p2 <choice> to add another choice."
echo "Or run ./agentplay/current.game/reproduce_game.sh to replay the full session."
