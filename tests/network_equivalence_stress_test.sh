#!/usr/bin/env bash
# Stress test: Run network_vs_local_equivalence_e2e.sh concurrently
#
# Runs multiple copies of the equivalence test in parallel to verify
# the network implementation is stable under concurrent load.
#
# Usage: ./network_equivalence_stress_test.sh [copies] [rounds]
# Default: 10 copies × 3 rounds

set -euo pipefail

COPIES=${1:-10}
ROUNDS=${2:-3}

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
TEST_SCRIPT="$SCRIPT_DIR/network_vs_local_equivalence_e2e.sh"

if [[ ! -x "$TEST_SCRIPT" ]]; then
    echo -e "${RED}Error: Test script not found: $TEST_SCRIPT${NC}"
    exit 1
fi

echo "=== Network Equivalence Stress Test ==="
echo "Configuration: $COPIES copies × $ROUNDS rounds"
echo

TOTAL_TESTS=$((COPIES * ROUNDS))
PASSED=0
FAILED=0
FAILED_ROUNDS=""

for round in $(seq 1 $ROUNDS); do
    echo -e "${BLUE}=== Round $round of $ROUNDS ===${NC}"
    echo "Starting $COPIES concurrent tests..."

    # Create temp directory for this round's PIDs and results
    ROUND_DIR="/tmp/network_stress_round_${round}_$$"
    mkdir -p "$ROUND_DIR"

    # Start all copies in parallel
    for copy in $(seq 1 $COPIES); do
        # Use different seed for each copy to get diverse test coverage
        SEED=$((round * 1000 + copy))

        (
            # Override seed in the test script by setting environment
            # The test script uses fixed SEED=3, but we run it as-is
            # since each copy uses a random port anyway
            if "$TEST_SCRIPT" >"$ROUND_DIR/output_$copy.log" 2>&1; then
                echo "PASS" > "$ROUND_DIR/result_$copy"
            else
                echo "FAIL" > "$ROUND_DIR/result_$copy"
            fi
        ) &
        echo "  Started copy $copy (PID $!)"
    done

    # Wait for all copies to complete with progress
    echo "Waiting for $COPIES tests to complete..."
    ELAPSED=0
    STILL_RUNNING=$COPIES
    while [[ $STILL_RUNNING -gt 0 && $ELAPSED -lt 300 ]]; do
        sleep 5
        ELAPSED=$((ELAPSED + 5))
        STILL_RUNNING=0
        for copy in $(seq 1 $COPIES); do
            if [[ ! -f "$ROUND_DIR/result_$copy" ]]; then
                ((STILL_RUNNING++))
            fi
        done
        echo "  Progress: $((COPIES - STILL_RUNNING))/$COPIES complete after ${ELAPSED}s"
    done
    # Final wait for any stragglers
    wait 2>/dev/null || true

    # Collect results
    ROUND_PASSED=0
    ROUND_FAILED=0
    for copy in $(seq 1 $COPIES); do
        if [[ -f "$ROUND_DIR/result_$copy" ]] && grep -q "PASS" "$ROUND_DIR/result_$copy"; then
            ((ROUND_PASSED++))
        else
            ((ROUND_FAILED++))
            echo -e "  ${RED}Copy $copy FAILED${NC}"
            if [[ -f "$ROUND_DIR/output_$copy.log" ]]; then
                echo "  Last 10 lines of output:"
                tail -10 "$ROUND_DIR/output_$copy.log" | sed 's/^/    /'
            fi
        fi
    done

    echo -e "Round $round: ${GREEN}$ROUND_PASSED passed${NC}, ${RED}$ROUND_FAILED failed${NC}"
    echo

    PASSED=$((PASSED + ROUND_PASSED))
    FAILED=$((FAILED + ROUND_FAILED))

    if [[ $ROUND_FAILED -gt 0 ]]; then
        FAILED_ROUNDS="$FAILED_ROUNDS $round"
    fi

    # Clean up round directory
    rm -rf "$ROUND_DIR"
done

echo "=== Stress Test Complete ==="
echo -e "Total: ${GREEN}$PASSED passed${NC}, ${RED}$FAILED failed${NC} out of $TOTAL_TESTS"

if [[ $FAILED -eq 0 ]]; then
    echo -e "${GREEN}✓ All $TOTAL_TESTS tests passed!${NC}"
    exit 0
else
    echo -e "${RED}✗ $FAILED tests failed in rounds:$FAILED_ROUNDS${NC}"
    exit 1
fi
