#!/usr/bin/env bash
# Shared helpers for E2E test scripts
#
# Usage: source "$SCRIPT_DIR/lib/test_helpers.sh"
#
# This file provides common functionality for all E2E test scripts:
# - Path resolution (WORKSPACE_ROOT)
# - Release binary building (ensure_mtg_binary)
# - Consistent invocation patterns via run_mtg_cargo() and run_mtg_prebuilt()
#
# Environment variables:
# - MTG_NETWORK_MODE=1: Run tui commands through network stack (server + 2 clients)
#   This tests the networking layer with existing test scripts.
#   NOTE: Some options are not supported in network mode (--stop-on-choice, etc.)
#   The network script will exit with code 2 if unsupported options are used.
#
# Functions:
# - ensure_mtg_binary(): Build the release binary (call before run_mtg_prebuilt)
# - run_mtg_cargo(): Always uses cargo run (builds if needed)
# - run_mtg_prebuilt(): Uses pre-built binary (caller must ensure it's built)
#
# Both run_mtg_* functions respect MTG_NETWORK_MODE for 'tui' subcommand.

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
# Call this before using run_mtg_prebuilt()
ensure_mtg_binary() {
    cd "$WORKSPACE_ROOT"

    # Always build release binary at start of each test
    # This ensures tests use latest code and provides consistent timing
    # Include network feature for client/server functionality
    echo "Building release binary..."
    cargo build --release --bin mtg --features network
    echo ""

    # Set MTG_BIN for use in run_mtg_prebuilt
    export MTG_BIN="$WORKSPACE_ROOT/target/release/mtg"

    # Verify binary exists
    if [ ! -f "$MTG_BIN" ]; then
        echo "Error: Failed to build $MTG_BIN"
        exit 1
    fi
}

# Internal helper: Try network mode for tui commands
# Returns 0 if network mode handled the command (success or failure)
# Returns 1 if should fall back to local mode
# Sets NETWORK_EXIT_CODE to the exit code from network mode
_try_network_mode() {
    local first_arg="$1"
    shift

    # Only attempt network mode for 'tui' subcommand
    if [ "$first_arg" != "tui" ]; then
        return 1
    fi

    # Check if network mode is enabled
    if [ "${MTG_NETWORK_MODE:-}" != "1" ]; then
        return 1
    fi

    local network_script="$WORKSPACE_ROOT/scripts/mtg_tui_networked.py"
    if [ ! -f "$network_script" ]; then
        echo "[NETWORK MODE] Network script not found, using local mode"
        return 1
    fi

    # Set up environment for network script
    export MTG_BINARY="${MTG_BIN:-$WORKSPACE_ROOT/target/release/mtg}"
    export MTG_CARDSFOLDER="${MTG_CARDSFOLDER:-$WORKSPACE_ROOT/cardsfolder}"

    echo ">>> [NETWORK MODE] mtg_tui_networked.py $@"

    # Run network script
    NETWORK_EXIT_CODE=0
    python3 "$network_script" "$@" || NETWORK_EXIT_CODE=$?

    # Check if network mode is not supported for these options (exit code 2)
    if [ $NETWORK_EXIT_CODE -eq 2 ]; then
        echo "[NETWORK MODE] Unsupported options, falling back to local mode"
        return 1
    fi

    # Network mode handled it (success or failure)
    return 0
}

# Run mtg using cargo run (always rebuilds if needed)
# Usage: run_mtg_cargo tui deck1.dck deck2.dck --p1 heuristic ...
#
# This always uses cargo run, which ensures the binary is up-to-date.
# Use this when you want guaranteed fresh builds.
#
# Respects MTG_NETWORK_MODE=1 for 'tui' subcommand.
run_mtg_cargo() {
    local first_arg="${1:-}"

    # Try network mode first for tui commands
    if _try_network_mode "$@"; then
        return $NETWORK_EXIT_CODE
    fi

    # Local mode via cargo
    echo ">>> cargo run --release --bin mtg --features network -- $@"
    cd "$WORKSPACE_ROOT"
    cargo run --release --bin mtg --features network -- "$@"
}

