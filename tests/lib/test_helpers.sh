#!/usr/bin/env bash
# Shared helpers for E2E test scripts
#
# Usage: source "$SCRIPT_DIR/lib/test_helpers.sh"
#
# This file provides common functionality for all E2E test scripts:
# - Path resolution (WORKSPACE_ROOT)
# - Release binary building (ensure_mtg_binary)
# - Consistent invocation patterns via run_mtg()
#
# Environment variables:
# - MTG_NETWORK_MODE=1: Run tui commands through network stack (server + 2 clients)
#   This tests the networking layer with existing test scripts.
#   NOTE: Some options are not supported in network mode (--seed, --stop-on-choice, etc.)
#   The network script will exit with code 2 if unsupported options are used.

# Get absolute path to workspace root (script is in tests/, helpers in tests/lib/)
# Note: Caller must set SCRIPT_DIR before sourcing this file:
#   SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
#
# If SCRIPT_DIR is not set, attempt to derive it from BASH_SOURCE
if [ -z "$SCRIPT_DIR" ]; then
    # When run as a test by the harness (not sourced), use the lib directory as reference
    SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
    # Go up one level from lib/ to tests/
    export WORKSPACE_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
else
    # Normal case: SCRIPT_DIR set by caller (test script in tests/)
    export WORKSPACE_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
fi

# Ensure release binary exists and is up-to-date
# This builds the binary once per test script invocation
# Optional: Call this explicitly at the start of each test for consistent timing
ensure_mtg_binary() {
    cd "$WORKSPACE_ROOT"

    # Always build release binary at start of each test
    # This ensures tests use latest code and provides consistent timing
    # Include network feature for client/server functionality
    echo "Building release binary..."
    cargo build --release --bin mtg --features network
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
#
# If ensure_mtg_binary() was called first, uses the pre-built binary.
# Otherwise, uses cargo run --release --bin mtg (builds on first invocation).
#
# If MTG_NETWORK_MODE=1 and first argument is "tui", will attempt to use
# the network drop-in replacement script. If unsupported options are used,
# the network script exits with code 2 and run_mtg falls back to local mode.
run_mtg() {
    local first_arg="${1:-}"

    # Check if network mode is enabled and this is a tui command
    if [ "${MTG_NETWORK_MODE:-}" = "1" ] && [ "$first_arg" = "tui" ]; then
        # Try network mode first
        local network_script="$WORKSPACE_ROOT/scripts/mtg_tui_networked.py"
        if [ -f "$network_script" ]; then
            # Remove 'tui' from args and pass rest to network script
            shift
            echo ">>> [NETWORK MODE] mtg_tui_networked.py $@"

            # Set up environment for network script
            export MTG_BINARY="${MTG_BIN:-$WORKSPACE_ROOT/target/release/mtg}"
            export MTG_CARDSFOLDER="${MTG_CARDSFOLDER:-$WORKSPACE_ROOT/cardsfolder}"

            # Run network script
            local exit_code=0
            python3 "$network_script" "$@" || exit_code=$?

            # Check if network mode is not supported for these options
            if [ $exit_code -eq 2 ]; then
                echo "[NETWORK MODE] Unsupported options, falling back to local mode"
                # Put 'tui' back and run locally
                echo ">>> mtg tui $@"
                if [ -n "$MTG_BIN" ] && [ -f "$MTG_BIN" ]; then
                    "$MTG_BIN" tui "$@"
                else
                    cargo run --release --bin mtg -- tui "$@"
                fi
                return $?
            fi
            return $exit_code
        else
            echo "[NETWORK MODE] Network script not found, using local mode"
        fi
    fi

    # Local mode (default)
    echo ">>> mtg $@"
    if [ -n "$MTG_BIN" ] && [ -f "$MTG_BIN" ]; then
        # Use pre-built binary if available
        "$MTG_BIN" "$@"
    else
        # Fall back to cargo run (builds if needed)
        cargo run --release --bin mtg -- "$@"
    fi
}
