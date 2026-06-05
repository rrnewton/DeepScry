#!/bin/bash
#
# System utilization monitoring posthook
#
# This script stops the background monitoring process started by utilization_prehook.sh
# and analyzes the collected data to report:
# - Average CPU utilization
# - Peak CPU utilization
# - Time spent at various utilization levels
# - Assessment of parallelism efficiency
#
# Usage:
#   source ./utilization_posthook.sh
#
# Expects these environment variables (set by prehook):
#   UTIL_MONITOR_PID   - PID of the background monitoring process
#   UTIL_STATS_FILE    - Path to the file where stats were collected
#   UTIL_START_TIME    - Timestamp when monitoring started

# Colors for output
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
RED='\033[0;31m'
NC='\033[0m' # No Color

# Check if monitoring was started
if [ -z "$UTIL_MONITOR_PID" ] || [ -z "$UTIL_STATS_FILE" ] || [ -z "$UTIL_START_TIME" ]; then
    echo -e "${RED}Error: Monitoring environment variables not set${NC}"
    echo "Make sure to source utilization_prehook.sh first"
    return 1 2>/dev/null || exit 1
fi

# Stop the monitoring process
if kill "$UTIL_MONITOR_PID" 2>/dev/null; then
    # Give it a moment to finish writing
    sleep 0.5
    # Force kill if still alive
    kill -9 "$UTIL_MONITOR_PID" 2>/dev/null || true
fi

# Wait for the process to actually terminate
wait "$UTIL_MONITOR_PID" 2>/dev/null || true

# Calculate total duration
UTIL_END_TIME=$(date +%s)
DURATION=$((UTIL_END_TIME - UTIL_START_TIME))

echo ""
echo "========================================"
echo -e "${CYAN}System Utilization Report${NC}"
echo "========================================"
echo ""

# Check if we collected any data
if [ ! -f "$UTIL_STATS_FILE" ] || [ ! -s "$UTIL_STATS_FILE" ]; then
    echo -e "${YELLOW}Warning: No utilization data collected${NC}"
    rm -f "$UTIL_STATS_FILE" 2>/dev/null
    return 0 2>/dev/null || exit 0
fi

# Get number of CPU cores
NUM_CORES=$(nproc)

