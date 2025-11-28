#!/bin/bash
# Periodically run benchmarks when git depth advances by 5+ commits
#
# This is a thin wrapper around run_benchmark.sh that only runs if
# we've advanced 5+ commits since the last recorded benchmark.

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Get CPU name for results directory
get_cpu_name() {
    grep "model name" /proc/cpuinfo | head -1 | cut -d':' -f2 | sed 's/^[ \t]*//' | sed 's/ /_/g' | sed 's/[^a-zA-Z0-9_-]//g'
}

CPU_NAME=$(get_cpu_name)
CSV_FILE="experiment_results/$CPU_NAME/perf_history.csv"
MIN_DEPTH_DELTA=5

# Get current git depth
current_depth=$(git rev-list --count HEAD)

# Get last recorded git depth from CSV (0 if file doesn't exist)
if [ -f "$CSV_FILE" ]; then
    last_depth=$(tail -n +2 "$CSV_FILE" | tail -1 | cut -d',' -f3)
    if [ -z "$last_depth" ] || ! [[ "$last_depth" =~ ^[0-9]+$ ]]; then
        last_depth=0
    fi
else
    last_depth=0
fi

depth_delta=$((current_depth - last_depth))

echo "Current git depth: $current_depth"
echo "Last recorded depth: $last_depth"
echo "Depth delta: $depth_delta (minimum: $MIN_DEPTH_DELTA)"

if [ $depth_delta -lt $MIN_DEPTH_DELTA ]; then
    echo "Skipping benchmarks - need $((MIN_DEPTH_DELTA - depth_delta)) more commits"
    exit 0
fi

echo "Running benchmarks..."
exec "$SCRIPT_DIR/run_benchmark.sh" "$@"
