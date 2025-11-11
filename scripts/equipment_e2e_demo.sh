#!/usr/bin/env bash
# Equipment End-to-End Demonstration Script
#
# This script demonstrates Equipment working in actual gameplay via the MTG CLI.
# It uses the Fixed controller with pre-programmed choices to show:
#   1. Player1 casts Bonesplitter Equipment (+2/+0, Equip {1})
#   2. Player1 casts Grizzly Bears (2/2 creature)
#   3. Player1 activates Equip ability, targeting Grizzly Bears
#   4. Grizzly Bears becomes 4/2 (base 2/2 + Equipment +2/+0)
#   5. Player1 attacks with equipped Grizzly Bears
#   6. Grizzly Bears deals 4 damage (buffed from base 2)

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

echo "========================================"
echo "MTG Forge - Equipment E2E Demonstration"
echo "========================================"
echo ""
echo "This demonstrates the complete Equipment workflow:"
echo "  - Cast Bonesplitter Equipment (+2/+0, costs {1}, Equip {1})"
echo "  - Cast Grizzly Bears creature (2/2, costs {1}{G})"
echo "  - Activate Equip ability to attach Bonesplitter"
echo "  - Creature becomes 4/2 (2/2 base + 2/0 Equipment bonus)"
echo "  - Attack with equipped creature dealing 4 damage"
echo ""
echo "Deck: 24 Forest, 16 Grizzly Bears, 20 Bonesplitter"
echo "Seed: 200 (chosen to get good opening hand)"
echo ""
echo "Running game..."
echo ""

# Player1 choices (semicolon-separated):
# Turn 1: Play Forest (1), Cast Bonesplitter (1)
# Turn 2 (P2): Pass (0)
# Turn 3: Play Forest (1), Cast Grizzly Bears (1)
# Turn 4 (P2): Pass (0)
# Turn 5: Activate Equip (3), Choose target Bears (0 - auto if only 1 creature), Attack with Bears (1)

cargo run --release --bin mtg -- tui \
    decks/equipment_test.dck \
    decks/equipment_test.dck \
    --p1=fixed \
    --p2=fixed \
    --p1-fixed-inputs="1;1;0;1;1;0;3;0;1;0" \
    --p2-fixed-inputs="0;0;0;0;0;0" \
    --seed=200 \
    2>&1 | grep -E "(Turn [0-9]+ -|Player1 (plays|casts|activates|declares)|Bonesplitter.*attaches|Grizzly Bears.*(enters|deals|declares)|attaches to|deals [0-9]+ damage|Player2: [0-9]+ life|SCRIPT chose|P/T:)" | head -100

echo ""
echo "========================================"
echo "Equipment E2E Demonstration Complete"
echo "========================================"
