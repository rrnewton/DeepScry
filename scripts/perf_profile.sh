#!/bin/bash
# Run perf-based profiling on MTG Forge benchmarks
#
# Usage: ./scripts/perf_profile.sh [mode] [benchmark_name]
#   mode: stat (hardware counters) or record (detailed profiling) - default: stat
#   benchmark_name: Optional specific benchmark to profile
#
# Examples:
#   ./scripts/perf_profile.sh stat                    # Run perf stat on all benchmarks
#   ./scripts/perf_profile.sh stat fresh              # Run perf stat on fresh benchmark
#   ./scripts/perf_profile.sh record fresh            # Record detailed profile of fresh benchmark
#
# Requirements:
#   - perf tool must be installed
#   - Container/system must have perf_event_paranoid set to 0 or -1
#   - CAP_PERFMON or CAP_SYS_ADMIN capabilities may be required

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# Get CPU name and normalize it for use in directory paths
get_cpu_name() {
    local cpu_name=$(grep "model name" /proc/cpuinfo | head -1 | cut -d':' -f2 | sed 's/^[ \t]*//')
    echo "$cpu_name" | sed 's/ /_/g' | sed 's/[^a-zA-Z0-9_-]//g'
}

CPU_NAME=$(get_cpu_name)
RESULTS_DIR="$REPO_ROOT/experiment_results/$CPU_NAME"
PERF_DIR="$RESULTS_DIR/perf"

# Ensure results directory exists
mkdir -p "$PERF_DIR"

# Get git metadata
GIT_COMMIT_SHORT=$(git rev-parse --short HEAD)
GIT_DEPTH=$(git rev-list --count HEAD)
TIMESTAMP=$(date +"%Y%m%d_%H%M%S")

# Parse arguments
MODE="${1:-stat}"
BENCH_FILTER="${2:-}"

if [[ "$MODE" != "stat" && "$MODE" != "record" ]]; then
    echo "Error: Invalid mode '$MODE'. Must be 'stat' or 'record'"
    exit 1
fi

echo "=== Perf Profiling ==="
echo "CPU: $CPU_NAME"
echo "Mode: $MODE"
echo "Git commit: $GIT_COMMIT_SHORT (depth: $GIT_DEPTH)"
echo "Results directory: $PERF_DIR"
echo ""

# Check perf availability
if ! command -v perf &> /dev/null; then
    echo "Error: perf command not found. Please install perf."
    exit 1
fi

echo "Perf version: $(perf --version)"
PARANOID=$(cat /proc/sys/kernel/perf_event_paranoid 2>/dev/null || echo "unknown")
echo "perf_event_paranoid: $PARANOID"
echo ""

if [[ "$PARANOID" != "0" && "$PARANOID" != "-1" && "$PARANOID" != "unknown" ]]; then
    echo "Warning: perf_event_paranoid is $PARANOID. You may have limited profiling capabilities."
    echo "For best results, set it to 0 or -1 (requires root/container privileges)."
    echo ""
fi

cd "$REPO_ROOT"

# Build benchmarks in release mode first
echo "Building benchmarks in release mode..."
cargo build --release --bench game_benchmark
echo ""

if [ "$MODE" = "stat" ]; then
    # Run perf stat to collect hardware performance counters
    OUTPUT_FILE="$PERF_DIR/perf_stat_${TIMESTAMP}_#${GIT_DEPTH}.txt"

    echo "Running perf stat (collecting hardware counters)..."
    echo "Output will be saved to: $OUTPUT_FILE"
    echo ""

    # Define comprehensive event list
    # Note: Some events may not be available on all CPUs
    EVENTS="cycles,instructions,cache-references,cache-misses"
    EVENTS+=",branches,branch-misses"
    EVENTS+=",L1-dcache-loads,L1-dcache-load-misses"
    EVENTS+=",L1-icache-loads,L1-icache-load-misses"
    EVENTS+=",LLC-loads,LLC-load-misses"
    EVENTS+=",dTLB-loads,dTLB-load-misses"

    # Write header to output file
    cat > "$OUTPUT_FILE" << EOF
================================================================================
MTG Forge-rs Perf Stat Results
================================================================================
CPU: $CPU_NAME
Timestamp: $(date +"%Y-%m-%d %H:%M:%S %Z")
Git Commit: $GIT_COMMIT_SHORT
Git Depth: $GIT_DEPTH
Perf Version: $(perf --version)
perf_event_paranoid: $PARANOID
================================================================================

EOF

    # Run benchmark with perf stat
    if [ -n "$BENCH_FILTER" ]; then
        echo "Profiling specific benchmark: $BENCH_FILTER"
        perf stat -e "$EVENTS" -d \
            cargo bench --bench game_benchmark "$BENCH_FILTER" \
            2>&1 | tee -a "$OUTPUT_FILE"
    else
        echo "Profiling all benchmarks"
        perf stat -e "$EVENTS" -d \
            cargo bench --bench game_benchmark \
            2>&1 | tee -a "$OUTPUT_FILE"
    fi

    echo ""
    echo "=== Perf Stat Complete ==="
    echo "Results saved to: $OUTPUT_FILE"

elif [ "$MODE" = "record" ]; then
    # Run perf record to collect detailed profiling data
    PERF_DATA="$PERF_DIR/perf_${TIMESTAMP}_#${GIT_DEPTH}.data"
    REPORT_FILE="$PERF_DIR/perf_report_${TIMESTAMP}_#${GIT_DEPTH}.txt"

    echo "Running perf record (collecting detailed profile)..."
    echo "Perf data will be saved to: $PERF_DATA"
    echo ""

    # Record with call graph for flamegraph generation
    # -F 999: Sample at 999 Hz (just under 1000 to avoid lockstep sampling)
    # -g: Record call graph (stack traces)
    # --call-graph dwarf: Use DWARF for accurate call graphs
    PERF_OPTS="-F 999 -g --call-graph dwarf"

    if [ -n "$BENCH_FILTER" ]; then
        echo "Recording specific benchmark: $BENCH_FILTER"
        perf record $PERF_OPTS -o "$PERF_DATA" -- \
            cargo bench --bench game_benchmark "$BENCH_FILTER" --profile-time 10
    else
        echo "Recording all benchmarks"
        perf record $PERF_OPTS -o "$PERF_DATA" -- \
            cargo bench --bench game_benchmark --profile-time 10
    fi

    echo ""
    echo "Generating perf report..."
    perf report -i "$PERF_DATA" --stdio > "$REPORT_FILE" 2>&1 || true

    echo ""
    echo "=== Perf Record Complete ==="
    echo "Perf data saved to: $PERF_DATA"
    echo "Report saved to: $REPORT_FILE"
    echo ""
    echo "To view the report:"
    echo "  cat $REPORT_FILE"
    echo "To interactively explore the data:"
    echo "  perf report -i $PERF_DATA"
    echo "To generate a flamegraph (requires flamegraph tools):"
    echo "  perf script -i $PERF_DATA | stackcollapse-perf.pl | flamegraph.pl > flamegraph.svg"
fi

echo ""
echo "Done!"
