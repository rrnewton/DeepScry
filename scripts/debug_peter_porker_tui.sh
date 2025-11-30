#!/usr/bin/env bash
# Debug script for Peter Porker TUI rendering bug
# This script runs the Fancy TUI with debug logging enabled
# Logs will be written to mtg_forge.log (automatically by main.rs for Fancy TUI mode)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

echo "=========================================="
echo "Peter Porker TUI Debug Session"
echo "=========================================="
echo ""
echo "This will start an interactive Fancy TUI game."
echo "Debug logs will be written to: mtg_forge.log"
echo ""
echo "Peter Porker is GUARANTEED in your opening hand!"
echo ""
echo "To reproduce the bug:"
echo "  Turn 1: Play a Forest, pass"
echo "  Turn 2: Play a Forest, cast Spider-Ham Peter Porker (1G)"
echo "  Turn 3: Attack with Peter Porker"
echo "  -> BUG: Peter Porker disappears from Creatures section"
echo "  Exit the game (Ctrl+C)"
echo "  Check mtg_forge.log for debug output"
echo ""
echo "Starting in 3 seconds..."
sleep 3

# Run with debug logging for TUI, zone, token, and SBA (state-based actions) events
# Use --p1-draw to GUARANTEE Peter Porker is in opening hand
RUST_LOG=tui=debug,zone=debug,token=debug,sba=debug cargo run --release --bin mtg -- tui \
    decks/peter_porker_test.dck \
    decks/peter_porker_test.dck \
    --p1=fancy \
    --p2=heuristic \
    --p1-draw="Forest;Spider-Ham, Peter Porker;Forest;Forest;Forest;Forest;Forest" \
    --seed=42

echo ""
echo "=========================================="
echo "Session complete!"
echo "Debug logs saved to: mtg_forge.log"
echo ""
echo "Useful grep commands:"
echo "  grep 'Peter Porker' mtg_forge.log"
echo "  grep 'Categorizing.*Peter' mtg_forge.log"
echo "  grep 'draw_battlefield' mtg_forge.log | head -50"
echo "=========================================="
