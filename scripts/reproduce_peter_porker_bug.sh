#!/usr/bin/env bash
# Scripted reproducer for Peter Porker TUI bug
# This uses agentplay to deterministically play Peter Porker, attack with it,
# and trigger the disappearance bug

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

echo "=========================================="
echo "Peter Porker Bug Reproducer (Scripted)"
echo "=========================================="
echo ""
echo "This script will:"
echo "  1. Start a game with Peter Porker in hand"
echo "  2. Play Peter Porker"
echo "  3. Attack with Peter Porker"
echo "  4. Observe the bug with debug logging"
echo ""
echo "Starting..."
echo ""

# Clean up any previous session
rm -rf agentplay/current.game 2>/dev/null || true

# Start a new game with --p1-draw to ensure Peter Porker is in hand
echo "=== Starting game with Peter Porker in P1's hand ==="
RUST_LOG=tui=debug,zone=debug,token=debug \
./agentplay/start_game.sh \
    decks/peter_porker_test.dck \
    decks/peter_porker_test.dck \
    --p1-draw="Forest;Spider-Ham, Peter Porker;Forest;Forest;Forest;Forest;Forest"

echo ""
echo "=== P1 Turn 1: Play Forest, pass ==="
./agentplay/continue_game.sh "play forest;*"

echo ""
echo "=== P2 Turn 1: Play Forest, pass ==="
./agentplay/continue_game.sh "*;play forest;*"

echo ""
echo "=== P1 Turn 2: Play Forest, cast Peter Porker ==="
./agentplay/continue_game.sh "*;play forest;cast spider-ham;*"

echo ""
echo "=== P2 Turn 2: Pass ==="
./agentplay/continue_game.sh "*;*"

echo ""
echo "=== P1 Turn 3: Attack with Peter Porker ==="
./agentplay/continue_game.sh "*;attack peter;*"

echo ""
echo "=========================================="
echo "Bug should now be visible!"
echo ""
echo "Check the game snapshot:"
cat agentplay/current.game/game.snapshot | jq -r '.game_state.zones.battlefield[] | select(.name | contains("Peter")) | "\(.name) (id=\(.id))"'
echo ""
echo "Debug logs saved to: mtg_forge.log"
echo ""
echo "Useful analysis commands:"
echo "  grep 'Peter Porker' mtg_forge.log | tail -20"
echo "  grep 'Categorizing.*Peter' mtg_forge.log"
echo "  grep 'draw_battlefield' mtg_forge.log | grep 'player 0'"
echo "=========================================="
