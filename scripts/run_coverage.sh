#!/bin/bash
# Run tests with code coverage instrumentation and generate reports
#
# Requires: cargo install cargo-llvm-cov
#           rustup component add llvm-tools-preview
#
# Usage:
#   ./scripts/run_coverage.sh              # Unit tests, text summary + HTML
#   ./scripts/run_coverage.sh --full       # Unit tests + examples, merged
#   ./scripts/run_coverage.sh --lcov       # Also generate LCOV for CI
#   ./scripts/run_coverage.sh --branch     # Include branch coverage (nightly)
#
# Output:
#   Text summary printed to stdout
#   HTML report: experiment_results/coverage/html/index.html
#   LCOV file:   experiment_results/coverage/lcov.info (with --lcov)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
COVERAGE_DIR="$REPO_ROOT/experiment_results/coverage"

# Parse arguments
RUN_FULL=false
GENERATE_LCOV=false
BRANCH_COVERAGE=false

while [[ $# -gt 0 ]]; do
    case $1 in
        --full)
            RUN_FULL=true
            shift
            ;;
        --lcov)
            GENERATE_LCOV=true
            shift
            ;;
        --branch)
            BRANCH_COVERAGE=true
            shift
            ;;
        -h|--help)
            echo "Usage: $0 [--full] [--lcov] [--branch]"
            echo ""
            echo "Options:"
            echo "  --full     Run unit tests + examples (slower, more complete)"
            echo "  --lcov     Also generate LCOV output for CI integration"
            echo "  --branch   Include branch coverage (requires nightly toolchain)"
            echo ""
            echo "Output:"
            echo "  Text summary to stdout"
            echo "  HTML report: experiment_results/coverage/html/index.html"
            echo "  LCOV file:   experiment_results/coverage/lcov.info (with --lcov)"
            exit 0
            ;;
        *)
            echo "Unknown option: $1"
            echo "Usage: $0 [--full] [--lcov] [--branch]"
            exit 1
            ;;
    esac
done

# Check that cargo-llvm-cov is installed
if ! command -v cargo-llvm-cov >/dev/null 2>&1; then
    echo "Error: cargo-llvm-cov not found"
    echo ""
    echo "Install with:"
    echo "  cargo install cargo-llvm-cov"
    echo "  rustup component add llvm-tools-preview"
    exit 1
fi

# Build common flags
IGNORE_REGEX='mtg-benchmarks|/tests/|/examples/'
BRANCH_FLAG=""
if [ "$BRANCH_COVERAGE" = true ]; then
    BRANCH_FLAG="--branch"
fi

echo "=== Code Coverage ==="
echo ""

# Clean previous coverage data
echo "Cleaning previous coverage data..."
cargo llvm-cov clean --workspace
echo ""

# Step 1: Run unit tests with coverage (--no-report to accumulate)
echo "=== Running unit tests with coverage instrumentation ==="
echo ""
cargo llvm-cov --no-report nextest --features network
echo ""

# Step 2 (optional): Run examples with coverage
if [ "$RUN_FULL" = true ]; then
    echo "=== Running examples with coverage instrumentation ==="
    echo ""

    # Discover available examples
    EXAMPLES=$(cargo run -p mtg-forge-rs --example 2>&1 | grep -A 1000 "Available examples:" | tail -n +2 | sed 's/^[[:space:]]*//' | grep -v '^$')

    if [ -n "$EXAMPLES" ]; then
        while IFS= read -r example; do
            echo "  Running example: $example"
            cargo llvm-cov --no-report run -p mtg-forge-rs --example "$example" 2>&1 | tail -1
        done <<< "$EXAMPLES"
        echo ""
    else
        echo "  Warning: No examples found, skipping"
        echo ""
    fi
fi

# Ensure output directory exists
mkdir -p "$COVERAGE_DIR"

# Step 3: Generate text summary
echo "=== Coverage Summary ==="
echo ""
cargo llvm-cov report --summary-only $BRANCH_FLAG \
    --ignore-filename-regex "$IGNORE_REGEX"
echo ""

# Step 4: Generate HTML report
echo "Generating HTML report..."
cargo llvm-cov report --html --output-dir "$COVERAGE_DIR/html" $BRANCH_FLAG \
    --ignore-filename-regex "$IGNORE_REGEX"
echo "HTML report: $COVERAGE_DIR/html/index.html"
echo ""

# Step 5 (optional): Generate LCOV
if [ "$GENERATE_LCOV" = true ]; then
    echo "Generating LCOV report..."
    cargo llvm-cov report --lcov --output-path "$COVERAGE_DIR/lcov.info" $BRANCH_FLAG \
        --ignore-filename-regex "$IGNORE_REGEX"
    echo "LCOV report: $COVERAGE_DIR/lcov.info"
    echo ""
fi

echo "=== Coverage Complete ==="
