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

    # Default binary location for the release+network build.
    export MTG_BIN="${MTG_BIN:-$WORKSPACE_ROOT/target/release/mtg}"

    # Fast path: reuse a binary that was already built by the caller. CI builds
    # `mtg --release --features network` ONCE in the "Build release binary"
    # step, then runs the whole shell-script test binary; having each of the 26
    # scripts re-invoke `cargo build` was the single biggest contributor to the
    # ~1046s serial shell-test time (mtg-578). When MTG_REUSE_PREBUILT=1 is set
    # (CI does this) and the binary exists, skip the rebuild entirely. Local
    # `make validate` runs do NOT set the flag, so they keep the always-fresh
    # behaviour below.
    if [ "${MTG_REUSE_PREBUILT:-}" = "1" ] && [ -x "$MTG_BIN" ]; then
        echo "Reusing pre-built release binary (MTG_REUSE_PREBUILT=1): $MTG_BIN"
        echo ""
        return 0
    fi

    # mtg-717 build-once: MTG_REUSE_PREBUILT=1 PROMISES a prebuilt binary (CI's
    # build-once `--use-prebuilt` shards set it after downloading the mtg-bin
    # artifact; local `make validate` builds it once up front via the runner's
    # build.mtg-release step and also sets it). If the flag is set but the binary
    # is ABSENT, the artifact handoff is broken — HARD-FAIL rather than silently
    # resurrecting the ~1h cold `cargo build` inside the shard (that would defeat
    # build-once and violate the project's fatal-on-missing-prereq rule).
    if [ "${MTG_REUSE_PREBUILT:-}" = "1" ]; then
        echo "ERROR: MTG_REUSE_PREBUILT=1 but prebuilt binary is missing/not executable: $MTG_BIN" >&2
        echo "       Refusing to silently cold-rebuild. The mtg-bin artifact handoff is broken." >&2
        exit 1
    fi

    # Always build release binary at start of each test
    # This ensures tests use latest code and provides consistent timing
    # Include network feature for client/server functionality
    echo "Building release binary..."
    cargo build --release --bin mtg --features network
    echo ""

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

# Internal core: run a shell command in its OWN process group with a timeout,
# and on timeout KILL THE ENTIRE PROCESS GROUP so no grandchild survives.
#
# Why this exists (mtg-615): the old implementation backgrounded a shell
# FUNCTION (`run_mtg_prebuilt "$@" &`) and, on timeout, only signalled that
# function's subshell PID. The real `mtg` binary is a GRANDCHILD; killing the
# subshell reparents it to init (ppid=1) where it kept running — and a
# `mtg tui` controller reading stdin EOF busy-loops at ~100% CPU, leaving an
# orphan spinning a full core for minutes. Polluting concurrent/subsequent
# tests with false timeout-flakes.
#
# Fix: launch the command tree under `setsid` so it becomes the leader of a
# brand-new process group (PGID == the setsid child's PID). Poll for the
# timeout; when it fires, send SIGTERM then SIGKILL to the NEGATIVE pid
# (`kill -- -PGID`), which targets every process in the group — child, the
# `mtg` binary, and anything it spawned. Nothing is left orphaned.
#
# Args:
#   $1            - timeout in seconds
#   $2            - stdin data to pipe to the command, or empty string for none
#   $3..          - the command + args to run (a shell function such as
#                   run_mtg_prebuilt, kept for network-mode compatibility)
# Returns:
#   0   - completed successfully
#   124 - timed out (group killed)
#   N   - command's own exit code
_run_with_pgroup_timeout() {
    local timeout_secs="$1"
    local stdin_data="$2"
    shift 2

    # Launch the command as a new process-group leader via setsid, backgrounded.
    # We re-enter this same script's functions through `bash -c`, so export the
    # helper functions and the env they rely on into the setsid child.
    export -f run_mtg_prebuilt run_mtg_cargo run_mtg _try_network_mode
    export MTG_BIN MTG_NETWORK_MODE MTG_CARDSFOLDER WORKSPACE_ROOT

    # Pass the command through the positional list to an inline `bash -c` that
    # invokes it via `"$@"`. The first positional ($1 inside bash -c) is the
    # exported shell-function name (e.g. run_mtg_prebuilt); bash resolves it
    # against the exported function table. We DON'T `exec` it because it's a
    # function, not an external binary. The literal "pgrp" is bash -c's $0
    # (just a label for error messages).
    if [ -n "$stdin_data" ]; then
        setsid bash -c '"$@"' pgrp "$@" <<<"$(printf '%b' "$stdin_data")" &
    else
        setsid bash -c '"$@"' pgrp "$@" &
    fi
    local leader_pid=$!
    # With setsid, the child is the group leader, so PGID == leader_pid.
    local pgid="$leader_pid"

    # Poll for completion or timeout.
    local elapsed=0
    while kill -0 "$leader_pid" 2>/dev/null; do
        if [ "$elapsed" -ge "$timeout_secs" ]; then
            # Timeout: kill the WHOLE process group (negative pid).
            kill -TERM -- "-$pgid" 2>/dev/null
            sleep 0.5
            kill -KILL -- "-$pgid" 2>/dev/null
            wait "$leader_pid" 2>/dev/null
            return 124
        fi
        sleep 1
        elapsed=$((elapsed + 1))
    done

    # Normal exit: reap and propagate the command's exit code. Also sweep the
    # group with SIGKILL in case the command forked anything that outlived it
    # (belt-and-suspenders: a clean exit normally leaves nothing here).
    wait "$leader_pid"
    local rc=$?
    kill -KILL -- "-$pgid" 2>/dev/null
    return $rc
}

# Run mtg with a timeout, compatible with network mode.
# Usage: run_mtg_with_timeout TIMEOUT_SECONDS tui deck1.dck deck2.dck --p1 heuristic ...
#
# Runs run_mtg_prebuilt in its own process group with timeout handling, so a
# timeout kills the entire group (no orphaned `mtg` grandchild — mtg-615).
# Works with shell functions (unlike bare `timeout cmd`).
#
# For explicit piped input use run_mtg_with_timeout_stdin, or pipe into this
# function directly (stdin is inherited by the process group).
#
# Returns:
#   0 - Command completed successfully
#   124 - Command timed out
#   Other - Command failed with that exit code
run_mtg_with_timeout() {
    local timeout_secs="$1"
    shift
    _run_with_pgroup_timeout "$timeout_secs" "" run_mtg_prebuilt "$@"
}

# Run mtg with a timeout and stdin input, compatible with network mode.
# Usage: run_mtg_with_timeout_stdin TIMEOUT_SECONDS "input_data" tui deck1.dck ...
#
# Like run_mtg_with_timeout but accepts stdin data as second argument. Uses the
# same process-group kill semantics so no orphan survives a timeout (mtg-615).
run_mtg_with_timeout_stdin() {
    local timeout_secs="$1"
    local stdin_data="$2"
    shift 2
    _run_with_pgroup_timeout "$timeout_secs" "$stdin_data" run_mtg_prebuilt "$@"
}
