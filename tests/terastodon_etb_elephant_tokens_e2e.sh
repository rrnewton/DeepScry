#!/usr/bin/env bash
# E2E test: Terastodon ({6}{G}{G} Creature Elephant 9/9) ETB trigger
# destroys a noncreature permanent and creates a 3/3 green Elephant token
# for the destroyed permanent's controller (via RepeatEach + ChangeZoneTable).
#
# Regression test for 2010 World Championship compat tracker mtg-914 B2.
#
# Two bugs were fixed:
# 1. extract_effects_from_svar called params_to_effect (no SVars) for
#    RepeatEach, so the RepeatSubAbility$ SVar (DBToken) could not be
#    resolved — RepeatEach was silently Unimplemented.
# 2. Even after RepeatEach was parsed correctly, in the trigger execution
#    path (check_triggers) DestroyPermanent consumed its target into
#    trigger_destroy_targets, but RepeatEach had empty targets because it
#    expected remaining chosen_targets (spell path) not pre-consumed destroy
#    targets. Fix: accumulate trigger_destroy_targets during DestroyPermanent
#    resolution and use them to fill RepeatEach targets.
#
# Card script:
#   T:Mode$ ChangesZone | ... | Execute$ TrigDestroy
#   SVar:TrigDestroy:DB$ Destroy | TargetMin$ 0 | TargetMax$ 3
#                                 | ValidTgts$ Permanent.nonCreature
#                                 | SubAbility$ MakeTokens
#   SVar:MakeTokens:DB$ RepeatEach | RepeatSubAbility$ DBToken
#                                   | DefinedCards$ Targeted | ChangeZoneTable$ True
#   SVar:DBToken:DB$ Token | TokenOwner$ RememberedController
#                           | TokenScript$ g_3_3_elephant
#
# Scenario (test_puzzles/terastodon_etb_elephant_tokens.pzl):
# - P1 casts Terastodon (8 Forests as mana).
# - ETB trigger fires: P1 destroys one of their own Forests (noncreature permanent).
# - RepeatEach runs: for each destroyed permanent (1), its controller (P1)
#   creates a 3/3 green Elephant token.
# - Verifies: (a) token was created (no "WARNING: Effect 'RepeatEach'..." log),
#   (b) Elephant Token appears on P1's battlefield.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Terastodon ETB Elephant Tokens E2E Test ==="
echo

cd "$WORKSPACE_ROOT"

PUZZLE="$WORKSPACE_ROOT/test_puzzles/terastodon_etb_elephant_tokens.pzl"
LOG=/tmp/terastodon_etb_elephant_tokens_e2e.txt

if run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE" \
    --p1=fixed --p2=zero \
    --p1-fixed-inputs="cast Terastodon;*;*;*;*;*;*;*;*;*;*;*;*;*" \
    --seed 42 --verbosity 3 --no-color-logs \
    > "$LOG" 2>&1; then
    echo -e "${GREEN}✓ Game completed${NC}"
else
    EXIT_STATUS=$?
    echo -e "${RED}✗ Game failed (exit $EXIT_STATUS)${NC}"
    head -80 "$LOG"
    exit 1
fi

# (a) No "Unimplemented effect 'RepeatEach'" warning — the RepeatEach must
#     have been parsed and executed correctly (not as Unimplemented no-op).
if grep -qE "WARNING.*RepeatEach|Unimplemented.*RepeatEach" "$LOG"; then
    echo -e "${RED}✗ RepeatEach was still Unimplemented — token-payoff not wired${NC}"
    grep -iE "repeatEach|unimplemented" "$LOG" | head -5
    exit 1
else
    echo -e "${GREEN}✓ No Unimplemented RepeatEach warning${NC}"
fi

# (b) Terastodon ETB fired and a permanent was destroyed (went to graveyard).
if grep -qE "goes to graveyard" "$LOG"; then
    echo -e "${GREEN}✓ A permanent was destroyed by Terastodon ETB${NC}"
else
    echo -e "${RED}✗ No permanent was destroyed by Terastodon ETB${NC}"
    grep -iE "terastodon|destroy|graveyard" "$LOG" | head -8
    exit 1
fi

# (c) An Elephant Token was created — this is the RepeatEach payoff.
if grep -qE "Created Elephant Token" "$LOG"; then
    echo -e "${GREEN}✓ Elephant Token created by RepeatEach (token-per-destroyed-permanent)${NC}"
else
    echo -e "${RED}✗ No Elephant Token created — RepeatEach payoff missing${NC}"
    grep -iE "elephant|token|repeat" "$LOG" | head -8
    exit 1
fi

# (d) The Elephant Token appears on Player 1's battlefield in subsequent display.
if grep -qE "Elephant Token.*- 3/3" "$LOG"; then
    echo -e "${GREEN}✓ Elephant Token (3/3) visible on battlefield${NC}"
else
    echo -e "${RED}✗ Elephant Token not visible on battlefield${NC}"
    grep -iE "elephant|3/3" "$LOG" | head -8
    exit 1
fi

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "Full log: $LOG"
exit 0
