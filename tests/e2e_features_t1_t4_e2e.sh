#!/usr/bin/env bash
# E2E test wrapper for Feature Tiers 1-4
#
# SCRIPT_DIR is determined via dirname $0
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

# Build the release binary once if not already built
ensure_mtg_binary

echo "=== Running E2E Feature Test Suite (Tiers 1-4) ==="
cd "$WORKSPACE_ROOT"

# Run the Python test runner for implemented features (F1, F2, T4)
python3 tests/run_e2e_tests.py --filter F1
python3 tests/run_e2e_tests.py --filter F2
python3 tests/run_e2e_tests.py --filter T4

echo "=== E2E Feature Test Suite Completed ==="
exit 0
