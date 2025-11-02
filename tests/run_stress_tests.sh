#!/bin/bash
# Run snapshot/resume stress tests in parallel using GNU parallel
#
# Usage:
#   ./run_stress_tests.sh [--sequential] [--keep] [deck_names...]
#
# Examples:
#   ./run_stress_tests.sh                                  # Run all default decks
#   ./run_stress_tests.sh royal_assassin white_aggro_4ed  # Run specific decks
#   ./run_stress_tests.sh --sequential                     # Force sequential execution
#   ./run_stress_tests.sh --keep                           # Save all test artifacts
#   ./run_stress_tests.sh --keep --sequential grizzly_bears
#
# Flags:
#   --sequential    Force sequential execution (disable parallel)
#   --keep          Save all artifacts (logs, gamestates, snapshots) to test_artifacts/
#
# If no deck names are provided, runs default set of decks.
# Deck names should be without .dck extension.

set -e  # Exit on error

# Parse command line arguments
FORCE_SEQUENTIAL=false
KEEP_ARTIFACTS=false
DECK_ARGS=()

while [[ $# -gt 0 ]]; do
    case $1 in
        -h|--help)
            echo "Run snapshot/resume stress tests in parallel using GNU parallel"
            echo ""
            echo "Usage:"
            echo "  $0 [--sequential] [--keep] [deck_names...]"
            echo ""
            echo "Examples:"
            echo "  $0                                  # Run all default decks"
            echo "  $0 royal_assassin white_aggro_4ed  # Run specific decks"
            echo "  $0 --sequential                     # Force sequential execution"
            echo "  $0 --keep                           # Save all test artifacts"
            echo "  $0 --keep --sequential grizzly_bears"
            echo ""
            echo "Flags:"
            echo "  -h, --help      Show this help message"
            echo "  --sequential    Force sequential execution (disable parallel)"
            echo "  --keep          Save all artifacts (logs, gamestates, snapshots) to test_artifacts/"
            echo ""
            echo "If no deck names are provided, runs default set of decks."
            echo "Deck names should be without .dck extension."
            exit 0
            ;;
        --sequential)
            FORCE_SEQUENTIAL=true
            shift
            ;;
        --keep|--keep-logs)
            KEEP_ARTIFACTS=true
            shift
            ;;
        *)
            DECK_ARGS+=("$1")
            shift
            ;;
    esac
done

# Check for cardsfolder
if [ ! -d "cardsfolder" ]; then
    echo "Warning: cardsfolder not found, skipping stress tests"
    exit 0
fi

