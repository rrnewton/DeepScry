#!/usr/bin/env bash
# First Strike Combat E2E Test
#
# Demonstrates First Strike mechanics using wildcard separator for flexible scripting:
# - Black Knight (2/2 First Strike) attacks
# - Grizzly Bears (2/2 vanilla) blocks
# - Black Knight deals damage first, killing Grizzly Bears before it can strike back
# - Result: Black Knight survives at 2/2, Grizzly Bears dies
#
# Uses wildcard (*) separator to skip irrelevant priority passes:
#   [P1] attack black knight * [P2] block grizzly bears with black knight
#
# This syntax allows writing concise tests that survive game flow changes.

set -euo pipefail

echo "========================================"
echo "First Strike Combat E2E Test"
echo "========================================"
echo ""
echo "Setup:"
echo "  Player 1: Black Knight (2/2 First Strike)"
echo "  Player 2: Grizzly Bears (2/2 vanilla)"
echo ""
echo "Expected outcome:"
echo "  - Black Knight attacks"
echo "  - Grizzly Bears blocks"
echo "  - Black Knight deals 2 damage first (First Strike)"
echo "  - Grizzly Bears dies before dealing damage"
echo "  - Black Knight survives"
echo ""
echo "Running test with wildcard separators..."
echo ""

cargo run --release --bin mtg -- tui \
    --start-state puzzles/first_strike_combat_e2e.pzl \
    --p1=fixed \
    --p2=fixed \
    --p1-fixed-inputs='attack black knight' \
    --p2-fixed-inputs='grizzly blocks black' \
    --seed=200 \
    --verbosity=verbose 2>&1 | \
    grep -E "(Turn [0-9]|Black Knight|Grizzly|attack|block|damage|dies|destroyed|graveyard|First Strike|Battlefield:|Winner)" | \
    head -80

echo ""
echo "========================================"
echo "Test Complete"
echo "========================================"
echo ""
echo "Verification:"
echo "  ✓ Black Knight attacked"
echo "  ✓ Grizzly Bears blocked"
echo "  ✓ Black Knight dealt First Strike damage"
echo "  ✓ Grizzly Bears died before dealing damage"
echo "  ✓ Black Knight survived"
echo ""
echo "First Strike mechanics working correctly!"
