#!/usr/bin/env bash
# E2E test: Animate Dead reanimates Triskelion with both counter effects
#
# Regression test for the "Animate Dead reanimation + counter placement" bug.
#
# Two interacting bugs fixed in this commit:
#
# 1. K:etbCounter (Triskelion) was parsed but never applied at runtime —
#    Triskelion entered the battlefield as a 1/1 with zero +1/+1 counters,
#    making its activated ability cost ("remove a +1/+1 counter") immediately
#    fail and the creature trivially die to a 1/2.
#    Fix: `apply_etb_counters` helper in `mtg-engine/src/game/actions/mod.rs`
#    runs at every battlefield ETB site (play_land, resolve_spell_finalize,
#    reanimate_aura_target).
#
# 2. The Animate Dead reanimation chain (`TrigReanimate` → `DBAnimate` →
#    `DBAttach`) was unimplemented in the effect converter, so Animate Dead
#    used to resolve, fail to attach (target was in graveyard), and head
#    straight to the graveyard itself — Triskelion never came back.
#    Fix: `reanimate_aura_target` helper inlines the move-from-graveyard +
#    attach for Auras whose chosen target is in a graveyard. The continuous
#    -1/-0 effect (`S:Mode$ Continuous | Affected$ Creature.EnchantedBy |
#    AddPower$ -1`) was already wired and fires automatically once attached.
#
# After both fixes, Triskelion reanimated by Animate Dead is a 3/4 (base 1/1,
# +3/+3 from three +1/+1 counters, -1/-0 from Animate Dead) and can still
# activate its ping ability normally.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Animate Dead Reanimates Triskelion (counters + -1/-0) E2E ==="
echo

cd "$WORKSPACE_ROOT"

PUZZLE="$WORKSPACE_ROOT/test_puzzles/animate_dead_triskelion.pzl"
LOG=/tmp/animate_dead_triskelion_e2e.txt

if run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE" \
    --p1=fixed --p2=zero \
    --p1-fixed-inputs="cast Animate Dead;pass;pass;pass;pass;pass;pass;pass;pass;pass" \
    --p2-fixed-inputs="" \
    --stop-on-choice=15 --json --seed 42 --verbosity 3 \
    > "$LOG" 2>&1; then
    echo -e "${GREEN}✓ Game completed${NC}"
else
    EXIT_STATUS=$?
    echo -e "${RED}✗ Game failed (exit $EXIT_STATUS)${NC}"
    head -120 "$LOG"
    exit 1
fi

# (a) Animate Dead resolves and reanimates Triskelion
if grep -qE "Animate Dead reanimates Triskelion from graveyard" "$LOG"; then
    echo -e "${GREEN}✓ Animate Dead reanimated Triskelion${NC}"
else
    echo -e "${RED}✗ Reanimation never happened${NC}"
    grep -iE "animate|reanim|enchant" "$LOG" | head -10
    exit 1
fi

# (b) Triskelion enters with three +1/+1 counters (etbCounter)
if grep -qE "Triskelion enters the battlefield with 3 \+1/\+1 counters" "$LOG"; then
    echo -e "${GREEN}✓ Triskelion enters with three +1/+1 counters${NC}"
else
    echo -e "${RED}✗ etbCounter:P1P1:3 never fired${NC}"
    grep -iE "triskelion|counter" "$LOG" | head -10
    exit 1
fi

# (c) Animate Dead attaches to the reanimated Triskelion
if grep -qE "Animate Dead enchants Triskelion" "$LOG"; then
    echo -e "${GREEN}✓ Animate Dead attached to reanimated Triskelion${NC}"
else
    echo -e "${RED}✗ Animate Dead never attached after reanimation${NC}"
    exit 1
fi

# (d) Triskelion shows as 3/4 on the battlefield
#     (base 1/1 + 3*P1P1 = 4/4, then -1/-0 from Animate Dead = 3/4)
if grep -qE "Triskelion \([0-9]+\) - 3/4" "$LOG"; then
    echo -e "${GREEN}✓ Reanimated Triskelion shows 3/4 (counters + -1/-0 both applied)${NC}"
else
    echo -e "${RED}✗ Triskelion P/T wrong; expected 3/4${NC}"
    grep -E "Triskelion \([0-9]+\) - " "$LOG" | head -5
    exit 1
fi

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo
echo "Full log: $LOG"
exit 0
