#!/bin/bash
# Run benchmarks and append results to performance history CSV
#
# Usage: ./scripts/run_benchmark.sh [benchmark_name]
#   benchmark_name: Optional specific benchmark to run (default: all)
#
# Examples:
#   ./scripts/run_benchmark.sh              # Run all benchmarks
#   ./scripts/run_benchmark.sh snapshot     # Run only snapshot benchmark
#   ./scripts/run_benchmark.sh fresh        # Run only fresh benchmark

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# Get CPU name and normalize it for use in directory paths
# Extracts from /proc/cpuinfo, replaces spaces with underscores, strips non-alphanumeric except _ and -
get_cpu_name() {
    local cpu_name=$(grep "model name" /proc/cpuinfo | head -1 | cut -d':' -f2 | sed 's/^[ \t]*//')
    # Replace spaces with underscores and remove any characters that aren't alphanumeric, underscore, or dash
    echo "$cpu_name" | sed 's/ /_/g' | sed 's/[^a-zA-Z0-9_-]//g'
}

CPU_NAME=$(get_cpu_name)
RESULTS_DIR="$REPO_ROOT/experiment_results/$CPU_NAME"
HISTORY_FILE="$RESULTS_DIR/perf_history.csv"

# Ensure results directory exists
mkdir -p "$RESULTS_DIR"

# Get git metadata
GIT_COMMIT=$(git rev-parse HEAD)
GIT_COMMIT_SHORT=$(git rev-parse --short HEAD)
GIT_DEPTH=$(git rev-list --count HEAD)
GIT_BRANCH=$(git rev-parse --abbrev-ref HEAD)
GIT_DIRTY=""
if ! git diff-index --quiet HEAD --; then
    GIT_DIRTY="_dirty"
fi

# Get timestamp
TIMESTAMP=$(date -u +"%Y-%m-%dT%H:%M:%SZ")
TIMESTAMP_READABLE=$(date +"%Y-%m-%d %H:%M:%S %Z")
TIMESTAMP_FILE=$(date +"%Y%m%d")

# Create log file path with date and git depth
LOG_FILE="$RESULTS_DIR/benchmark_log_${TIMESTAMP_FILE}_#${GIT_DEPTH}.log"

echo "=== Running Benchmarks ==="
echo "CPU: $CPU_NAME"
echo "Timestamp: $TIMESTAMP_READABLE"
echo "Git commit: $GIT_COMMIT_SHORT (depth: $GIT_DEPTH, branch: $GIT_BRANCH)${GIT_DIRTY}"
echo "Results will be appended to: $HISTORY_FILE"
echo "Full benchmark output will be saved to: $LOG_FILE"
echo ""

# Create CSV header if file doesn't exist
if [ ! -f "$HISTORY_FILE" ]; then
    echo "Creating new performance history file with header..."
    cat > "$HISTORY_FILE" << 'EOF'
timestamp,git_commit,git_depth,git_branch,git_dirty,benchmark_name,seed,num_games,total_turns,total_actions,total_duration_ms,avg_turns_per_game,avg_actions_per_game,avg_duration_ms_per_game,games_per_sec,actions_per_sec,turns_per_sec,actions_per_turn,total_bytes_allocated,total_bytes_deallocated,net_bytes,avg_bytes_per_game,bytes_per_turn,bytes_per_sec
EOF
fi

