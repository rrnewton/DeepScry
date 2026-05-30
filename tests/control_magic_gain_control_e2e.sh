#!/usr/bin/env bash
# E2E test: Control Magic ({2}{U}{U} Enchantment Aura) steals control of the
# enchanted creature, and control reverts when the Aura is destroyed.
#
# Regression test for the "Card Compatibility: Control Magic" beads issue
# (mtg-493). Control Magic is:
#   S:Mode$ Continuous | Affected$ Card.EnchantedBy | GainControl$ You
# which parses into StaticAbility::GainControl and is applied as a continuous
# CR 613.2 layer-2 control change in GameState::recompute_aura_control().
#
# Scenario (test_puzzles/control_magic_steal_revert.pzl):
# - P1 hand: Control Magic, Disenchant. P1 board: City of Brass x6.
# - P2 board: Savannah Lions.
# - P1 casts Control Magic on the Lions -> it "comes under P1's control".
# - P1 casts Disenchant on its own Control Magic -> the Aura is destroyed and
#   the Lions "returns to P2's control".
#
# This generalizes to every control-stealing Aura (Mind Control, Persuasion,
# Enslave, Confiscate, ...).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Control Magic Gain-Control E2E Test ==="
echo

cd "$WORKSPACE_ROOT"

PUZZLE="$WORKSPACE_ROOT/test_puzzles/control_magic_steal_revert.pzl"
LOG=/tmp/control_magic_gain_control_e2e.txt

# The trailing fixed inputs run out of meaningful actions after the two casts;
# that is expected. We capture the log and grep for the control-change lines.
run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE" \
    --p1=fixed --p2=zero \
    --p1-fixed-inputs="cast Control Magic;cast Disenchant;Control Magic;*" \
    --p2-fixed-inputs="" \
    --seed 3 --verbosity 3 \
    > "$LOG" 2>&1 || true

# (a) Control Magic steals the creature.
if grep -qE "Savannah Lions comes under .*'s control" "$LOG"; then
    echo -e "${GREEN}✓ Control Magic transferred control of Savannah Lions${NC}"
else
    echo -e "${RED}✗ Control Magic did not transfer control${NC}"
    grep -iE "control magic|comes under|enchants" "$LOG" | head -8
    exit 1
fi

# (b) Disenchant destroys the Aura.
if grep -qE "destroys Control Magic" "$LOG"; then
    echo -e "${GREEN}✓ Disenchant destroyed Control Magic${NC}"
else
    echo -e "${RED}✗ Control Magic was not destroyed${NC}"
    grep -iE "disenchant|destroy" "$LOG" | head -8
    exit 1
fi

# (c) Control reverts to the original controller when the Aura leaves.
if grep -qE "Savannah Lions returns to .*'s control" "$LOG"; then
    echo -e "${GREEN}✓ Control reverted to original controller${NC}"
else
    echo -e "${RED}✗ Control did not revert after the Aura left${NC}"
    grep -iE "returns to|control" "$LOG" | head -8
    exit 1
fi

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "Full log: $LOG"
exit 0
