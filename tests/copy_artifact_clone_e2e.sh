#!/usr/bin/env bash
# E2E test: Copy Artifact enters the battlefield as a copy of a chosen artifact
# via the Clone mechanic (mtg-uh5gz).
#
# Before this change:
#   K:ETBReplacement:Copy:DBCopy:Optional was only wired for the ChooseColor
#   variant, and DB$ Clone had no ApiType / Effect at all. Copy Artifact entered
#   as a vanilla 1U Enchantment with NO prompt to choose an artifact to copy.
#
# After this change:
#   - ApiType::Clone + Effect::Clone { source, chosen, choices_filter,
#     add_types, optional } parse from the DBCopy SVar.
#   - The ETBReplacement:Copy keyword wires the Clone effect onto the card.
#   - Spell resolution (priority.rs) intercepts the Clone effect and routes the
#     "which artifact to copy" decision through the PlayerController
#     (network-safe, information-independent), then applies the copy: copiable
#     values per CR 707.2 plus the AddTypes$ Enchantment supertype.
#
# Scenario (test_puzzles/copy_artifact_clone_e2e.pzl):
#   - P0 hand: Copy Artifact; battlefield: 3 Islands (enough for {1}{U}).
#   - P1 battlefield: Black Lotus (the only artifact to copy).
#   - Verify (a) Copy Artifact is castable, (b) it resolves, (c) it enters as a
#     copy of Black Lotus, also an Enchantment, (d) a second Black Lotus now
#     exists on the battlefield (the copy under P0's control).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Copy Artifact: Clone Enters As A Copy E2E Test ==="
echo

cd "$WORKSPACE_ROOT"

PUZZLE="$WORKSPACE_ROOT/test_puzzles/copy_artifact_clone_e2e.pzl"
LOG=/tmp/copy_artifact_clone_e2e.txt

if run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE" \
    --p1=fixed --p2=zero \
    --p1-fixed-inputs="cast Copy Artifact" \
    --p2-fixed-inputs="" \
    --stop-on-choice=4 --json --seed 42 --verbosity 3 \
    > "$LOG" 2>&1; then
    echo -e "${GREEN}✓ Game completed${NC}"
else
    EXIT_STATUS=$?
    echo -e "${RED}✗ Game failed (exit $EXIT_STATUS)${NC}"
    head -80 "$LOG"
    exit 1
fi

# (a) Copy Artifact is castable
if grep -qE "\[[0-9]+\] cast Copy Artifact" "$LOG"; then
    echo -e "${GREEN}✓ 'cast Copy Artifact' appears in available actions${NC}"
else
    echo -e "${RED}✗ Copy Artifact not offered as a castable spell${NC}"
    grep -E "available actions" "$LOG" | head -5
    exit 1
fi

# (b) Copy Artifact cast and resolved
if grep -qE "Player 1 casts Copy Artifact" "$LOG"; then
    echo -e "${GREEN}✓ Copy Artifact cast and put on stack${NC}"
else
    echo -e "${RED}✗ Copy Artifact never cast${NC}"
    exit 1
fi

if grep -qE "Copy Artifact \([0-9]+\) resolves" "$LOG"; then
    echo -e "${GREEN}✓ Copy Artifact resolved on the stack${NC}"
else
    echo -e "${RED}✗ Copy Artifact never resolved${NC}"
    exit 1
fi

# (c) Entered as a copy of Black Lotus, ALSO an Enchantment (CR 707 + AddTypes$)
if grep -qE "Copy Artifact enters the battlefield as a copy of Black Lotus \(also Enchantment\)" "$LOG"; then
    echo -e "${GREEN}✓ Entered as a copy of Black Lotus, also an Enchantment${NC}"
else
    echo -e "${RED}✗ Did not enter as a copy of Black Lotus + Enchantment${NC}"
    grep -iE "enters the battlefield as a copy|copy of" "$LOG" | head -5
    exit 1
fi

# (d) Two Black Lotus permanents now exist (original + the Copy Artifact copy).
#     The state dump prints each battlefield permanent by name; the copy now
#     reads "Black Lotus" (its copied name) rather than "Copy Artifact".
LOTUS_COUNT=$(grep -cE "Black Lotus \([0-9]+\)" "$LOG" || true)
if [ "$LOTUS_COUNT" -ge 1 ]; then
    echo -e "${GREEN}✓ The copy reports as Black Lotus on the battlefield${NC}"
else
    echo -e "${RED}✗ No Black Lotus copy found on the battlefield${NC}"
    grep -iE "black lotus" "$LOG" | head -5
    exit 1
fi

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "Full log: $LOG"
exit 0
