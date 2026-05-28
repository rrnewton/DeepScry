#!/usr/bin/env bash
# E2E test: The Abyss destroys the active player's nonartifact creature each upkeep.
#
# Regression test for mtg-sgkjv (Card Compatibility: The Abyss).
#
# The Abyss: World Enchantment, "At the beginning of each player's upkeep,
# destroy target nonartifact creature that player controls of their choice.
# It can't be regenerated."
#   T:Mode$ Phase | Phase$ Upkeep | ValidPlayer$ Player | Execute$ TrigDestroy
#   SVar:TrigDestroy:DB$ Destroy | ValidTgts$ Creature.nonArtifact+ActivePlayerCtrl | NoRegen$ True
#
# Before the fix the Phase-trigger Execute$ handler had a hardcoded ApiType
# allowlist that did NOT include Destroy, so the upkeep trigger fired but
# silently did nothing (placeholder target never resolved → fizzle). After
# the fix:
#   - the Destroy effect is parsed (via the shared params_to_effect converter)
#   - TargetRestriction honors nonArtifact (requires_nonartifact) and
#     ActivePlayerCtrl (target must be the active player's creature)
#   - NoRegen$ True sets no_regenerate so the creature can't be regenerated
#   - check_triggers_for_controller resolves the target among the ACTIVE
#     player's matching creatures
#
# Scenario (test_puzzles/the_abyss_upkeep_destroy.pzl):
#   P1 (active) controls The Abyss + Grizzly Bears (nonartifact) + Ornithopter
#     (artifact creature). P2 controls Grizzly Bears.
# On P1's upkeep, The Abyss must destroy P1's Grizzly Bears, NOT the artifact
# Ornithopter, and NOT P2's creature.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== The Abyss: Upkeep Destroy E2E Test ==="
echo

cd "$WORKSPACE_ROOT"

PUZZLE="$WORKSPACE_ROOT/test_puzzles/the_abyss_upkeep_destroy.pzl"
if [[ ! -f "$PUZZLE" ]]; then
    echo -e "${RED}Error: puzzle not found: $PUZZLE${NC}"
    exit 1
fi

FULL_LOG=/tmp/the_abyss_upkeep_destroy_test.txt

if run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE" \
    --p1=zero \
    --p2=zero \
    --p1-fixed-inputs="" \
    --p2-fixed-inputs="" \
    --stop-on-choice=2 \
    --seed 42 \
    --verbosity 3 \
    > "$FULL_LOG" 2>&1; then
    echo -e "${GREEN}✓ Game completed${NC}"
else
    EXIT_STATUS=$?
    echo -e "${RED}✗ Game failed (exit $EXIT_STATUS)${NC}"
    head -100 "$FULL_LOG"
    exit 1
fi

# Slice out ONLY Turn 1 (Player 1's upkeep). The Abyss fires on EACH player's
# upkeep, so later turns legitimately destroy other players' creatures; this
# test asserts the active-player / nonartifact behavior on the FIRST upkeep.
LOG=/tmp/the_abyss_upkeep_destroy_turn1.txt
awk '/Turn 1 - Player 1/{f=1} /Turn 2 - Player 2/{f=0} f' "$FULL_LOG" > "$LOG"
if [[ ! -s "$LOG" ]]; then
    echo -e "${RED}Error: could not isolate Turn 1 section from log${NC}"
    head -100 "$FULL_LOG"
    exit 1
fi

# Required: the upkeep trigger fired on Turn 1.
if grep -qF "Trigger: The Abyss" "$LOG"; then
    echo -e "${GREEN}✓ The Abyss upkeep trigger fired${NC}"
else
    echo -e "${RED}✗ The Abyss upkeep trigger did NOT fire on Turn 1${NC}"
    exit 1
fi

# Required: P1's nonartifact creature (Grizzly Bears (8)) was destroyed.
if grep -qE "^  Grizzly Bears \(8\) goes to graveyard" "$LOG"; then
    echo -e "${GREEN}✓ The Abyss destroyed the active player's nonartifact creature${NC}"
    grep -E "goes to graveyard" "$LOG" | head -3
else
    echo -e "${RED}✗ The Abyss did NOT destroy P1's Grizzly Bears${NC}"
    echo "Graveyard log lines:"
    grep -E "goes to graveyard" "$LOG" || echo "(none)"
    exit 1
fi

# Required: the artifact creature (Ornithopter (9)) was NOT destroyed
# (nonArtifact restriction excludes it).
if grep -qE "^  Ornithopter \(9\) goes to graveyard" "$LOG"; then
    echo -e "${RED}✗ Regression: The Abyss wrongly destroyed an ARTIFACT creature${NC}"
    exit 1
else
    echo -e "${GREEN}✓ The artifact creature (Ornithopter) was correctly spared${NC}"
fi

# Required: P2's creature (Grizzly Bears (17)) was NOT destroyed on P1's upkeep
# (ActivePlayerCtrl restricts to the active player's creatures).
if grep -qE "^  Grizzly Bears \(17\) goes to graveyard" "$LOG"; then
    echo -e "${RED}✗ Regression: The Abyss destroyed a NON-active player's creature${NC}"
    exit 1
else
    echo -e "${GREEN}✓ The non-active player's creature was correctly spared${NC}"
fi

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "Full log: $LOG"
exit 0
