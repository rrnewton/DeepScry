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
echo "To reproduce the bug:"
echo "  1. Play Spider-Ham, Peter Porker"
echo "  2. Attack with Peter Porker"
echo "  3. Observe if Peter Porker disappears from Creatures section"
echo "  4. Exit the game"
echo "  5. Check mtg_forge.log for debug output"
echo ""
echo "Starting in 3 seconds..."
sleep 3

# Run with debug logging for TUI, zone, and token events
RUST_LOG=tui=debug,zone=debug,token=debug cargo run --release --bin mtg -- tui \
    decks/ryan_spiderman_draft.dck \
    decks/julian_spiderman_draft.dck \
    --p1=fancy \
    --p2=heuristic \
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