# Analyze the collected data
ANALYSIS=$(awk -v cores="$NUM_CORES" '
BEGIN {
    count = 0
    sum = 0
    max = 0
    min = 100

    # Buckets for utilization histogram
    bucket_0_25 = 0    # 0-25% busy
    bucket_25_50 = 0   # 25-50% busy
    bucket_50_75 = 0   # 50-75% busy
    bucket_75_90 = 0   # 75-90% busy
    bucket_90_100 = 0  # 90-100% busy
}
{
    if (NF >= 2) {
        util = $2
        count++
        sum += util

        if (util > max) max = util
        if (util < min) min = util

        # Categorize into buckets
        if (util < 25) bucket_0_25++
        else if (util < 50) bucket_25_50++
        else if (util < 75) bucket_50_75++
        else if (util < 90) bucket_75_90++
        else bucket_90_100++
    }
}
END {
    if (count > 0) {
        avg = sum / count

        printf "samples=%d\n", count
        printf "avg=%.1f\n", avg
        printf "min=%.1f\n", min
        printf "max=%.1f\n", max
        printf "bucket_0_25=%d\n", bucket_0_25
        printf "bucket_25_50=%d\n", bucket_25_50
        printf "bucket_50_75=%d\n", bucket_50_75
        printf "bucket_75_90=%d\n", bucket_75_90
        printf "bucket_90_100=%d\n", bucket_90_100
    }
}
' "$UTIL_STATS_FILE")

# Parse the analysis results
SAMPLES=$(echo "$ANALYSIS" | grep '^samples=' | cut -d= -f2)
AVG=$(echo "$ANALYSIS" | grep '^avg=' | cut -d= -f2)
MIN=$(echo "$ANALYSIS" | grep '^min=' | cut -d= -f2)
MAX=$(echo "$ANALYSIS" | grep '^max=' | cut -d= -f2)
BUCKET_0_25=$(echo "$ANALYSIS" | grep '^bucket_0_25=' | cut -d= -f2)
BUCKET_25_50=$(echo "$ANALYSIS" | grep '^bucket_25_50=' | cut -d= -f2)
BUCKET_50_75=$(echo "$ANALYSIS" | grep '^bucket_50_75=' | cut -d= -f2)
BUCKET_75_90=$(echo "$ANALYSIS" | grep '^bucket_75_90=' | cut -d= -f2)
BUCKET_90_100=$(echo "$ANALYSIS" | grep '^bucket_90_100=' | cut -d= -f2)

# Display results
echo "Duration:         ${DURATION}s"
echo "CPU Cores:        $NUM_CORES"
echo "Samples:          $SAMPLES"
echo ""
echo -e "${CYAN}CPU Utilization:${NC}"
echo "  Average:        ${AVG}%"
echo "  Minimum:        ${MIN}%"
echo "  Maximum:        ${MAX}%"
echo ""

# Calculate histogram percentages
if [ "$SAMPLES" -gt 0 ]; then
    PCT_0_25=$(echo "scale=1; 100 * $BUCKET_0_25 / $SAMPLES" | bc -l)
    PCT_25_50=$(echo "scale=1; 100 * $BUCKET_25_50 / $SAMPLES" | bc -l)
    PCT_50_75=$(echo "scale=1; 100 * $BUCKET_50_75 / $SAMPLES" | bc -l)
    PCT_75_90=$(echo "scale=1; 100 * $BUCKET_75_90 / $SAMPLES" | bc -l)
    PCT_90_100=$(echo "scale=1; 100 * $BUCKET_90_100 / $SAMPLES" | bc -l)

    echo -e "${CYAN}Time Distribution:${NC}"
    printf "  0-25%% busy:    %3d samples (%5.1f%%)\n" $BUCKET_0_25 $PCT_0_25
    printf "  25-50%% busy:   %3d samples (%5.1f%%)\n" $BUCKET_25_50 $PCT_25_50
    printf "  50-75%% busy:   %3d samples (%5.1f%%)\n" $BUCKET_50_75 $PCT_50_75
    printf "  75-90%% busy:   %3d samples (%5.1f%%)\n" $BUCKET_75_90 $PCT_75_90
    printf "  90-100%% busy:  %3d samples (%5.1f%%)\n" $BUCKET_90_100 $PCT_90_100
    echo ""

    # Assess parallelism efficiency
    echo -e "${CYAN}Parallelism Assessment:${NC}"

    # Calculate what percentage of time we're above 75% utilization
    HIGH_UTIL=$((BUCKET_75_90 + BUCKET_90_100))
    PCT_HIGH=$(echo "scale=1; 100 * $HIGH_UTIL / $SAMPLES" | bc -l)

    # Check if average is above threshold
    AVG_THRESHOLD=75
    AVG_INT=$(printf "%.0f" "$AVG")

    if [ "$AVG_INT" -ge 90 ]; then
        echo -e "  ${GREEN}✓ Excellent parallelism${NC} (avg ${AVG}% utilization)"
        echo "    CPUs are well-utilized throughout execution"
    elif [ "$AVG_INT" -ge "$AVG_THRESHOLD" ]; then
        echo -e "  ${GREEN}✓ Good parallelism${NC} (avg ${AVG}% utilization)"
        echo "    Most of the time CPUs are busy"
    elif [ "$AVG_INT" -ge 50 ]; then
        echo -e "  ${YELLOW}⚠ Moderate parallelism${NC} (avg ${AVG}% utilization)"
    else
        echo -e "  ${RED}✗ Poor parallelism${NC} (avg ${AVG}% utilization)"
    fi

    # Additional insights based on distribution
    LOW_UTIL=$((BUCKET_0_25 + BUCKET_25_50))
    PCT_LOW=$(echo "scale=1; 100 * $LOW_UTIL / $SAMPLES" | bc -l)
    PCT_LOW_INT=$(printf "%.0f" "$PCT_LOW")

    if [ "$PCT_LOW_INT" -gt 30 ]; then
        echo ""
        echo -e "  ${YELLOW}Note:${NC} Spent ${PCT_LOW}% of time at <50% CPU utilization"
        echo "        This suggests periods of sequential execution or I/O waits"
    fi
fi

echo ""
echo "========================================"
echo ""

# Cleanup
rm -f "$UTIL_STATS_FILE"

# Unset environment variables
unset UTIL_MONITOR_PID
unset UTIL_STATS_FILE
unset UTIL_START_TIME
