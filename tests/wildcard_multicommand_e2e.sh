#!/usr/bin/env bash
# Wildcard Multi-Command E2E Test
#
# Tests the wildcard (*) separator feature for Fixed controller multi-command scripts.
# This demonstrates:
# - Multiple commands separated by semicolons (;)
# - Wildcard (*) separator to skip irrelevant priority passes
# - Commands only execute when they match available actions (spell/ability choices)
#
# Syntax: --p1-fixed-inputs='pass; *; attack silvercoat'
#   - First command: 'pass' (executes immediately at first priority)
#   - Wildcard: '*' (enters wildcard mode - skips priority passes until next command matches)
#   - Third command: 'attack silvercoat' (waits until attack phase, then executes)
#
# Note: Wildcard mode currently only works for spell/ability choices in choose_spell_ability_to_play.
# Blocking choices use a simpler command consumption model.
#
# Test scenario:
# - Player 1: Silvercoat Lion (2/2) attacks after passing priority
# - Player 2: No creatures (cannot block)
# - Silvercoat Lion deals damage directly to Player 2

set -euo pipefail

# Get script directory and source shared test helpers
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/test_helpers.sh"

# Ensure release binary is built
ensure_mtg_binary

echo "========================================"
echo "Wildcard Multi-Command E2E Test"
echo "========================================"
echo ""
echo "This test demonstrates the wildcard (*) multi-command syntax:"
echo "  --p1-fixed-inputs='pass; *; attack silvercoat'"
echo ""
echo "Setup:"
echo "  Player 1: Silvercoat Lion (2/2)"
echo "  Player 2: No creatures"
echo ""
echo "Expected outcome:"
echo "  - P1 passes priority initially (first command)"
echo "  - Wildcard skips all priority passes until attack phase"
echo "  - P1 attacks with Silvercoat Lion when able"
echo "  - P2 takes damage (no blockers)"
echo ""
echo "Running test..."
echo ""

# Run the game with wildcard multi-command scripts using release binary
if OUTPUT=$(timeout 30s "$MTG_BIN" tui \
    --start-state "$WORKSPACE_ROOT/puzzles/wildcard_multicommand_e2e.pzl" \
    --p1=fixed \
    --p2=zero \
    --p1-fixed-inputs='pass; *; attack silvercoat' \
    --seed=300 \
    --verbosity=verbose 2>&1); then
    :  # Success - continue to verification
else
    EXIT_STATUS=$?
    if [[ $EXIT_STATUS == 124 ]]; then
        echo "✗ Test timed out after 30 seconds"
    else
        echo "✗ Game failed with exit code $EXIT_STATUS"
    fi
    echo ""
    echo "Output:"
    echo "$OUTPUT"
    exit 1
fi

echo "$OUTPUT" | grep -E "(Turn [0-9]|Silvercoat|Grizzly|attack|damage|Player 2 takes|Battlefield:|Winner)" | head -100

echo ""
echo "========================================"
echo "Verification"
echo "========================================"

# Check that Silvercoat Lion attacked
if echo "$OUTPUT" | grep -qi "silvercoat.*attack"; then
    echo "✓ Silvercoat Lion attacked"
else
    echo "✗ FAIL: Silvercoat Lion did not attack"
    echo ""
    echo "Full output:"
    echo "$OUTPUT"
    exit 1
fi

# Check that Player 2 took damage
if echo "$OUTPUT" | grep -qi "player 2 takes.*damage"; then
    echo "✓ Player 2 took damage from attack"
else
    echo "✗ FAIL: Player 2 did not take damage"
    echo ""
    echo "Full output:"
    echo "$OUTPUT"
    exit 1
fi

# Check that the game progressed past turn 1
if echo "$OUTPUT" | grep -qE "Turn [3-9]|Turn 1[0-9]"; then
    echo "✓ Game progressed multiple turns (wildcard skipped priority passes)"
else
    echo "✗ FAIL: Game did not progress properly"
    echo ""
    echo "Full output:"
    echo "$OUTPUT"
    exit 1
fi

echo ""
echo "========================================"
echo "Test PASSED"
echo "========================================"
echo ""
echo "Wildcard multi-command syntax working correctly!"
echo "  - Multiple commands executed in sequence"
echo "  - Wildcard separator skipped irrelevant priority passes"
echo "  - Attack command matched and executed at appropriate game phase"
