#!/usr/bin/env bash
# E2E test: Animate Dead's leave-the-battlefield SACRIFICE drawback.
#
# Regression test for the "Animate Dead DBDelay sacrifice trigger" gap
# (mtg-400 / mtg-394 item 9). Animate Dead's Oracle text is:
#
#   "Enchant creature card in a graveyard
#    When Animate Dead enters, ... return enchanted creature card to the
#    battlefield under your control ...
#    When Animate Dead leaves the battlefield, that creature's controller
#    sacrifices it."
#
# The final clause is the classic drawback: kill/bounce/Disenchant the Aura
# and the reanimated creature dies with it. This was previously UNimplemented
# — destroying Animate Dead left the creature behind permanently.
#
# Fix (this commit):
#   * New DelayedEffect::SacrificeOther{card} variant
#     (mtg-engine/src/core/delayed_trigger.rs) — sacrifice a card OTHER than
#     the one the delayed trigger is tracking.
#   * reanimate_aura_target (mtg-engine/src/game/actions/mod.rs) registers a
#     ZoneChange(Battlefield -> any) delayed trigger that WATCHES the Aura and
#     SACRIFICES the reanimated creature when the Aura leaves.
#   * fire_delayed_trigger (mtg-engine/src/game/state.rs) executes
#     SacrificeOther, no-op'ing if the creature already left the battlefield
#     (so a creature that DIES first — which removes the Aura via SBA and fires
#     this trigger — is never double-sacrificed).
#
# Scenario (test_puzzles/animate_dead_sacrifice_on_leave.pzl):
#   - P0 hand: Animate Dead, Disenchant. P0 board: City of Brass x4.
#   - P0 graveyard: Triskelion.
#   - P0 casts Animate Dead -> reanimates Triskelion (3/4 under P0 control).
#   - P0 casts Disenchant on its OWN Animate Dead -> the Aura is destroyed,
#     leaves the battlefield, and the delayed trigger sacrifices Triskelion.
#
# This generalizes to every removal of a reanimation Aura that carries the
# leave-the-battlefield-sacrifice clause (Dance of the Dead, Necromancy, ...).
#
# Relationship to Java Forge: Java implements this via the card script's
# `DB$ DelayedTrigger | Mode$ ChangesZone ... | Execute$ DBSacrifice` with
# `RememberObjects$ RememberedLKI`. We model the same semantics inline with a
# typed DelayedEffect::SacrificeOther rather than a generic scripted delayed
# trigger; the observable behavior matches.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Animate Dead Sacrifice-On-Leave E2E Test ==="
echo

cd "$WORKSPACE_ROOT"

PUZZLE="$WORKSPACE_ROOT/test_puzzles/animate_dead_sacrifice_on_leave.pzl"
LOG=/tmp/animate_dead_sacrifice_on_leave_e2e.txt

# The trailing fixed inputs run out of meaningful actions after the two casts;
# that is expected (TurnLimit draw). We capture the log and grep for the
# reanimation + sacrifice lines.
run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE" \
    --p1=fixed --p2=zero \
    --p1-fixed-inputs="cast Animate Dead;cast Disenchant;Animate Dead;*" \
    --p2-fixed-inputs="" \
    --seed 3 --verbosity 3 \
    > "$LOG" 2>&1 || true

# (a) Animate Dead reanimates Triskelion.
if grep -qE "Animate Dead reanimates Triskelion from graveyard" "$LOG"; then
    echo -e "${GREEN}✓ Animate Dead reanimated Triskelion${NC}"
else
    echo -e "${RED}✗ Reanimation never happened${NC}"
    grep -iE "animate|reanim|enchant" "$LOG" | head -10
    exit 1
fi

# (b) Disenchant destroys the Aura (Animate Dead leaves the battlefield).
if grep -qE "Disenchant \([0-9]+\) destroys Animate Dead" "$LOG"; then
    echo -e "${GREEN}✓ Disenchant destroyed Animate Dead${NC}"
else
    echo -e "${RED}✗ Animate Dead was not destroyed${NC}"
    grep -iE "disenchant|destroy" "$LOG" | head -8
    exit 1
fi

# (c) The leave-the-battlefield delayed trigger SACRIFICES the reanimated
#     Triskelion — the whole point of this fix.
if grep -qE "Triskelion is sacrificed" "$LOG"; then
    echo -e "${GREEN}✓ Reanimated Triskelion sacrificed when Animate Dead left${NC}"
else
    echo -e "${RED}✗ Triskelion was NOT sacrificed (drawback missing)${NC}"
    grep -iE "triskelion|sacrific" "$LOG" | head -10
    exit 1
fi

# (d) Triskelion ends up in the graveyard, NOT on the battlefield.
#     After the sacrifice it must show going to the graveyard.
if grep -qE "Triskelion \([0-9]+\) goes to graveyard" "$LOG"; then
    echo -e "${GREEN}✓ Triskelion moved to the graveyard${NC}"
else
    echo -e "${RED}✗ Triskelion did not move to the graveyard${NC}"
    grep -iE "triskelion" "$LOG" | head -10
    exit 1
fi

# (e) Triskelion must NOT remain on the battlefield after the sacrifice.
#     Scan the final board snapshot (the last "Battlefield:" block) for it.
if grep -qE "Triskelion \([0-9]+\) - [0-9]+/[0-9]+ \[on battlefield\]" "$LOG"; then
    # If a board-status line ever shows Triskelion as a live permanent AFTER
    # the sacrifice, that's a leftover. The sacrifice line above already
    # proves removal, but guard against a double-state bug.
    LAST_SAC=$(grep -nE "Triskelion is sacrificed" "$LOG" | tail -1 | cut -d: -f1)
    LAST_LIVE=$(grep -nE "Triskelion \([0-9]+\) - [0-9]+/[0-9]+ \[on battlefield\]" "$LOG" | tail -1 | cut -d: -f1)
    if [ -n "$LAST_LIVE" ] && [ -n "$LAST_SAC" ] && [ "$LAST_LIVE" -gt "$LAST_SAC" ]; then
        echo -e "${RED}✗ Triskelion still shown on the battlefield AFTER being sacrificed${NC}"
        exit 1
    fi
fi
echo -e "${GREEN}✓ Triskelion is not a live permanent after the sacrifice${NC}"

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "Full log: $LOG"
exit 0
