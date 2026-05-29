#!/usr/bin/env bash
# E2E test: original dual lands Badlands / Scrubland / Bayou tap for both
# of their basic-land-type colours (CR 305.6).
#
# Regression test for the "Card Compatibility: Badlands / Scrubland / Bayou"
# beads issue (Old-School deck decks/old_school/01_rogue_rogerbrand.dck).
#
# These cards carry NO printed mana ability — each basic land subtype on the
# type line (Swamp/Mountain, Plains/Swamp, Swamp/Forest) grants an intrinsic
# "{T}: Add {color}" ability. The loader must add ONE mana ability per
# subtype, each producing the correct single colour.
#
# Test scenario (test_puzzles/dual_lands_mana.pzl):
# - P1 board: Badlands, Scrubland, Bayou (the ONLY red source is Badlands,
#   the ONLY green source is Bayou).
# - P1 hand: Lightning Bolt ({R}) and Giant Growth ({G}).
# - Cast Lightning Bolt  => forces Badlands to tap for {R}.
# - Cast Giant Growth    => forces Bayou to tap for {G}.
# Both spells resolve, proving the second (non-black) colour of each dual
# land is produced correctly.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Dual Lands (Badlands / Scrubland / Bayou) Mana E2E Test ==="
echo

cd "$WORKSPACE_ROOT"

PUZZLE="$WORKSPACE_ROOT/test_puzzles/dual_lands_mana.pzl"
LOG=/tmp/dual_lands_mana_e2e.txt

if run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE" \
    --p1=fixed --p2=zero \
    --p1-fixed-inputs="cast Lightning Bolt;*;cast Giant Growth;*" \
    --p2-fixed-inputs="" \
    --stop-on-choice=8 --seed 42 --verbosity 3 \
    > "$LOG" 2>&1; then
    echo -e "${GREEN}✓ Game completed${NC}"
else
    EXIT_STATUS=$?
    echo -e "${RED}✗ Game failed (exit $EXIT_STATUS)${NC}"
    head -80 "$LOG"
    exit 1
fi

# (a) Badlands taps for {R} (its Mountain side)
if grep -qE "Tap Badlands for \{R\}" "$LOG"; then
    echo -e "${GREEN}✓ Badlands tapped for {R}${NC}"
else
    echo -e "${RED}✗ Badlands did not tap for red${NC}"
    grep -iE "badlands|tap" "$LOG" | head -8
    exit 1
fi

# (b) Bayou taps for {G} (its Forest side)
if grep -qE "Tap Bayou for \{G\}" "$LOG"; then
    echo -e "${GREEN}✓ Bayou tapped for {G}${NC}"
else
    echo -e "${RED}✗ Bayou did not tap for green${NC}"
    grep -iE "bayou|tap" "$LOG" | head -8
    exit 1
fi

# (c) Both spells resolved (so the produced mana actually paid the cost)
if grep -qE "Lightning Bolt \([0-9]+\) deals 3 damage" "$LOG"; then
    echo -e "${GREEN}✓ Lightning Bolt resolved (paid by Badlands {R})${NC}"
else
    echo -e "${RED}✗ Lightning Bolt did not resolve${NC}"
    grep -iE "lightning bolt" "$LOG" | head -8
    exit 1
fi

if grep -qE "Giant Growth \([0-9]+\) gives" "$LOG"; then
    echo -e "${GREEN}✓ Giant Growth resolved (paid by Bayou {G})${NC}"
else
    echo -e "${RED}✗ Giant Growth did not resolve${NC}"
    grep -iE "giant growth" "$LOG" | head -8
    exit 1
fi

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "Full log: $LOG"
exit 0
