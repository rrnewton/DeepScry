#!/usr/bin/env bash
# Shared helpers for E2E test scripts
#
# Usage: source "$SCRIPT_DIR/test_helpers.sh"
#
# This file provides common functionality for all E2E test scripts:
# - Path resolution (WORKSPACE_ROOT)
# - Release binary building (ensure_mtg_binary)
# - Consistent invocation patterns

# Detect if being run directly vs sourced
# When run directly (e.g. by test harness), exit successfully
if [ "${BASH_SOURCE[0]}" = "${0}" ]; then
    echo "test_helpers.sh: This file is meant to be sourced, not executed directly."
    echo "It provides helper functions for other test scripts."
    exit 0
fi

# Get absolute path to workspace root (script is in tests/)
# Note: Caller must set SCRIPT_DIR before sourcing this file:
#   SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
if [ -z "$SCRIPT_DIR" ]; then
    echo "Error: SCRIPT_DIR must be set before sourcing test_helpers.sh"
    exit 1
fi

export WORKSPACE_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# Ensure release binary exists and is up-to-date
# This builds the binary once per test script invocation
ensure_mtg_binary() {
    cd "$WORKSPACE_ROOT"

    # Always build release binary at start of each test
    # This ensures tests use latest code and provides consistent timing
    echo "Building release binary..."
    cargo build --release --bin mtg
    echo ""

    # Set MTG_BIN for use in tests
    export MTG_BIN="$WORKSPACE_ROOT/target/release/mtg"

    # Verify binary exists
    if [ ! -f "$MTG_BIN" ]; then
        echo "Error: Failed to build $MTG_BIN"
        exit 1
    fi
}

# Run mtg with given arguments
# Usage: run_mtg tui deck1.dck deck2.dck --p1 heuristic ...
run_mtg() {
    "$MTG_BIN" "$@"
}