# Determine which benchmark to run
BENCH_FILTER=""
if [ $# -ge 1 ]; then
    BENCH_FILTER="$1"
    echo "Running specific benchmark: $BENCH_FILTER"
else
    echo "Running all benchmarks"
fi

# Run benchmark and capture output
echo ""
echo "Running cargo bench..."
echo ""

# Write header to log file with metadata
cat > "$LOG_FILE" << EOF
================================================================================
MTG Forge-rs Benchmark Results
================================================================================
CPU: $CPU_NAME
Timestamp: $TIMESTAMP_READABLE ($TIMESTAMP)
Git Commit: $GIT_COMMIT
Git Commit (short): $GIT_COMMIT_SHORT
Git Depth: $GIT_DEPTH
Git Branch: $GIT_BRANCH
Git Dirty: ${GIT_DIRTY:-clean}
================================================================================

EOF

BENCH_OUTPUT=$(mktemp)
trap "rm -f $BENCH_OUTPUT" EXIT

if [ -n "$BENCH_FILTER" ]; then
    cargo bench --bench game_benchmark "$BENCH_FILTER" 2>&1 | tee -a "$LOG_FILE" | tee "$BENCH_OUTPUT"
else
    cargo bench --bench game_benchmark 2>&1 | tee -a "$LOG_FILE" | tee "$BENCH_OUTPUT"
fi

echo ""
echo "=== Parsing Results ==="
echo ""

# Parse aggregated metrics from benchmark output
# Look for lines like: "=== Aggregated Metrics - Fresh Mode (seed 42, 123 games) ==="
# Then extract metrics from subsequent lines

parse_metrics() {
    local output_file="$1"
    local csv_file="$2"

    # State machine to parse metrics blocks
    local in_metrics_block=0
    local benchmark_name=""
    local seed=""
    local num_games=""
    local total_turns=""
    local total_actions=""
    local total_duration_ms=""
    local avg_turns_per_game=""
    local avg_actions_per_game=""
    local avg_duration_ms=""
    local games_per_sec=""
    local actions_per_sec=""
    local turns_per_sec=""
    local actions_per_turn=""
    local total_bytes_allocated=""
    local total_bytes_deallocated=""
    local net_bytes=""
    local avg_bytes_per_game=""
    local bytes_per_turn=""
    local bytes_per_sec=""

    # Define helper function to write CSV row
    write_csv_row() {
        # Normalize benchmark name to lowercase, remove "with" variations
        local norm_name=$(echo "$benchmark_name" | tr '[:upper:]' '[:lower:]' | sed 's/ /_/g')

        echo "Found metrics: $benchmark_name (seed=$seed, games=$num_games)"

        # Write CSV row
        echo "${TIMESTAMP},${GIT_COMMIT_SHORT},${GIT_DEPTH},${GIT_BRANCH},${GIT_DIRTY},${norm_name},${seed},${num_games},${total_turns},${total_actions},${total_duration_ms},${avg_turns_per_game},${avg_actions_per_game},${avg_duration_ms},${games_per_sec},${actions_per_sec},${turns_per_sec},${actions_per_turn},${total_bytes_allocated},${total_bytes_deallocated},${net_bytes},${avg_bytes_per_game},${bytes_per_turn},${bytes_per_sec}" >> "$csv_file"
    }

    while IFS= read -r line; do
        # Check for metrics block header
        if [[ "$line" =~ ^===\ Aggregated\ Metrics\ -\ (.+)\ Mode\ \(seed\ ([0-9]+),\ ([0-9]+)\ games\)\ === ]]; then
            # If we were in a previous block, save it
            if [ $in_metrics_block -eq 1 ]; then
                write_csv_row
            fi

            # Start new block
            in_metrics_block=1
            benchmark_name="${BASH_REMATCH[1]}"
            seed="${BASH_REMATCH[2]}"
            num_games="${BASH_REMATCH[3]}"

            # Reset metrics
            total_turns=""
            total_actions=""
            total_duration_ms=""
            avg_turns_per_game=""
            avg_actions_per_game=""
            avg_duration_ms=""
            games_per_sec=""
            actions_per_sec=""
            turns_per_sec=""
            actions_per_turn=""
            total_bytes_allocated=""
            total_bytes_deallocated=""
            net_bytes=""
            avg_bytes_per_game=""
            bytes_per_turn=""
            bytes_per_sec=""

            continue
        fi

        # Parse individual metrics
        if [ $in_metrics_block -eq 1 ]; then
            if [[ "$line" =~ ^\ \ Total\ turns:\ ([0-9]+) ]]; then
                total_turns="${BASH_REMATCH[1]}"
            elif [[ "$line" =~ ^\ \ Total\ actions:\ ([0-9]+) ]]; then
                total_actions="${BASH_REMATCH[1]}"
            elif [[ "$line" =~ ^\ \ Total\ duration:\ ([0-9.]+)([a-z]+) ]]; then
                local duration="${BASH_REMATCH[1]}"
                local unit="${BASH_REMATCH[2]}"
                # Convert to milliseconds
                if [ "$unit" = "s" ]; then
                    total_duration_ms=$(echo "$duration * 1000" | bc)
                elif [ "$unit" = "ms" ]; then
                    total_duration_ms="$duration"
                elif [ "$unit" = "µs" ] || [ "$unit" = "us" ]; then
                    total_duration_ms=$(echo "$duration / 1000" | bc)
                else
                    total_duration_ms="$duration"
                fi
            elif [[ "$line" =~ ^\ \ Avg\ turns/game:\ ([0-9.]+) ]]; then
                avg_turns_per_game="${BASH_REMATCH[1]}"
            elif [[ "$line" =~ ^\ \ Avg\ actions/game:\ ([0-9.]+) ]]; then
                avg_actions_per_game="${BASH_REMATCH[1]}"
            elif [[ "$line" =~ ^\ \ Avg\ duration/game\ \(naive\):\ ([0-9.]+)([a-zµ]+) ]]; then
                local duration="${BASH_REMATCH[1]}"
                local unit="${BASH_REMATCH[2]}"
                # Convert to milliseconds
                if [ "$unit" = "s" ]; then
                    avg_duration_ms=$(echo "$duration * 1000" | bc)
                elif [ "$unit" = "ms" ]; then
                    avg_duration_ms="$duration"
                elif [ "$unit" = "µs" ] || [ "$unit" = "us" ]; then
                    avg_duration_ms=$(echo "$duration / 1000" | bc)
                else
                    avg_duration_ms="$duration"
                fi
            elif [[ "$line" =~ ^\ \ Games/sec:\ ([0-9.]+) ]]; then
                games_per_sec="${BASH_REMATCH[1]}"
            elif [[ "$line" =~ ^\ \ Actions/sec:\ ([0-9.]+) ]]; then
                actions_per_sec="${BASH_REMATCH[1]}"
            elif [[ "$line" =~ ^\ \ Turns/sec:\ ([0-9.]+) ]]; then
                turns_per_sec="${BASH_REMATCH[1]}"
            elif [[ "$line" =~ ^\ \ Actions/turn:\ ([0-9.]+) ]]; then
                actions_per_turn="${BASH_REMATCH[1]}"
            elif [[ "$line" =~ ^\ \ Total\ bytes\ allocated:\ ([0-9]+) ]]; then
                total_bytes_allocated="${BASH_REMATCH[1]}"
            elif [[ "$line" =~ ^\ \ Total\ bytes\ deallocated:\ ([0-9]+) ]]; then
                total_bytes_deallocated="${BASH_REMATCH[1]}"
            elif [[ "$line" =~ ^\ \ Net\ bytes:\ (-?[0-9]+) ]]; then
                net_bytes="${BASH_REMATCH[1]}"
            elif [[ "$line" =~ ^\ \ Avg\ bytes/game:\ ([0-9.]+) ]]; then
                avg_bytes_per_game="${BASH_REMATCH[1]}"
            elif [[ "$line" =~ ^\ \ Bytes/turn:\ ([0-9.]+) ]]; then
                bytes_per_turn="${BASH_REMATCH[1]}"
            elif [[ "$line" =~ ^\ \ Bytes/sec:\ ([0-9.]+) ]]; then
                bytes_per_sec="${BASH_REMATCH[1]}"
            fi

            # Check if we've reached the end of the metrics block
            if [[ "$line" =~ ^Note: ]] || [[ "$line" =~ ^=== ]] || [[ "$line" =~ ^Benchmarking ]]; then
                if [ $in_metrics_block -eq 1 ]; then
                    write_csv_row
                    in_metrics_block=0
                fi
            fi
        fi
    done < "$output_file"

    # Write final block if we ended while in one
    if [ $in_metrics_block -eq 1 ]; then
        write_csv_row
    fi
}

parse_metrics "$BENCH_OUTPUT" "$HISTORY_FILE"

echo ""
echo "=== Results Saved ==="
echo "Performance history updated: $HISTORY_FILE"
echo "Full benchmark log saved to: $LOG_FILE"
echo ""
echo "Recent CSV entries:"
tail -n 5 "$HISTORY_FILE"
