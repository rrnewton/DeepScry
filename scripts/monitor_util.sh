#!/bin/bash
#
# System utilization monitoring wrapper
#
# Wraps a command with CPU utilization monitoring. Starts monitoring before
# the command runs and reports statistics after completion.
#
# Usage:
#   ./monitor_util.sh <command> [args...]
#
# Example:
#   ./monitor_util.sh make test
#   ./monitor_util.sh cargo nextest run
#   ./monitor_util.sh sleep 5
#
# This script uses utilization_prehook.sh and utilization_posthook.sh to
# monitor system CPU utilization during command execution.

set -e  # Exit on error

# Get script directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Check if command was provided
if [ $# -eq 0 ]; then
    echo "Error: No command specified"
    echo ""
    echo "Usage: $0 <command> [args...]"
    echo ""
    echo "Example:"
    echo "  $0 make test"
    echo "  $0 cargo nextest run"
    echo ""
    exit 1
fi

# Start monitoring
if [ -f "$SCRIPT_DIR/utilization_prehook.sh" ]; then
    source "$SCRIPT_DIR/utilization_prehook.sh"
else
    echo "Error: utilization_prehook.sh not found in $SCRIPT_DIR"
    exit 1
fi

# Run the command with all arguments
# Capture exit code to report it after monitoring stops
set +e  # Don't exit on error - we want to report utilization even if command fails
"$@"
EXIT_CODE=$?
set -e

# Stop monitoring and show report
if [ -f "$SCRIPT_DIR/utilization_posthook.sh" ]; then
    source "$SCRIPT_DIR/utilization_posthook.sh"
else
    echo "Error: utilization_posthook.sh not found in $SCRIPT_DIR"
    exit 1
fi

# Exit with the same code as the wrapped command
exit $EXIT_CODE
