#!/bin/bash
# Periodically run benchmarks when git depth advances by 5+ commits
#
# This script checks if the current git depth is at least 5 commits past
# the last recorded benchmark in experiment_results/perf_history.csv.
# If so, it runs the benchmarks and appends the results.

set -e

# Color codes for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Get CPU name and normalize it for use in directory paths
# Extracts from /proc/cpuinfo, replaces spaces with underscores, strips non-alphanumeric except _ and -
get_cpu_name() {
    local cpu_name=$(grep "model name" /proc/cpuinfo | head -1 | cut -d':' -f2 | sed 's/^[ \t]*//')
    # Replace spaces with underscores and remove any characters that aren't alphanumeric, underscore, or dash
    echo "$cpu_name" | sed 's/ /_/g' | sed 's/[^a-zA-Z0-9_-]//g'
}

# Configuration
CPU_NAME=$(get_cpu_name)
CSV_FILE="experiment_results/$CPU_NAME/perf_history.csv"
MIN_DEPTH_DELTA=5

# Helper functions
log_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

log_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

log_warning() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# Get current git depth
get_current_depth() {
    git rev-list --count HEAD
}

# Get last recorded git depth from CSV
get_last_recorded_depth() {
    if [ ! -f "$CSV_FILE" ]; then
        echo "0"
        return
    fi

    # Get last non-header line, extract 3rd column (git_depth)
    local last_depth=$(tail -n +2 "$CSV_FILE" | tail -1 | cut -d',' -f3)

    # If empty or not a number, return 0
    if [ -z "$last_depth" ] || ! [[ "$last_depth" =~ ^[0-9]+$ ]]; then
        echo "0"
    else
        echo "$last_depth"
    fi
}

# Main script
main() {
    log_info "Checking if benchmarks should run..."
    log_info "CPU: $CPU_NAME"

    # Ensure we're in a git repository
    if ! git rev-parse --git-dir > /dev/null 2>&1; then
        log_error "Not in a git repository!"
        exit 1
    fi

    # Get current and last recorded depths
    current_depth=$(get_current_depth)
    last_depth=$(get_last_recorded_depth)
    depth_delta=$((current_depth - last_depth))

    log_info "Current git depth: $current_depth"
    log_info "Last recorded depth: $last_depth"
    log_info "Depth delta: $depth_delta"

    # Check if we should run benchmarks
    if [ $depth_delta -lt $MIN_DEPTH_DELTA ]; then
        log_warning "Depth delta ($depth_delta) is less than minimum ($MIN_DEPTH_DELTA)"
        log_warning "Skipping benchmarks - need $((MIN_DEPTH_DELTA - depth_delta)) more commits"
        exit 0
    fi

    log_success "Depth delta ($depth_delta) >= minimum ($MIN_DEPTH_DELTA)"
    log_info "Running benchmarks..."

    # Ensure experiment_results directory exists (CPU-specific)
    mkdir -p "experiment_results/$CPU_NAME"

    # Initialize CSV file with header if it doesn't exist
    if [ ! -f "$CSV_FILE" ]; then
        log_info "Creating $CSV_FILE with header..."
        echo "timestamp,git_commit,git_depth,git_branch,git_dirty,benchmark_name,seed,num_games,total_turns,total_actions,total_duration_ms,avg_turns_per_game,avg_actions_per_game,avg_duration_ms_per_game,games_per_sec,actions_per_sec,turns_per_sec,actions_per_turn,total_bytes_allocated,total_bytes_deallocated,net_bytes,avg_bytes_per_game,bytes_per_turn,bytes_per_sec" > "$CSV_FILE"
    fi

    # Run benchmarks
    # Note: The benchmark suite should append results to the CSV file
    log_info "Building benchmarks..."
    cargo bench --no-run

    log_info "Running benchmark suite..."
    cargo bench 2>&1 | tee benchmark_output.log

    # Check if benchmarks completed successfully
    if [ $? -eq 0 ]; then
        log_success "Benchmarks completed successfully!"

        # Verify results were recorded
        new_last_depth=$(get_last_recorded_depth)
        if [ "$new_last_depth" -gt "$last_depth" ]; then
            log_success "Results recorded in $CSV_FILE (depth: $new_last_depth)"
        else
            log_warning "Benchmarks ran but results may not have been recorded in CSV"
        fi
    else
        log_error "Benchmarks failed!"
        exit 1
    fi

    # Show summary
    echo ""
    log_info "=== Benchmark Summary ==="
    log_info "Previous depth: $last_depth"
    log_info "Current depth:  $current_depth"
    log_info "Results file:   $CSV_FILE"

    # Show recent entries count
    local entry_count=$(tail -n +2 "$CSV_FILE" | wc -l)
    log_info "Total benchmark entries: $entry_count"
}

# Run main function
main "$@"
