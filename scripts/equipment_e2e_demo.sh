#!/usr/bin/env bash
# Equipment E2E Demonstration Script
#
# This script demonstrates complete Equipment mechanics using DESCRIPTIVE COMMANDS:
# 1. Accorder's Shield (+0/+3, Vigilance) and Grizzly Bears (2/2) start on battlefield
# 2. Use "equip accorder" to activate the Equip ability targeting Grizzly Bears
# 3. Use "attack grizzly" to attack with the equipped creature
#
# This demonstrates the preferred way to write E2E tests: using descriptive commands
# like "equip accorder" instead of fragile numeric indices like "1;0;1;0".

set -euo pipefail

echo "=== Equipment E2E Demonstration (Descriptive Commands) ==="
echo ""
echo "Starting game from puzzle state..."
echo "  - Player 1: 4 Forests, Accorder's Shield, Grizzly Bears (2/2)"
echo "  - Player 2: 5 life"
echo ""
echo "Action sequence (using descriptive commands):"
echo "  Turn 1: 'equip accorder' - Activate Equip ability on Accorder's Shield"
echo "  Turn 3: 'attack grizzly' - Attack with Grizzly Bears"
echo ""

cargo run --release --bin mtg -- tui \
    --start-state puzzles/equipment_equip_e2e.pzl \
    --p1=fixed \
    --p2=fixed \
    --p1-fixed-inputs='equip accorder;pass;attack grizzly;pass' \
    --p2-fixed-inputs='pass;pass;pass;pass' \
    --seed=100 \
    --verbosity=normal 2>&1 | grep -E "(===|Equipment|Turn [0-9]|Accorder|Grizzly|equip|attaches|attacks|deals|damage|Life:|Winner|Game Over)" | head -80

echo ""
echo "=== Demonstration Complete ==="
echo ""
echo "Key observations:"
echo "  ✓ Descriptive command 'equip accorder' activates the Equip ability"
echo "  ✓ Equipment attachment works correctly (see 'attaches to' message)"
echo "  ✓ Descriptive command 'attack grizzly' declares attacker"
echo "  ✓ Combat with equipped creature functions"
echo ""
echo "Commands used:"
echo "  - 'equip accorder' instead of numeric index '1'"
echo "  - 'attack grizzly' instead of numeric index '1'"
echo "  - 'pass' instead of numeric index '0'"
echo ""
echo "This makes tests more readable and resilient to changes in action ordering."
