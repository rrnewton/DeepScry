#!/usr/bin/env bash
# E2E test: Psionic Blast cross-player self-damage (mtg-533, REOPENED).
#
# Psionic Blast: "deals 4 damage to any target AND 2 damage to you."
# The "2 to you" rider is `DB$ DealDamage | Defined$ You`, which the converter
# encodes as TargetRef::Player(PlayerId(0)). Because PLACEHOLDER_ID == 0 and a
# real player 0 (P1) also exists, the spell-resolution path (resolve_effect_
# target) was MISSING an arm to resolve that placeholder to the caster — so the
# unresolved PlayerId(0) fell through to deal_damage() and hit the literal
# player 0 (P1). On a cross-player cast (P2 casts at P1) the 2 self-damage
# wrongly landed on P1 instead of the caster P2.
#
# This was a FALSE-CLOSED bug: the original test only exercised the self-cast
# direction (caster == P1 == player 0), where the literal-0 placeholder
# coincidentally landed on the right player.
#
# Fix: resolve_effect_target (and the display logger) now resolve a placeholder
# Player target on DealDamage to card_owner (the caster), mirroring the existing
# PreventDamage `Defined$ You` arm and the trigger-path resolve_effect_placeholder.
#
# This test asserts recipient IDENTITY + post-damage life, per the hardened
# targeted_compatibility skill: P2 casts Psionic Blast at P1; P1 must lose 4 and
# P2 (the caster) must lose 2.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Psionic Blast: Cross-Player Self-Damage E2E Test ==="
echo

cd "$WORKSPACE_ROOT"

# --- Case 1: cross-player at a PLAYER (the core regression) ---
# P2 (puzzle p1, active player) casts Psionic Blast at P1.
# Menu: [0] Player 1 (them)  [1] Player 2 (you)  -> pick 0 to hit P1.
PUZZLE1="$WORKSPACE_ROOT/test_puzzles/psionic_blast_crossplayer_selfdamage.pzl"
LOG1=/tmp/psionic_blast_crossplayer_player.txt
printf '1\n0\n0\n0\n0\n0\n' \
    | "$MTG_BIN" tui --start-state "$PUZZLE1" --p1 zero --p2 tui \
        --seed 42 --verbosity 3 > "$LOG1" 2>&1 || true

# The 4 damage must hit Player 1 (target), leaving P1 at 16.
if grep -qE "Psionic Blast \([0-9]+\) deals 4 damage to Player 1 \(life: 16\)" "$LOG1"; then
    echo -e "${GREEN}✓ [player] 4 damage hit Player 1 (target), P1 life 16${NC}"
else
    echo -e "${RED}✗ [player] 4 damage did not hit Player 1 / wrong life${NC}"
    grep -E "deals .* damage to Player" "$LOG1" || echo "(no damage lines)"
    exit 1
fi

# The 2 self-damage must hit Player 2 (the CASTER), leaving P2 at 18 — NOT P1.
if grep -qE "Psionic Blast \([0-9]+\) deals 2 damage to Player 2 \(life: 18\)" "$LOG1"; then
    echo -e "${GREEN}✓ [player] 2 self-damage hit Player 2 (caster), P2 life 18${NC}"
else
    echo -e "${RED}✗ [player] 2 self-damage went to the WRONG player (mtg-533 regression)${NC}"
    grep -E "deals .* damage to Player" "$LOG1" || echo "(no damage lines)"
    exit 1
fi

# Guard: the 2 must NOT have hit Player 1 (the original bug signature).
if grep -qE "Psionic Blast \([0-9]+\) deals 2 damage to Player 1" "$LOG1"; then
    echo -e "${RED}✗ [player] 2 self-damage hit Player 1 — cross-player bug regressed${NC}"
    exit 1
fi

# --- Case 2: cross-player at a CREATURE (self-damage still to caster) ---
# P2 casts Psionic Blast at a P1 creature (Sengir Vampire). The 4 kills the
# creature; the 2 self-damage still hits the caster P2 (life 18).
PUZZLE2="$WORKSPACE_ROOT/test_puzzles/psionic_blast_creature_target.pzl"
LOG2=/tmp/psionic_blast_crossplayer_creature.txt
printf '1\n0\n0\n0\n0\n0\n' \
    | "$MTG_BIN" tui --start-state "$PUZZLE2" --p1 zero --p2 tui \
        --seed 42 --verbosity 3 > "$LOG2" 2>&1 || true

if grep -qE "Sengir Vampire \([0-9]+\) takes 4 damage" "$LOG2" \
    && grep -qE "Sengir Vampire \([0-9]+\) goes to graveyard" "$LOG2"; then
    echo -e "${GREEN}✓ [creature] 4 damage killed the targeted creature${NC}"
else
    echo -e "${RED}✗ [creature] creature did not take 4 / die${NC}"
    grep -E "takes|graveyard" "$LOG2" || echo "(none)"
    exit 1
fi

if grep -qE "Psionic Blast \([0-9]+\) deals 2 damage to Player 2 \(life: 18\)" "$LOG2"; then
    echo -e "${GREEN}✓ [creature] 2 self-damage hit caster Player 2 (life 18)${NC}"
else
    echo -e "${RED}✗ [creature] 2 self-damage did not hit the caster${NC}"
    grep -E "deals .* damage to Player" "$LOG2" || echo "(no damage lines)"
    exit 1
fi

# --- Case 3: non-regression, self-cast direction (P1 casts at P2) ---
PUZZLE3="$WORKSPACE_ROOT/test_puzzles/psionic_blast_p1_casts.pzl"
LOG3=/tmp/psionic_blast_p1_casts.txt
printf '1\n0\n0\n0\n0\n0\n' \
    | "$MTG_BIN" tui --start-state "$PUZZLE3" --p1 tui --p2 zero \
        --seed 42 --verbosity 3 > "$LOG3" 2>&1 || true

if grep -qE "Psionic Blast \([0-9]+\) deals 4 damage to Player 2 \(life: 16\)" "$LOG3" \
    && grep -qE "Psionic Blast \([0-9]+\) deals 2 damage to Player 1 \(life: 18\)" "$LOG3"; then
    echo -e "${GREEN}✓ [self-cast] P1 casts at P2: 4 to P2 (16), 2 self to P1 (18)${NC}"
else
    echo -e "${RED}✗ [self-cast] non-regression failed${NC}"
    grep -E "deals .* damage to Player" "$LOG3" || echo "(no damage lines)"
    exit 1
fi

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
exit 0