# Run mtg using pre-built binary (caller must call ensure_mtg_binary first)
# Usage: run_mtg_prebuilt tui deck1.dck deck2.dck --p1 heuristic ...
#
# This uses the pre-built binary at $MTG_BIN. Caller is responsible for
# calling ensure_mtg_binary() before using this function.
#
# Respects MTG_NETWORK_MODE=1 for 'tui' subcommand.
run_mtg_prebuilt() {
    local first_arg="${1:-}"

    # Try network mode first for tui commands
    if _try_network_mode "$@"; then
        return $NETWORK_EXIT_CODE
    fi

    # Local mode via pre-built binary
    if [ -z "$MTG_BIN" ] || [ ! -f "$MTG_BIN" ]; then
        echo "Error: MTG_BIN not set or binary doesn't exist."
        echo "Call ensure_mtg_binary() before run_mtg_prebuilt()"
        exit 1
    fi

    echo ">>> $MTG_BIN $@"
    "$MTG_BIN" "$@"
}

# Legacy alias for backwards compatibility
# DEPRECATED: Use run_mtg_prebuilt() or run_mtg_cargo() instead
run_mtg() {
    # If MTG_BIN is set and exists, use prebuilt; otherwise use cargo
    if [ -n "$MTG_BIN" ] && [ -f "$MTG_BIN" ]; then
        run_mtg_prebuilt "$@"
    else
        run_mtg_cargo "$@"
    fi
}

# Run mtg with a timeout, compatible with network mode
# Usage: run_mtg_with_timeout TIMEOUT_SECONDS tui deck1.dck deck2.dck --p1 heuristic ...
#
# This runs run_mtg_prebuilt in a background process with timeout handling.
# Works with shell functions (unlike `timeout cmd` which only works with executables).
#
# For piped input, use: echo "input" | run_mtg_with_timeout_stdin TIMEOUT ...
#
# Returns:
#   0 - Command completed successfully
#   124 - Command timed out
#   Other - Command failed with that exit code
run_mtg_with_timeout() {
    local timeout_secs="$1"
    shift

    # Run in background and capture PID
    run_mtg_prebuilt "$@" &
    local cmd_pid=$!

    # Wait with timeout
    local elapsed=0
    while kill -0 $cmd_pid 2>/dev/null; do
        if [ $elapsed -ge $timeout_secs ]; then
            # Timeout - kill process group
            kill -TERM $cmd_pid 2>/dev/null
            sleep 0.5
            kill -KILL $cmd_pid 2>/dev/null
            wait $cmd_pid 2>/dev/null
            return 124
        fi
        sleep 1
        elapsed=$((elapsed + 1))
    done

    # Get exit status
    wait $cmd_pid
    return $?
}

# Run mtg with a timeout and stdin input, compatible with network mode
# Usage: run_mtg_with_timeout_stdin TIMEOUT_SECONDS "input_data" tui deck1.dck ...
#
# Like run_mtg_with_timeout but accepts stdin data as second argument.
run_mtg_with_timeout_stdin() {
    local timeout_secs="$1"
    local stdin_data="$2"
    shift 2

    # Run in background with piped input
    echo -e "$stdin_data" | run_mtg_prebuilt "$@" &
    local cmd_pid=$!

    # Wait with timeout
    local elapsed=0
    while kill -0 $cmd_pid 2>/dev/null; do
        if [ $elapsed -ge $timeout_secs ]; then
            # Timeout - kill process group
            kill -TERM $cmd_pid 2>/dev/null
            sleep 0.5
            kill -KILL $cmd_pid 2>/dev/null
            wait $cmd_pid 2>/dev/null
            return 124
        fi
        sleep 1
        elapsed=$((elapsed + 1))
    done

    # Get exit status
    wait $cmd_pid
    return $?
}
