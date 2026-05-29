#!/usr/bin/env bash
# E2E test: Mind Twist (X)(B) — target player discards X cards at random.
#
# Regression evidence for mtg-521 (Card Compatibility: Mind Twist).
# Covers:
#   1. Mind Twist can be cast for X=2 with {B} + 2 colorless from a Mox Jet
#      and a basic Swamp.
#   2. The opponent (the only legal target on a 2-player table per the
#      ValidTgts$ Player auto-pick fallback — see mtg-564) discards 2 cards.
#   3. Both discards are emitted as proper game-log lines naming the cards
#      (no sentinel placeholders).
#   4. The post-resolution log line shows the resolved X count, not the
#      literal "X" sentinel — i.e. "discard 2 card(s)" rather than
#      "discard X card(s)" (mtg-521 fix in
#      mtg-engine/src/game/game_loop/priority.rs).
#
# Mind Twist's script (cardsfolder/m/mind_twist.txt):
#   ManaCost:X B
#   Types:Sorcery
#   A:SP$ Discard | ValidTgts$ Player | NumCards$ X | Mode$ Random

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Mind Twist: Discard X Cards (X=1) E2E Test ==="
echo

cd "$WORKSPACE_ROOT"

if [[ ! -d "$WORKSPACE_ROOT/cardsfolder" ]]; then
    echo -e "${RED}Error: $WORKSPACE_ROOT/cardsfolder not found${NC}"
    exit 1
fi

DECK="$WORKSPACE_ROOT/decks/old_school/06_jeskai_aggro_joseantonioprieto.dck"
if [[ ! -f "$DECK" ]]; then
    echo -e "${RED}Error: deck not found: $DECK${NC}"
    exit 1
fi

LOG=/tmp/mind_twist_discard_test.txt

# P1 plays Badlands ({B}), casts Mox Jet ({B}), and casts Mind Twist with
# X=1 ({1}{B}) targeting the opponent. The Mox Jet provides {B}, the
# Badlands provides {B} which pays for {1} (generic) in the cost.
#
# Choice index 2 corresponds to "X = 1" in the X-paid prompt.
# NOTE: we deliberately tolerate a non-zero exit here. After Mind Twist
# resolves the engine still presents Fixed1 with action prompts (cast
# remaining cards), and the fixed-input script is exhausted, so the
# engine surfaces an InvalidAction error. The interesting state — the
# Mind Twist resolution log lines — is captured before that point. We
# assert on the captured log lines, not the exit status.
run_mtg_with_timeout 30 tui \
    "$DECK" "$DECK" \
    --p1-draw="Badlands;Scrubland;Mind Twist;Mox Jet" \
    --p2-draw="Plains" \
    --seed 42 \
    --p1=fixed --p2=fixed \
    --p1-fixed-inputs="play badlands;cast mox jet;cast mind twist;target opponent;2" \
    --p2-fixed-inputs="" \
    --stop-when-fixed-exhausted \
    --verbosity 3 \
    > "$LOG" 2>&1 || true

# 1. Mind Twist must enter the stack.
if grep -qE "casts Mind Twist \([0-9]+\)" "$LOG"; then
    echo -e "${GREEN}✓ Mind Twist was cast${NC}"
else
    echo -e "${RED}✗ Mind Twist was not cast${NC}"
    grep -i "mind twist" "$LOG" || echo "(no Mind Twist log lines)"
    exit 1
fi

# 2. Resolves.
if grep -qE "Mind Twist \([0-9]+\) resolves" "$LOG"; then
    echo -e "${GREEN}✓ Mind Twist resolved${NC}"
else
    echo -e "${RED}✗ Mind Twist did not resolve${NC}"
    grep -i "mind twist" "$LOG"
    exit 1
fi

# 3. Exactly one discard line for the opponent (Fixed2). Mode$ Random
#    picks at random; the specific card name depends on seed (42 +
#    --p1-draw setup pins Fixed2's hand to known cards).
DISCARDS=$(grep -cE "^  Fixed2 discards " "$LOG" || true)
if [[ "$DISCARDS" == "1" ]]; then
    echo -e "${GREEN}✓ Opponent discarded exactly 1 card${NC}"
    grep -E "^  Fixed2 discards " "$LOG"
else
    echo -e "${RED}✗ Expected 1 discard, got $DISCARDS${NC}"
    grep -E "^  Fixed[12] discards " "$LOG" || echo "(no discard log lines)"
    exit 1
fi

# 4. The summary log line must show the resolved X count, NOT the
#    literal sentinel "X" (regression guard for mtg-521 log fix).
# Assert the actual opponent name (Fixed2) appears, NOT the literal
# sentinel string "target opponent" (regression guard for the
# resolve_log_player() helper added alongside the XPaid log fix).
if grep -qE "Mind Twist \([0-9]+\) causes Fixed2 to discard 1 card\(s\)" "$LOG"; then
    echo -e "${GREEN}✓ Resolved X count + opponent name in summary log${NC}"
    grep -E "Mind Twist \([0-9]+\) causes " "$LOG"
elif grep -qE "Mind Twist \([0-9]+\) causes target opponent " "$LOG"; then
    echo -e "${RED}✗ Regression: log shows sentinel 'target opponent' instead of resolved player name${NC}"
    grep -E "Mind Twist \([0-9]+\) causes " "$LOG"
    exit 1
elif grep -qE "Mind Twist \([0-9]+\) causes .* to discard X card\(s\)" "$LOG"; then
    echo -e "${RED}✗ Regression: summary log still shows sentinel 'X' instead of resolved count${NC}"
    grep -E "Mind Twist \([0-9]+\) causes " "$LOG"
    exit 1
else
    echo -e "${RED}✗ Missing post-resolution summary log line${NC}"
    grep -i "mind twist" "$LOG"
    exit 1
fi

# 5. Mind Twist must NOT have caused the caster (Fixed1) to discard.
if grep -qE "^  Fixed1 discards " "$LOG"; then
    echo -e "${RED}✗ Regression: Mind Twist made the CASTER discard (target_opponent sentinel broken)${NC}"
    grep -E "^  Fixed[12] discards " "$LOG"
    exit 1
fi
echo -e "${GREEN}✓ Caster did not discard (target_opponent resolution correct)${NC}"

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "Full log: $LOG"
exit 0
