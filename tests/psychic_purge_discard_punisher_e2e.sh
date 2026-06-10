#!/usr/bin/env bash
# E2E test: Psychic Purge's opponent-forced-discard punisher (mtg-648 / mtg-534).
#
# Psychic Purge (cardsfolder/p/psychic_purge.txt):
#   A:SP$ DealDamage | ValidTgts$ Any | NumDmg$ 1
#   T:Mode$ Discarded | ValidCard$ Card.Self
#     | ValidCause$ SpellAbility.OppCtrl | Execute$ TrigLoseLife
#   SVar:TrigLoseLife:DB$ LoseLife | Defined$ TriggeredCauseController | LifeAmount$ 5
# "When a spell or ability an opponent controls causes you to discard Psychic
#  Purge, that player loses 5 life."
#
# Before the fix there was no TriggerEvent::Discarded, so the punisher clause
# was silently dropped at load. After the fix the loader emits a Discarded
# self-trigger (fired on the discarded card's LKI in the graveyard, CR
# 603.6/603.10), gated by requires_opponent_cause, whose LoseLife targets the
# discard's CAUSE controller. The cause is threaded EXPLICITLY through the
# discard call path as a `cause: Option<PlayerId>` parameter (NOT mutable
# GameState state) — None for a self-discard, the resolving spell/ability's
# controller for a forced one — so there is nothing to reconstruct on a network
# shadow or WASM rewind.
#
# Two scenarios (seed 42):
#   PUNISH  (psychic_purge_opponent_discard_punish_e2e.pzl):
#     Player 1 (p0) casts Mind Rot at Player 2 (p1), who is holding Psychic
#     Purge. When p1 discards Psychic Purge, the CASTER (Player 1) loses 5 life
#     (20 -> 15) — an OPPONENT's spell caused the discard.
#   NO-PUNISH (psychic_purge_self_discard_no_punish_e2e.pzl):
#     Player 1 (p0) holds 8 cards incl. Psychic Purge and discards it to hand
#     size in their OWN cleanup step (no spell/ability cause). No life is lost
#     by anyone — the opponent-only gate does not fire on a self-discard.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Psychic Purge discard-punisher E2E ==="
echo

cd "$WORKSPACE_ROOT"

# ---------------------------------------------------------------------------
# Scenario 1: opponent's Mind Rot forces the discard -> caster loses 5 life.
# ---------------------------------------------------------------------------
# p0 casts Mind Rot targeting p1 (`p2` = "the second player" in the rich-input
# target grammar), then wildcard-passes. p1 is heuristic and chooses cards.
PUNISH_INPUTS="cast Mind Rot;p2"
for _ in $(seq 1 8); do PUNISH_INPUTS="${PUNISH_INPUTS};*"; done

PUNISH_LOG=/tmp/psychic_purge_opponent_discard_punish_e2e.txt
if run_mtg_with_timeout 60 tui \
    --start-state "$WORKSPACE_ROOT/test_puzzles/psychic_purge_opponent_discard_punish_e2e.pzl" \
    --p1=fixed --p2=heuristic \
    --p1-fixed-inputs="$PUNISH_INPUTS" \
    --json --seed 42 --verbosity 3 \
    > "$PUNISH_LOG" 2>&1; then
    echo -e "${GREEN}✓ Game completed (punish scenario)${NC}"
else
    echo -e "${RED}✗ Game failed (punish scenario)${NC}"
    head -80 "$PUNISH_LOG"
    exit 1
fi

# Psychic Purge must actually be discarded by the forced discard.
if ! grep -qE "Player 2 discards Psychic Purge" "$PUNISH_LOG"; then
    echo -e "${RED}✗ Psychic Purge was not discarded by Mind Rot${NC}"
    grep -E "discards|Mind Rot" "$PUNISH_LOG" || true
    exit 1
fi
echo -e "${GREEN}✓ Mind Rot forced Player 2 to discard Psychic Purge${NC}"

# The CAUSE controller (Mind Rot's caster, Player 1) must lose 5 life (20 -> 15).
if ! grep -qE "Player 1 loses 5 life \(life: 15\)" "$PUNISH_LOG"; then
    echo -e "${RED}✗ Psychic Purge punisher did NOT fire (caster should lose 5 life -> 15)${NC}"
    grep -E "loses .* life|life:" "$PUNISH_LOG" || echo "(no life-loss line — silent drop?)"
    exit 1
fi
echo -e "${GREEN}✓ Punisher fired: Mind Rot's caster lost 5 life (20 -> 15)${NC}"

# ---------------------------------------------------------------------------
# Scenario 2: own cleanup-step discard -> NO punisher (self-discard).
# ---------------------------------------------------------------------------
NOPUNISH_INPUTS=""
for _ in $(seq 1 20); do NOPUNISH_INPUTS="${NOPUNISH_INPUTS}*;"; done
NOPUNISH_INPUTS="${NOPUNISH_INPUTS%;}"

NOPUNISH_LOG=/tmp/psychic_purge_self_discard_no_punish_e2e.txt
if run_mtg_with_timeout 60 tui \
    --start-state "$WORKSPACE_ROOT/test_puzzles/psychic_purge_self_discard_no_punish_e2e.pzl" \
    --p1=fixed --p2=heuristic \
    --p1-fixed-inputs="$NOPUNISH_INPUTS" \
    --json --seed 42 --verbosity 3 \
    > "$NOPUNISH_LOG" 2>&1; then
    echo -e "${GREEN}✓ Game completed (no-punish scenario)${NC}"
else
    echo -e "${RED}✗ Game failed (no-punish scenario)${NC}"
    head -80 "$NOPUNISH_LOG"
    exit 1
fi

# Psychic Purge must be discarded in the cleanup step ...
if ! grep -qE "Player 1 discards Psychic Purge" "$NOPUNISH_LOG"; then
    echo -e "${RED}✗ Psychic Purge was not discarded in cleanup${NC}"
    grep -E "Cleanup|discards" "$NOPUNISH_LOG" || true
    exit 1
fi
echo -e "${GREEN}✓ Player 1 discarded Psychic Purge in their own cleanup step${NC}"

# ... and NOBODY may lose 5 life from it (self-discard has no opponent cause).
if grep -qE "loses 5 life" "$NOPUNISH_LOG"; then
    echo -e "${RED}✗ Punisher wrongly fired on a SELF-discard (someone lost 5 life)${NC}"
    grep -E "loses 5 life" "$NOPUNISH_LOG"
    exit 1
fi
echo -e "${GREEN}✓ No punisher on the self-discard (no 5-life loss)${NC}"

echo
echo -e "${GREEN}=== Psychic Purge discard-punisher E2E PASSED ===${NC}"
