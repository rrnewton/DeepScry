#!/usr/bin/env bash
# E2E test: Icy Manipulator correctly taps an opponent's permanent.
#
# Card: Icy Manipulator (cardsfolder/i/icy_manipulator.txt) — mtg-511
# Deck: 05 Mono Black Rogerbrand (mtg-560)
#
# Script:
#   ManaCost:4
#   Types:Artifact
#   A:AB$ Tap | Cost$ 1 T | ValidTgts$ Artifact,Creature,Land
#     | SpellDescription$ Tap target artifact, creature, or land.
#
# Fix (compat-monoblack-v2): The heuristic controller previously targeted
# its OWN source (Icy Manipulator itself) when activating the AB$ Tap
# ability, making it useless as a control tool. After the fix:
#   1. has_tap_effect → prefer opponent permanents in choose_targets
#   2. should_cast_spell → cast utility artifacts with non-mana activated
#      abilities when the opponent has permanents
#
# Bugs: mtg-zssaf (auto-target-source)
#
# Test scenario:
# P1 (heuristic) has Icy Manipulator + 6 Swamps
# P2 (heuristic) has Sengir Vampire + 6 Swamps
# P1 should cast Icy Manipulator and then tap Sengir Vampire (an opponent
# permanent) rather than tapping itself.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Icy Manipulator Targets Opponent E2E ==="
echo

cd "$WORKSPACE_ROOT"

LOG=/tmp/icy_manipulator_taps_opponent_e2e.txt

# P1 has Icy Manipulator + 6 Swamps; P2 has Sengir Vampire + 6 Swamps.
# P1 (heuristic) should cast Icy Manipulator and then tap Sengir Vampire.
if run_mtg_with_timeout 30 tui \
    "$WORKSPACE_ROOT/decks/old_school/05_mono_black_rogerbrand.dck" \
    "$WORKSPACE_ROOT/decks/old_school/05_mono_black_rogerbrand.dck" \
    --p1-draw "Icy Manipulator;Swamp;Swamp;Swamp;Swamp;Swamp" \
    --p2-draw "Sengir Vampire;Swamp;Swamp;Swamp;Swamp;Swamp" \
    --p1=heuristic --p2=heuristic \
    --seed 42 --verbosity 2 > "$LOG" 2>&1; then
    echo -e "${GREEN}✓ Game completed${NC}"
else
    echo -e "${RED}✗ Game failed${NC}"
    head -80 "$LOG"
    exit 1
fi

# Required: Icy Manipulator was cast and resolved.
if grep -qE "Icy Manipulator \([0-9]+\) resolves" "$LOG"; then
    echo -e "${GREEN}✓ Icy Manipulator was cast and resolved${NC}"
else
    echo -e "${RED}✗ Icy Manipulator was not cast (or did not resolve)${NC}"
    grep -E "Icy" "$LOG" | head -5 || echo "(no Icy mentions in log)"
    exit 1
fi

# Required: Icy Manipulator activated its ability.
if grep -qE "Icy Manipulator activates ability: Tap target artifact, creature, or land" "$LOG"; then
    echo -e "${GREEN}✓ Icy Manipulator tap ability activated${NC}"
else
    echo -e "${RED}✗ Icy Manipulator tap ability was not activated${NC}"
    grep -E "activates" "$LOG" | head -5 || echo "(no activates in log)"
    exit 1
fi

# Required: NOT targeting itself (the original bug mtg-zssaf).
if grep -qF "targeting Icy Manipulator" "$LOG"; then
    echo -e "${RED}✗ BUG (mtg-zssaf): Icy Manipulator targeted ITSELF — self-tap bug still present${NC}"
    grep -F "targeting" "$LOG" | head -5
    exit 1
fi
echo -e "${GREEN}✓ Icy Manipulator did not target itself${NC}"

# Required: targets an opponent permanent.
if grep -qE "targeting (Sengir Vampire|Juzam|Juzám|Black Knight|Swamp)" "$LOG"; then
    echo -e "${GREEN}✓ Icy Manipulator targeted an opponent permanent${NC}"
else
    echo -e "${RED}✗ Could not verify Icy Manipulator targeted an opponent permanent${NC}"
    grep -E "targeting" "$LOG" | head -10 || echo "(no targeting lines in log)"
    exit 1
fi

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "Full log: $LOG"
exit 0
