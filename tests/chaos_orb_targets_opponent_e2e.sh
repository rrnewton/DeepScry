#!/usr/bin/env bash
# E2E test: Chaos Orb targeted-destroy (mtg-389).
#
# Owner's interpretation: Chaos Orb is a paper DEXTERITY card. We approximate
# it digitally as a single targeted destroy:
#   "{1}, {T}: Destroy target nontoken permanent, then destroy Chaos Orb."
# (We DIVERGE from Forge-Java, which models the physical flip with
# FlipOntoBattlefield + DestroyAll Remembered. See cardsfolder/c/chaos_orb.txt.)
#
# This test drives the interactive (stdin) controller so we can pick a SPECIFIC
# target by menu index, exercising three aspects per the targeted_compatibility
# skill:
#   1. destroy an opponent CREATURE (Grizzly Bears) + Chaos Orb self-destroys
#   2. destroy an opponent NONCREATURE permanent (Mountain) + self-destroy
#   3. destroy the controller's OWN noncreature permanent (Plains) + self-destroy
# In every case the Defined$ Self subability destroys Chaos Orb itself.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Chaos Orb: Targeted Destroy E2E Test ==="
echo

cd "$WORKSPACE_ROOT"

PUZZLE="$WORKSPACE_ROOT/test_puzzles/chaos_orb_destroys_target.pzl"
# Menu (P1's turn, after activating Chaos Orb):
#   [0] Chaos Orb (yours)  [1] Plains  [2] Plains  [3] Mountain (theirs)  [4] Grizzly Bears (theirs)

run_case() {
    local label="$1" target_idx="$2" expect_target="$3" log="$4"
    # stdin: [1] activate Chaos Orb, then target index, then passes.
    printf '1\n%s\n0\n0\n0\n0\n' "$target_idx" \
        | "$MTG_BIN" tui --start-state "$PUZZLE" --p1 tui --p2 zero \
            --seed 42 --verbosity 3 > "$log" 2>&1 || true

    if grep -qE "targeting $expect_target \([0-9]+\)" "$log"; then
        echo -e "${GREEN}✓ [$label] Chaos Orb targeted $expect_target${NC}"
    else
        echo -e "${RED}✗ [$label] Chaos Orb did NOT target $expect_target${NC}"
        grep -E "targeting" "$log" || echo "(no targeting lines)"
        exit 1
    fi

    if grep -qE "$expect_target \([0-9]+\) goes to graveyard" "$log"; then
        echo -e "${GREEN}✓ [$label] $expect_target destroyed (graveyard)${NC}"
    else
        echo -e "${RED}✗ [$label] $expect_target was NOT destroyed${NC}"
        grep -E "goes to graveyard" "$log" || echo "(no graveyard lines)"
        exit 1
    fi

    if grep -qE "Chaos Orb \([0-9]+\) goes to graveyard" "$log"; then
        echo -e "${GREEN}✓ [$label] Chaos Orb self-destroyed${NC}"
    else
        echo -e "${RED}✗ [$label] Chaos Orb did NOT self-destroy${NC}"
        exit 1
    fi
}

run_case "opponent creature"    4 "Grizzly Bears" /tmp/chaos_orb_creature.txt
run_case "opponent noncreature" 3 "Mountain"      /tmp/chaos_orb_noncreature.txt
run_case "own permanent"        1 "Plains"         /tmp/chaos_orb_own.txt

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
exit 0
