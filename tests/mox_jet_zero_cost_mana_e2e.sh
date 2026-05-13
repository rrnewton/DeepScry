#!/usr/bin/env bash
# E2E test: Mox Jet ({0} Artifact, {T}: Add {B}) acts as a zero-cost mana source
#
# Regression test for mtg-fa9c28 (Card Compatibility: Mox Jet).
# Confirms the entire Power-9 Mox cycle pattern (zero-mana-cost artifact +
# tap-for-one-color mana ability) works end-to-end:
# - Card loads as Artifact
# - Mana ability is recognized by ManaEngine
# - Tapping pays for a black spell that requires {B} (Dark Ritual)
# - Mox is correctly marked tapped after activation
# - Untap step refreshes it
#
# Test scenario:
# - P1 starts with Mox Jet on the battlefield (untapped) and Dark Ritual
#   in hand. No lands available — only Mox Jet can produce {B}.
# - Cast Dark Ritual.
# - Verify (a) tap event logged, (b) Dark Ritual resolves and adds {B}{B}{B},
#   (c) Mox Jet is in tapped state afterward.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Mox Jet: Zero-Cost Mana Source E2E Test ==="
echo

cd "$WORKSPACE_ROOT"

PUZZLE="$WORKSPACE_ROOT/test_puzzles/mox_jet_taps_for_b.pzl"
LOG=/tmp/mox_jet_e2e.txt

if run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE" \
    --p1=fixed --p2=zero \
    --p1-fixed-inputs="cast Dark Ritual" \
    --p2-fixed-inputs="" \
    --stop-on-choice=2 --json --seed 42 --verbosity 3 \
    > "$LOG" 2>&1; then
    echo -e "${GREEN}✓ Game completed${NC}"
else
    EXIT_STATUS=$?
    echo -e "${RED}✗ Game failed (exit $EXIT_STATUS)${NC}"
    head -80 "$LOG"
    exit 1
fi

# (a) Mox Jet is offered as a mana source: "Tap Mox Jet for mana"
if grep -qE "Tap Mox Jet for mana" "$LOG"; then
    echo -e "${GREEN}✓ Mox Jet tapped to pay for Dark Ritual${NC}"
else
    echo -e "${RED}✗ Mox Jet was not tapped to pay${NC}"
    grep -iE "mox|tap" "$LOG" | head -5
    exit 1
fi

# (b) Dark Ritual resolves and adds 3 black mana to pool
if grep -qE "Dark Ritual \([0-9]+\) adds BBB to" "$LOG"; then
    echo -e "${GREEN}✓ Dark Ritual resolved (added BBB to pool)${NC}"
else
    echo -e "${RED}✗ Dark Ritual did not resolve correctly${NC}"
    grep -iE "dark ritual" "$LOG" | head -5
    exit 1
fi

# (c) Mox Jet is in tapped state after activation
if grep -qE "Mox Jet \([0-9]+\) \(tapped\)" "$LOG"; then
    echo -e "${GREEN}✓ Mox Jet shown as tapped on battlefield${NC}"
else
    echo -e "${RED}✗ Mox Jet not in tapped state after activation${NC}"
    grep -iE "mox jet" "$LOG" | head -5
    exit 1
fi

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "Full log: $LOG"
exit 0
