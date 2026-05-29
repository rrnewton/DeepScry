#!/usr/bin/env bash
# E2E test: Chaos Orb FlipOntoBattlefield targets opponent permanents
#
# Regression test for mtg-392 (was: mtg-389 compat issue).
# Before the fix, Chaos Orb's FlipOntoBattlefield was a `DestroyPermanent`
# with `requires_nontoken=true` and no controller restriction. The activation
# auto-targeted the first valid nontoken permanent which was the Orb itself,
# never any opponent permanent.
#
# After the fix the converter sets `restriction.controller = OppCtrl`, so
# valid_targets only contains opponent permanents, and the Orb correctly
# destroys an opponent permanent (in addition to self-destroying via the
# Defined$ Self subability chain).
#
# Test scenario:
# - Player 1 has Chaos Orb on the battlefield + 2 Plains for {1} cost
# - Player 2 has Mountain + Grizzly Bears
# - Activate Chaos Orb; verify both an opponent permanent AND Chaos Orb are
#   moved to a graveyard.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Chaos Orb: Targets Opponent Permanent E2E Test ==="
echo

cd "$WORKSPACE_ROOT"

if [[ ! -d "$WORKSPACE_ROOT/cardsfolder" ]]; then
    echo -e "${RED}Error: $WORKSPACE_ROOT/cardsfolder not found${NC}"
    exit 1
fi

PUZZLE="$WORKSPACE_ROOT/test_puzzles/chaos_orb_destroys_target.pzl"
if [[ ! -f "$PUZZLE" ]]; then
    echo -e "${RED}Error: puzzle not found: $PUZZLE${NC}"
    exit 1
fi

LOG=/tmp/chaos_orb_targets_opponent_test.txt

if run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE" \
    --p1=fixed \
    --p2=zero \
    --p1-fixed-inputs="activate Chaos Orb" \
    --p2-fixed-inputs="" \
    --stop-on-choice=4 \
    --json \
    --seed 42 \
    --verbosity 3 \
    > "$LOG" 2>&1; then
    echo -e "${GREEN}✓ Game completed${NC}"
else
    EXIT_STATUS=$?
    echo -e "${RED}✗ Game failed (exit $EXIT_STATUS)${NC}"
    head -80 "$LOG"
    exit 1
fi

# Required: Chaos Orb's flip targeted an opponent permanent (Mountain or Grizzly Bears)
if grep -qE "^  *-> targeting (Mountain|Grizzly Bears) " "$LOG"; then
    echo -e "${GREEN}✓ Chaos Orb targeted an opponent permanent${NC}"
    grep -E "^  *-> targeting " "$LOG" | head -3
else
    echo -e "${RED}✗ Chaos Orb did NOT target an opponent permanent${NC}"
    echo "Targeting log lines:"
    grep -E "^  *-> targeting " "$LOG" || echo "(none)"
    exit 1
fi

# Required: opponent permanent moved to graveyard from Chaos Orb's effect
if grep -qE "^  (Mountain|Grizzly Bears) \([0-9]+\) goes to graveyard" "$LOG"; then
    echo -e "${GREEN}✓ Opponent permanent moved to graveyard${NC}"
    grep -E " goes to graveyard" "$LOG" | head -5
else
    echo -e "${RED}✗ No opponent permanent went to graveyard${NC}"
    echo "Movement log lines:"
    grep -E " goes to graveyard" "$LOG" || echo "(none)"
    exit 1
fi

# Required: Chaos Orb itself destroyed via Defined$ Self subability chain
if grep -qE "^  Chaos Orb \([0-9]+\) goes to graveyard" "$LOG"; then
    echo -e "${GREEN}✓ Chaos Orb self-destroyed${NC}"
else
    echo -e "${RED}✗ Chaos Orb did not self-destroy${NC}"
    exit 1
fi

# Required: Chaos Orb did NOT pick itself as the flip target
if grep -qE "^  *-> targeting Chaos Orb " "$LOG"; then
    echo -e "${RED}✗ Regression: Chaos Orb auto-targeted itself${NC}"
    exit 1
fi

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "Full log: $LOG"
exit 0