# Default decks to test if none specified
if [ ${#DECK_ARGS[@]} -eq 0 ]; then
    DECKS=(
        "royal_assassin"
        "white_aggro_4ed"
        "grizzly_bears"
        "white_weenie"
    )
else
    DECKS=("${DECK_ARGS[@]}")
fi

# Controller matchups to test
# Format: "p1_controller:p2_controller[:flags]"
# Flags can include "--switch-fixed"
MATCHUPS=(
    "heuristic:heuristic"
    "random:heuristic"
    "random:heuristic:--switch-fixed"
    "heuristic:random:--switch-fixed"
)

echo "=== MTG Snapshot/Resume Stress Tests ==="
echo ""
echo "Building release binary once (scripts call mtg many times)..."
cargo build --release
echo ""
echo "Decks to test: ${DECKS[*]}"
echo "Controller matchups: ${#MATCHUPS[@]}"
echo ""

# Generate list of all test combinations
TEST_CASES=()
for deck in "${DECKS[@]}"; do
    deck_path="decks/${deck}.dck"

    # Check if deck file exists
    if [ ! -f "$deck_path" ]; then
        echo "Warning: Skipping $deck - file not found at $deck_path"
        continue
    fi

    for matchup in "${MATCHUPS[@]}"; do
        # Split matchup into p1, p2, and optional flags
        IFS=':' read -r p1 p2 flags <<< "$matchup"
        TEST_CASES+=("$deck_path:$p1:$p2:$flags")
    done
done

if [ ${#TEST_CASES[@]} -eq 0 ]; then
    echo "Error: No valid test cases found"
    exit 1
fi

TOTAL=${#TEST_CASES[@]}
echo "Total test cases: $TOTAL"
echo ""

# Make test script executable
chmod +x scripts/snapshot_stress_test_single.py

# Function to print summary and exit with appropriate status
print_summary_and_exit() {
    local passed="$1"
    local total="$2"
    local failed="$3"
    # Optional 4th arg: failed tests list, may contain \n sequences
    local failed_tests="${4-}"

    echo ""
    echo "========================================"
    echo "Summary: $passed/$total tests passed"
    echo "========================================"

    if [ "$failed" -gt 0 ]; then
        echo ""
        if [ -n "$failed_tests" ]; then
            echo "Failed tests:"
            # Interpret any embedded \n sequences for readability
            printf "%b\n" "$failed_tests"
            echo ""
            echo "❌ $failed test(s) failed!"
        else
            echo "❌ $failed test(s) failed! (see detailed output above)"
        fi
        exit 1
    fi

    echo ""
    echo "✅ All stress tests passed!"
    exit 0
}

# Function to run a single test case
run_test() {
    local test_case="$1"

    # Parse test case: deck_path:p1:p2:flags
    IFS=':' read -r deck_path p1 p2 flags <<< "$test_case"

    # Extract deck name for display
    deck_name=$(basename "$deck_path" .dck)

    # Build test description
    local test_desc="$p1 vs $p2"
    if [ -n "$flags" ]; then
        test_desc="$test_desc $flags"
    fi

    # Build command with optional flags
    local cmd="./scripts/snapshot_stress_test_single.py $deck_path $p1 $p2 --quiet"
    if [ "$KEEP_ARTIFACTS" = true ]; then
        cmd="$cmd --keep"
    fi
    if [ -n "$flags" ]; then
        cmd="$cmd $flags"
    fi

    echo " > $cmd"

    # Run the test and capture output
    local output
    output=$($cmd 2>&1)
    local exit_code=$?

    if [ $exit_code -eq 0 ]; then
        echo "✓ $deck_name ($test_desc)"
        return 0
    else
        echo "✗ $deck_name ($test_desc)"
        # Print detailed failure info to stderr for capturing
        {
            echo ""
            echo "=========================================="
            echo "FAILED: $deck_name ($test_desc)"
            echo "=========================================="
            echo ""
            echo "Reproduce with:"
            local repro_cmd="  ./scripts/snapshot_stress_test_single.py $deck_path $p1 $p2 --verbose"
            if [ "$KEEP_ARTIFACTS" = true ]; then
                repro_cmd="$repro_cmd --keep"
            fi
            if [ -n "$flags" ]; then
                repro_cmd="$repro_cmd $flags"
            fi
            echo "$repro_cmd"
            echo ""
            echo "Output:"
            echo "$output"
            echo ""
        } >&2
        return 1
    fi
}

export -f run_test
export KEEP_ARTIFACTS

# Check if GNU parallel is available and not forcing sequential
if [ "$FORCE_SEQUENTIAL" = true ] || ! command -v parallel &> /dev/null; then
    if [ "$FORCE_SEQUENTIAL" = true ]; then
        echo "INFO: --sequential flag specified, using sequential execution"
        echo ""
    else
        echo "WARNING: GNU parallel not found, falling back to sequential execution"
        echo "Install with: apt-get install parallel (Ubuntu/Debian) or brew install parallel (macOS)"
        echo ""
    fi

    # Sequential execution
    PASSED=0
    FAILED=0
    FAILED_TESTS=""

    for test_case in "${TEST_CASES[@]}"; do
        if run_test "$test_case"; then
            PASSED=$((PASSED + 1))
        else
            FAILED=$((FAILED + 1))
            IFS=':' read -r deck_path p1 p2 flags <<< "$test_case"
            deck_name=$(basename "$deck_path" .dck)
            test_desc="$p1 vs $p2"
            if [ -n "$flags" ]; then
                test_desc="$test_desc $flags"
            fi
            FAILED_TESTS="$FAILED_TESTS\n  - $deck_name ($test_desc)"
        fi
    done

    # Print summary and exit appropriately
    print_summary_and_exit "$PASSED" "$TOTAL" "$FAILED" "$FAILED_TESTS"
fi

# Parallel execution using GNU parallel
# Run with moderate parallelism (jobs=2) since each test spawns multiple game processes
# --line-buffer: Buffer output by line (prevents interleaving)
# --joblog: Track which jobs passed/failed
echo "Running $TOTAL tests in parallel..."
echo ""

JOBLOG=$(mktemp)
trap "rm -f $JOBLOG" EXIT

# Run all tests and capture exit codes in joblog
# Temporarily disable exit-on-error so we can process results even if tests fail
set +e
printf '%s\n' "${TEST_CASES[@]}" | parallel --jobs=$TOTAL --line-buffer --joblog "$JOBLOG" 'run_test {}'
set -e

# Count results from joblog (skip header line)
PASSED=$(awk 'NR>1 && $7==0 {count++} END {print count+0}' "$JOBLOG")
FAILED=$(awk 'NR>1 && $7!=0 {count++} END {print count+0}' "$JOBLOG")

# Print summary and exit appropriately (no explicit list of failed tests in parallel mode)
print_summary_and_exit "$PASSED" "$TOTAL" "$FAILED"
