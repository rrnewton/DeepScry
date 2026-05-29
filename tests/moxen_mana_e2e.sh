#!/usr/bin/env bash
# E2E test: Power-9 Moxen Mox Ruby / Mox Emerald / Mox Pearl tap for their
# printed single colour.
#
# Regression test for the "Card Compatibility: Mox Ruby / Mox Emerald /
# Mox Pearl" beads issues (mtg-526 / mtg-524 / mtg-525), siblings of the
# already-WORKING Mox Jet (mtg-405). All four are zero-cost artifacts with
# a single "{T}: Add {color}" mana ability.
#
# Test scenario (test_puzzles/moxen_mana.pzl):
# - P1 board: Mox Ruby, Mox Emerald, Mox Pearl, Swamp. NO other red/green
#   source exists.
# - Cast Lightning Bolt ({R}) => forces Mox Ruby to tap.
# - Cast Giant Growth  ({G})  => forces Mox Emerald to tap.
# - Cast Disenchant ({1}{W})  => consumes Mox Pearl's {W} (Swamp pays {1}).
# All three spells resolve, proving each Mox produces its printed colour.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Power-9 Moxen (Ruby / Emerald / Pearl) Mana E2E Test ==="
echo

cd "$WORKSPACE_ROOT"

PUZZLE="$WORKSPACE_ROOT/test_puzzles/moxen_mana.pzl"
LOG=/tmp/moxen_mana_e2e.txt

if run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE" \
    --p1=fixed --p2=zero \
    --p1-fixed-inputs="cast Lightning Bolt;*;cast Giant Growth;*;cast Disenchant;*;*" \
    --p2-fixed-inputs="" \
    --stop-on-choice=14 --seed 42 --verbosity 3 \
    > "$LOG" 2>&1; then
    echo -e "${GREEN}✓ Game completed${NC}"
else
    EXIT_STATUS=$?
    echo -e "${RED}✗ Game failed (exit $EXIT_STATUS)${NC}"
    head -80 "$LOG"
    exit 1
fi

check_line() {
    local pattern="$1" desc="$2"
    if grep -qE "$pattern" "$LOG"; then
        echo -e "${GREEN}✓ $desc${NC}"
    else
        echo -e "${RED}✗ $desc${NC}"
        grep -iE "mox|tap|lightning|giant|disenchant" "$LOG" | head -10
        exit 1
    fi
}

check_line "Tap Mox Ruby for mana"    "Mox Ruby tapped (pays {R} Lightning Bolt)"
check_line "Tap Mox Emerald for mana" "Mox Emerald tapped (pays {G} Giant Growth)"
check_line "Tap Mox Pearl for mana"   "Mox Pearl tapped (pays {W} Disenchant)"
check_line "Lightning Bolt \([0-9]+\) deals 3 damage" "Lightning Bolt resolved (Mox Ruby {R})"
check_line "Giant Growth \([0-9]+\) gives"            "Giant Growth resolved (Mox Emerald {G})"

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "Full log: $LOG"
exit 0
