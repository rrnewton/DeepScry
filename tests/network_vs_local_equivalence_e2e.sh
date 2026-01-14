#!/usr/bin/env bash
# E2E test: Network vs Local game equivalence
#
# This test runs the SAME game in two modes in PARALLEL:
# 1. Local mode: Single process with two heuristic AIs
# 2. Network mode: Server + two client processes with heuristic AIs
#
# Both use identical seeds, decks, and controller settings. The test verifies:
# - Both games complete successfully
# - Final action_count matches between network and local
# - GAMELOG entries match (deterministic gameplay)
#
# This test uses pre-built binaries and runs both games in parallel to minimize
# impact on validation time.
#
# Usage:
#   ./tests/network_vs_local_equivalence_e2e.sh [options]
#
# Options:
#   --deck1 PATH        First player's deck (default: avatar draft deck)
#   --deck2 PATH        Second player's deck (default: avatar draft deck)
#   --seed N            Game seed (default: 3)
#   --controller TYPE   Controller type (default: heuristic)

set -euo pipefail

# Get script directory and source shared test helpers
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"
source "$SCRIPT_DIR/lib/network_vs_local_common.sh"

echo "=== Network vs Local Game Equivalence E2E Test ==="
echo

# Use pre-built binary if available, otherwise build
if [ -f "$WORKSPACE_ROOT/target/release/mtg" ]; then
    export MTG_BIN="$WORKSPACE_ROOT/target/release/mtg"
    echo "Using pre-built binary: $MTG_BIN"
else
    echo "Building release binary..."
    ensure_mtg_binary
fi

# Verify binary has network feature
if ! "$MTG_BIN" server --help >/dev/null 2>&1; then
    echo "Warning: Binary doesn't support network mode, rebuilding..."
    ensure_mtg_binary
fi

cd "$WORKSPACE_ROOT"

# Check for required files
if [[ ! -d "$WORKSPACE_ROOT/cardsfolder" ]]; then
    echo "Warning: cardsfolder not found, skipping test"
    exit 0
fi

# Default decks (avatar draft decks - proven to work)
DEFAULT_DECK1="$WORKSPACE_ROOT/decks/booster_draft/avatar/ryan_avatar_draft.dck"
DEFAULT_DECK2="$WORKSPACE_ROOT/decks/booster_draft/avatar/gabriel_avatar_draft.dck"

# Default configuration
DECK1="$DEFAULT_DECK1"
DECK2="$DEFAULT_DECK2"
SEED=3
CONTROLLER_SEED=3
CONTROLLER_TYPE="heuristic"
P1_NAME="Ryan"
P2_NAME="Gabriel"

# Parse command line arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --deck1)
            DECK1="$2"
            shift 2
            ;;
        --deck2)
            DECK2="$2"
            shift 2
            ;;
        --seed)
            SEED="$2"
            shift 2
            ;;
        --controller)
            CONTROLLER_TYPE="$2"
            shift 2
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

# Validate decks exist
if [[ ! -f "$DECK1" ]]; then
    echo "Error: Deck not found: $DECK1"
    exit 1
fi
if [[ ! -f "$DECK2" ]]; then
    echo "Error: Deck not found: $DECK2"
    exit 1
fi

# Run the common test logic
run_network_vs_local_test \
    --deck1 "$DECK1" \
    --deck2 "$DECK2" \
    --seed "$SEED" \
    --controller-seed "$CONTROLLER_SEED" \
    --controller "$CONTROLLER_TYPE" \
    --p1-name "$P1_NAME" \
    --p2-name "$P2_NAME" \
    --timeout 120
