#!/usr/bin/env bash
# E2E test for HeuristicController - Grizzly Bears attacking
#
# This test verifies that the HeuristicController will attack with Grizzly Bears
# when the opponent has no blockers on the battlefield.
#
# Test scenario:
# - Player 1 (heuristic AI) has Grizzly Bears on battlefield
# - Player 2 has no creatures (cannot block)
# - Verify that Grizzly Bears attacks

set -euo pipefail

# Get script directory and source shared test helpers
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

# Ensure release binary is built
ensure_mtg_binary

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo "=== HeuristicController: Grizzly Bears Attack Test ==="
echo

echo

cd "$WORKSPACE_ROOT"

# Create test deck for attacker (minimal - just bears and lands)
ATTACKER_DECK="$WORKSPACE_ROOT/decks/heuristic_test_attacker.dck"
mkdir -p "$WORKSPACE_ROOT/decks"
cat > "$ATTACKER_DECK" << 'EOF'
[metadata]
Name=Heuristic Test - Attacker
Description=Minimal deck for testing Grizzly Bears attacking

[Main]
40 Forest
20 Grizzly Bears
EOF

# Create test deck for defender (no creatures - cannot block)
DEFENDER_DECK="$WORKSPACE_ROOT/decks/heuristic_test_defender_no_blockers.dck"
cat > "$DEFENDER_DECK" << 'EOF'
[metadata]
Name=Heuristic Test - Defender (No Creatures)
Description=Deck with no creatures to test attacking behavior

[Main]
60 Plains
EOF

echo "Created test decks:"
echo "  Attacker: $ATTACKER_DECK (Forest + Grizzly Bears)"
echo "  Defender: $DEFENDER_DECK (Plains only - no blockers)"
echo

# Check if cardsfolder exists
if [[ ! -d "$WORKSPACE_ROOT/cardsfolder" ]]; then
    echo -e "${YELLOW}Warning: cardsfolder not found, skipping test${NC}"
    exit 0
fi

echo "Running game: Heuristic AI (with Grizzly Bears) vs Zero AI (no creatures)"
echo "Seed: 100 (deterministic)"
echo "Looking for evidence of Grizzly Bears attacking..."
echo

# Run the game with heuristic AI as P1, zero AI as P2
# Use verbose output to see attack declarations
# Note: Using run_mtg_with_timeout instead of timeout for network mode compatibility
if run_mtg_with_timeout 30 tui \
    "$ATTACKER_DECK" \
    "$DEFENDER_DECK" \
    --p1 heuristic \
    --p2 zero \
    --seed 100 \
    --verbosity verbose \
    > /tmp/heuristic_attack_test.txt 2>&1; then

    echo -e "${GREEN}✓ Game completed successfully${NC}"
    echo

    # Check output for attack patterns
    # Look for "Grizzly Bears" and "attack" or "attacking" or "Declare Attackers"
    #
    # mtg-717 SIGPIPE hardening: under `set -o pipefail` (line 12), a `grep …|
    # grep -qi …` or `grep …| head` pipe makes the early-exiting consumer (grep
    # -q / head close the pipe on first match / Nth line) deliver SIGPIPE to the
    # still-writing upstream grep → the pipe reports 141 → pipefail+`set -e`
    # killed the whole test intermittently (exit 141). Capture the filtered text
    # into vars first (the `grep|grep` there reads ALL input, no early exit), and
    # guard the cosmetic `| head` display pipes with `|| true`. Behaviour is
    # identical; the flake is gone.
    attack_lines=$(grep -i "grizzly bears" /tmp/heuristic_attack_test.txt | grep -i "attack" || true)
    declare_lines=$(grep -i "declare.*attacker" /tmp/heuristic_attack_test.txt || true)
    bears_lines=$(grep -i "grizzly bears" /tmp/heuristic_attack_test.txt || true)
    if [ -n "$attack_lines" ]; then
        echo -e "${GREEN}✓ SUCCESS: Grizzly Bears attacked as expected${NC}"
        echo
        echo "Evidence from game log:"
        printf '%s\n' "$attack_lines" | head -5 || true
        EXIT_CODE=0
    elif [ -n "$declare_lines" ] && [ -n "$bears_lines" ]; then
        echo -e "${GREEN}✓ SUCCESS: Attack phase occurred with Grizzly Bears on battlefield${NC}"
        echo
        echo "Evidence from game log:"
        echo "Attack declarations:"
        printf '%s\n' "$declare_lines" | head -3 || true
        echo "Grizzly Bears mentions:"
        printf '%s\n' "$bears_lines" | head -3 || true
        EXIT_CODE=0
    else
        echo -e "${RED}✗ FAILURE: No evidence of Grizzly Bears attacking${NC}"
        echo
        echo "Game log excerpt (first 100 lines):"
        head -100 /tmp/heuristic_attack_test.txt
        echo "..."
        echo "Full log saved to /tmp/heuristic_attack_test.txt"
        EXIT_CODE=1
    fi
else
    EXIT_STATUS=$?
    if [[ $EXIT_STATUS == 124 ]]; then
        echo -e "${RED}✗ Test timed out after 30 seconds${NC}"
    else
        echo -e "${RED}✗ Game failed with exit code $EXIT_STATUS${NC}"
    fi
    echo
    echo "Output (first 100 lines):"
    head -100 /tmp/heuristic_attack_test.txt
    EXIT_CODE=1
fi

echo
echo "=== Test Complete ==="
echo "Full log available at: /tmp/heuristic_attack_test.txt"
exit $EXIT_CODE
