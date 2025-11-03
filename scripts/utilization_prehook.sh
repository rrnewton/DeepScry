#!/bin/bash
#
# System utilization monitoring prehook
#
# This script starts monitoring system utilization metrics in the background.
# It samples CPU usage periodically and stores results in a temp file.
#
# Usage:
#   source ./utilization_prehook.sh
#
# After sourcing, the following variables are set:
#   UTIL_MONITOR_PID   - PID of the background monitoring process
#   UTIL_STATS_FILE    - Path to the file where stats are being collected
#   UTIL_START_TIME    - Timestamp when monitoring started (epoch seconds)

# Create a temporary file for collecting stats
UTIL_STATS_FILE=$(mktemp /tmp/util_monitor.XXXXXX)
export UTIL_STATS_FILE

# Record start time
UTIL_START_TIME=$(date +%s)
export UTIL_START_TIME

# Start background monitoring process
# We'll sample every 1 second using mpstat or fallback to parsing /proc/stat
(
    # Initialize variables
    SAMPLE_INTERVAL=1

    # Check if mpstat is available (from sysstat package)
    if command -v mpstat &> /dev/null; then
        # Use mpstat for more accurate per-CPU measurements
        # Format: timestamp, all CPUs utilization
        while true; do
            # mpstat outputs: %usr %nice %sys %iowait %irq %soft %steal %guest %gnice %idle
            # We want to calculate %busy = 100 - %idle
            TIMESTAMP=$(date +%s)
            IDLE=$(mpstat 1 1 | awk '/Average:/ && /all/ {print $NF}')
            if [ -n "$IDLE" ]; then
                BUSY=$(echo "100 - $IDLE" | bc -l)
                echo "$TIMESTAMP $BUSY" >> "$UTIL_STATS_FILE"
            fi
        done
    else
        # Fallback: parse /proc/stat manually
        # This gives us aggregate CPU time across all cores
        PREV_LINE=""
        while true; do
            TIMESTAMP=$(date +%s)
            CURR_LINE=$(grep '^cpu ' /proc/stat)

            if [ -n "$PREV_LINE" ]; then
                # Parse current values
                read cpu user nice system idle iowait irq softirq steal guest guest_nice <<< "$CURR_LINE"
                CURR_TOTAL=$((user + nice + system + idle + iowait + irq + softirq + steal))
                CURR_IDLE=$((idle + iowait))

                # Parse previous values
                read cpu_p user_p nice_p system_p idle_p iowait_p irq_p softirq_p steal_p guest_p guest_nice_p <<< "$PREV_LINE"
                PREV_TOTAL=$((user_p + nice_p + system_p + idle_p + iowait_p + irq_p + softirq_p + steal_p))
                PREV_IDLE=$((idle_p + iowait_p))

                # Calculate deltas
                TOTAL_DELTA=$((CURR_TOTAL - PREV_TOTAL))
                IDLE_DELTA=$((CURR_IDLE - PREV_IDLE))

                # Calculate utilization percentage
                if [ $TOTAL_DELTA -gt 0 ]; then
                    BUSY=$(echo "scale=2; 100 * ($TOTAL_DELTA - $IDLE_DELTA) / $TOTAL_DELTA" | bc -l)
                    echo "$TIMESTAMP $BUSY" >> "$UTIL_STATS_FILE"
                fi
            fi

            PREV_LINE="$CURR_LINE"
            sleep "$SAMPLE_INTERVAL"
        done
    fi
) &

UTIL_MONITOR_PID=$!
export UTIL_MONITOR_PID

# Disown the process so it doesn't receive signals from parent shell
disown

# Provide feedback
echo "System utilization monitoring started (PID: $UTIL_MONITOR_PID)"
echo "Stats file: $UTIL_STATS_FILE"
