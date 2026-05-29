#!/usr/bin/env bash
# E2E test: Nevinyrral's Disk ({1}, {T}: Destroy all artifacts, creatures, and
# enchantments) and its enters-tapped replacement effect.
#
# Regression test for the "Card Compatibility: Nevinyrral's Disk" beads issue
# (mtg-528). Script:
#   R:Event$ Moved | ValidCard$ Card.Self | Destination$ Battlefield
#     | ReplaceWith$ ETBTapped
#   A:AB$ DestroyAll | Cost$ 1 T | ValidCards$ Artifact,Creature,Enchantment
#
# Scenario (test_puzzles/nevinyrrals_disk_destroyall.pzl):
# - P1 board: an UNTAPPED Disk + Plains x2 + a Grizzly Bears.
# - P2 board: Grizzly Bears + Sol Ring.
# - P1 activates the Disk: the Disk itself, both Grizzly Bears, and the Sol
#   Ring are all destroyed and move to their owners' graveyards.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Nevinyrral's Disk DestroyAll E2E Test ==="
echo

cd "$WORKSPACE_ROOT"

PUZZLE="$WORKSPACE_ROOT/test_puzzles/nevinyrrals_disk_destroyall.pzl"
LOG=/tmp/nevinyrrals_disk_destroyall_e2e.txt

if run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE" \
    --p1=fixed --p2=zero \
    --p1-fixed-inputs="activate Nevinyrral's Disk;*;*;*" \
    --p2-fixed-inputs="" \
    --stop-on-choice=6 --seed 42 --verbosity 3 \
    > "$LOG" 2>&1; then
    echo -e "${GREEN}✓ Game completed${NC}"
else
    EXIT_STATUS=$?
    echo -e "${RED}✗ Game failed (exit $EXIT_STATUS)${NC}"
    head -80 "$LOG"
    exit 1
fi

# (a) The Disk's board-wipe activates
if grep -qE "Destroy all artifacts, creatures, and enchantments" "$LOG"; then
    echo -e "${GREEN}✓ DestroyAll activated${NC}"
else
    echo -e "${RED}✗ DestroyAll did not activate${NC}"
    grep -iE "disk|destroy" "$LOG" | head -8
    exit 1
fi

# (b) A creature is destroyed
if grep -qE "Grizzly Bears \([0-9]+\) is destroyed" "$LOG"; then
    echo -e "${GREEN}✓ Creature (Grizzly Bears) destroyed${NC}"
else
    echo -e "${RED}✗ Creature not destroyed${NC}"
    grep -iE "destroy|grizzly" "$LOG" | head -8
    exit 1
fi

# (c) An artifact (Sol Ring) is destroyed
if grep -qE "Sol Ring \([0-9]+\) is destroyed" "$LOG"; then
    echo -e "${GREEN}✓ Artifact (Sol Ring) destroyed${NC}"
else
    echo -e "${RED}✗ Artifact not destroyed${NC}"
    grep -iE "destroy|sol ring" "$LOG" | head -8
    exit 1
fi

# (d) The Disk itself is destroyed (it is an artifact)
if grep -qE "Nevinyrral's Disk \([0-9]+\) is destroyed" "$LOG"; then
    echo -e "${GREEN}✓ The Disk destroyed itself${NC}"
else
    echo -e "${RED}✗ The Disk did not destroy itself${NC}"
    grep -iE "destroy|disk" "$LOG" | head -8
    exit 1
fi

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "Full log: $LOG"
exit 0
