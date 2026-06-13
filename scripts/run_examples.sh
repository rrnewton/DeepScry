#!/bin/bash
# Build all examples ONCE, then run pre-built binaries in parallel using xargs.
#
# Build-once approach (validate-hygiene principle): a single
# `cargo build --examples` compiles the shared mtg-engine lib once and links
# all example binaries in one pass.  Subsequent runs on an already-built tree
# skip compilation entirely.  The old approach ran `cargo run --example <X>`
# per-example inside GNU parallel --jobs 4; on CI (ubuntu-latest) GNU parallel
# is not installed, so it silently fell back to sequential execution and easily
# exceeded the 600 s per-step timeout (17 games sequentially ~441 s locally,
# more on a slow 2-vCPU runner).  Fix: build once, then run pre-built binaries
# with xargs -P$(nproc) — no external tool dependency, scales to core count.
#
# Usage:
#   ./run_examples.sh [--sequential]
#
# Options:
#   --sequential    Force sequential execution (useful for debugging/log clarity)

set -euo pipefail

FORCE_SEQUENTIAL=false
while [[ $# -gt 0 ]]; do
    case $1 in
        --sequential)
            FORCE_SEQUENTIAL=true
            shift
            ;;
        *)
            echo "Unknown option: $1"
            echo "Usage: $0 [--sequential]"
            exit 1
            ;;
    esac
done

# ---------------------------------------------------------------------------
# Step 1: build all examples once (shared lib compiled exactly once).
# ---------------------------------------------------------------------------
echo "=== Building all examples (one pass) ==="
TMPJSON=$(mktemp)
RESULTS_DIR=$(mktemp -d)
trap 'rm -f "$TMPJSON"; rm -rf "$RESULTS_DIR"' EXIT

cargo build -p mtg-engine --features network --examples \
    --message-format json >"$TMPJSON" 2>&1 || {
    echo "ERROR: cargo build --examples failed"
    cat "$TMPJSON"
    exit 1
}

# ---------------------------------------------------------------------------
# Step 2: collect the list of example executables from cargo JSON output.
# Exclude lobby_probe (requires a live server).
# ---------------------------------------------------------------------------
EXAMPLES_LIST=$(python3 - "$TMPJSON" <<'PYEOF'
import json, sys

with open(sys.argv[1]) as f:
    for line in f:
        line = line.strip()
        if not line:
            continue
        try:
            msg = json.loads(line)
        except json.JSONDecodeError:
            continue
        if msg.get("reason") == "compiler-artifact" and msg.get("executable"):
            exe = msg["executable"]
            name = exe.rsplit("/", 1)[-1]
            if name != "lobby_probe":
                print(exe)
PYEOF
)

if [ -z "$EXAMPLES_LIST" ]; then
    echo "ERROR: No example executables found in cargo build output!"
    exit 1
fi

TOTAL=$(echo "$EXAMPLES_LIST" | wc -l)
echo ""
echo "Built $TOTAL examples:"
echo "$EXAMPLES_LIST" | while IFS= read -r exe; do printf '  %s\n' "$(basename "$exe")"; done
echo ""

# ---------------------------------------------------------------------------
# Step 3: run examples in parallel (or sequentially with --sequential).
# run_one_example <exe_path> <results_dir>
# ---------------------------------------------------------------------------
run_one_example() {
    local exe="$1"
    local results_dir="$2"
    local name
    name="$(basename "$exe")"
    local log="$results_dir/$name.log"

    {
        echo "----------------------------------------"
        echo "Running example: $name"
        echo "----------------------------------------"
    } > "$log"

    if "$exe" >> "$log" 2>&1; then
        echo "" >> "$log"
        echo "PASSED: $name" >> "$log"
        printf '.'
        return 0
    else
        echo "" >> "$log"
        echo "FAILED: $name" >> "$log"
        printf '\n'
        echo "FAILED: $name"
        cat "$log"
        return 1
    fi
}
export -f run_one_example

if [ "$FORCE_SEQUENTIAL" = true ]; then
    echo "INFO: --sequential flag specified, running sequentially"
    echo ""
    PASSED=0
    FAILED=0
    while IFS= read -r exe; do
        if run_one_example "$exe" "$RESULTS_DIR"; then
            PASSED=$((PASSED + 1))
        else
            FAILED=$((FAILED + 1))
        fi
    done <<< "$EXAMPLES_LIST"
    echo ""
    echo ""
    echo "========================================"
    echo "Summary: $PASSED/$TOTAL examples passed"
    echo "========================================"
    if [ "$FAILED" -gt 0 ]; then
        exit 1
    fi
    echo ""
    echo "All examples passed!"
    exit 0
fi

# Parallel mode via xargs -P$(nproc) — no GNU parallel dependency, scales to
# all available cores.  Each invocation writes to its own log file so output
# does not interleave.
JOBS=$(nproc)
echo "=== Running $TOTAL examples in parallel (-j$JOBS) ==="
echo ""

XARGS_RC=0
echo "$EXAMPLES_LIST" | \
    xargs -d '\n' -P "$JOBS" -I {} \
    bash -c 'run_one_example "$@"' _ {} "$RESULTS_DIR" \
    || XARGS_RC=$?

echo ""
echo ""

# Tally results from per-example log files.
PASSED=0
FAILED=0
FAILED_NAMES=""
while IFS= read -r exe; do
    name="$(basename "$exe")"
    log="$RESULTS_DIR/$name.log"
    if [ -f "$log" ] && grep -q "^PASSED:" "$log"; then
        PASSED=$((PASSED + 1))
    else
        FAILED=$((FAILED + 1))
        FAILED_NAMES="$FAILED_NAMES\n  - $name"
    fi
done <<< "$EXAMPLES_LIST"

echo "========================================"
echo "Summary: $PASSED/$TOTAL examples passed"
echo "========================================"

if [ "$FAILED" -gt 0 ]; then
    echo ""
    printf "Failed examples:%b\n" "$FAILED_NAMES"
    exit 1
fi

echo ""
echo "All examples passed!"
exit 0
