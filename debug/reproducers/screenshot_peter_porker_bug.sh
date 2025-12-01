#!/usr/bin/env bash
# Script to reproduce Peter Porker bug with TUI screenshots
# Uses fancy-fixed controller to automatically capture screenshots at each decision point

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$REPO_ROOT"

echo "=========================================="
echo "Peter Porker Bug Reproducer (with Screenshots)"
echo "=========================================="
echo ""
echo "This will:"
echo "  1. Start a game with Peter Porker in hand"
echo "  2. Play Peter Porker"
echo "  3. Attack with Peter Porker"
echo "  4. Capture TUI screenshots at each step"
echo ""
echo "Starting..."
echo ""

# Clean up old screenshots
rm -rf screenshots/
mkdir -p screenshots/

# Define the scripted inputs to reproduce the bug
# Format: semicolon-separated commands
# * = pass priority/wildcard
# play forest = play a Forest land
# cast spider-ham = cast Spider-Ham, Peter Porker
# attack spider-ham = attack with Spider-Ham (matches "Spider-Ham, Peter Porker")
#
# Turn sequence:
# T1: P1 plays forest, passes
# T2: P2 plays forest (heuristic), P1 passes
# T3: P1 plays forest, casts spider-ham, passes
# T4: P2 passes (heuristic), P1 passes
# T5: P1 attacks with spider-ham
P1_SCRIPT="play forest;*;*;play forest;cast spider-ham;*;*;attack spider-ham;*"
P2_SCRIPT="*"

# Terminal size for screenshots (use small size to reproduce rendering bug)
# Default large size: 240x60 (shows everything correctly)
# Small size that triggers bug: 120x30
SCREENSHOT_WIDTH="${SCREENSHOT_WIDTH:-120}"
SCREENSHOT_HEIGHT="${SCREENSHOT_HEIGHT:-30}"

echo "=== Running game with fancy-fixed controller ==="
echo "Screenshots will be saved to: screenshots/"
echo "Terminal size: ${SCREENSHOT_WIDTH}x${SCREENSHOT_HEIGHT}"
echo ""

# Run with fancy-fixed controller which captures screenshots
RUST_LOG=zone=debug,token=debug,sba=debug cargo run --release --bin mtg -- tui \
    decks/peter_porker_test.dck \
    decks/peter_porker_test.dck \
    --p1=fancy-fixed \
    --p2=heuristic \
    --p1-fixed-inputs="$P1_SCRIPT" \
    --p1-draw="Forest;Spider-Ham, Peter Porker;Forest;Forest;Forest;Forest;Forest" \
    --seed=42 \
    --snapshot-output="screenshots/final.snapshot" \
    --screenshot-width="$SCREENSHOT_WIDTH" \
    --screenshot-height="$SCREENSHOT_HEIGHT" \
    2>&1 | tee screenshots/game_output.log

echo ""
echo "=========================================="
echo "Screenshots captured!"
echo ""
echo "Check screenshots directory:"
ls -lh screenshots/*.txt 2>/dev/null || echo "  (No screenshot files found)"
echo ""
echo "Game output saved to: screenshots/game_output.log"
echo ""
echo "To view a screenshot:"
echo "  cat screenshots/0001_*.txt"
echo "  cat screenshots/0002_*.txt"
echo "  ..."
echo "=========================================="
