#!/usr/bin/env bash
# E2E test: Falling Star targeted damage + tap-on-survive (mtg-503).
#
# Owner's interpretation: Falling Star is a paper DEXTERITY card. We approximate
# it digitally as a single targeted effect:
#   "{2}{R} Sorcery: Falling Star deals 3 damage to target creature.
#    If that creature survives, tap it."
# (We DIVERGE from Forge-Java's FlipOntoBattlefield + DamageAll + TapAll
# Remembered. See cardsfolder/f/falling_star.txt.)
#
# Implemented via SP$ DealDamage | ValidTgts$ Creature + a chained
# DB$ Tap | Defined$ Targeted. The Tap reuses the parent's target and is gated
# on survival in resolve_effect_target (actions/mod.rs) and in the display
# logger (game_loop/priority.rs): a creature that dies to the 3 damage is NOT
# tapped (and emits no "taps" log line).
#
# Two aspects (skill targeted_compatibility):
#   1. target a 2/2 (Grizzly Bears) -> dies, NO tap.
#   2. target a 4/4 (Sengir Vampire) -> survives 3 damage, gets TAPPED.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Falling Star: Damage + Tap-on-Survive E2E Test ==="
echo

cd "$WORKSPACE_ROOT"

PUZZLE="$WORKSPACE_ROOT/test_puzzles/falling_star_damage_tap.pzl"
# Menu (P1's turn, casting Falling Star): [0] Grizzly Bears  [1] Sengir Vampire

# --- Case 1: 2/2 dies, no tap ---
LOG1=/tmp/falling_star_dies.txt
printf '1\n0\n0\n0\n0\n0\n' \
    | "$MTG_BIN" tui --start-state "$PUZZLE" --p1 tui --p2 zero \
        --seed 42 --verbosity 3 > "$LOG1" 2>&1 || true

if grep -qE "Grizzly Bears \([0-9]+\) takes 3 damage" "$LOG1"; then
    echo -e "${GREEN}✓ [dies] Grizzly Bears took 3 damage${NC}"
else
    echo -e "${RED}✗ [dies] Grizzly Bears did not take 3 damage${NC}"
    grep -E "takes|deals" "$LOG1" || echo "(none)"
    exit 1
fi

if grep -qE "Grizzly Bears \([0-9]+\) goes to graveyard" "$LOG1"; then
    echo -e "${GREEN}✓ [dies] Grizzly Bears died${NC}"
else
    echo -e "${RED}✗ [dies] Grizzly Bears did not die${NC}"
    exit 1
fi

# A dead creature must NOT be tapped — no "Falling Star ... taps" line at all.
if grep -qE "Falling Star \([0-9]+\) taps " "$LOG1"; then
    echo -e "${RED}✗ [dies] Falling Star logged a tap of a dead creature${NC}"
    grep -E " taps " "$LOG1"
    exit 1
else
    echo -e "${GREEN}✓ [dies] No tap of the dead creature (correct: it died)${NC}"
fi

# --- Case 2: 4/4 survives, gets tapped ---
LOG2=/tmp/falling_star_survives.txt
printf '1\n1\n0\n0\n0\n0\n' \
    | "$MTG_BIN" tui --start-state "$PUZZLE" --p1 tui --p2 zero \
        --seed 42 --verbosity 3 > "$LOG2" 2>&1 || true

if grep -qE "Sengir Vampire \([0-9]+\) takes 3 damage" "$LOG2"; then
    echo -e "${GREEN}✓ [survives] Sengir Vampire took 3 damage${NC}"
else
    echo -e "${RED}✗ [survives] Sengir Vampire did not take 3 damage${NC}"
    grep -E "takes|deals" "$LOG2" || echo "(none)"
    exit 1
fi

if grep -qE "Sengir Vampire \([0-9]+\) goes to graveyard" "$LOG2"; then
    echo -e "${RED}✗ [survives] Sengir Vampire wrongly died to 3 damage (4 toughness)${NC}"
    exit 1
fi

if grep -qE "Falling Star \([0-9]+\) taps Sengir Vampire \([0-9]+\)" "$LOG2"; then
    echo -e "${GREEN}✓ [survives] Falling Star tapped the surviving Sengir Vampire${NC}"
else
    echo -e "${RED}✗ [survives] Surviving creature was NOT tapped${NC}"
    grep -E " taps " "$LOG2" || echo "(no tap lines)"
    exit 1
fi

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
exit 0
