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

# Run the Python test runner (exit 0 by default, outputting TAP)
python3 tests/run_e2e_tests.py

echo "=== E2E Feature Test Suite Completed ==="
exit 0
