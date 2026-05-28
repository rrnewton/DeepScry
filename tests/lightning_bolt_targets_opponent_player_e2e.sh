#!/usr/bin/env bash
# E2E test: Lightning Bolt can target the opponent PLAYER, not just creatures.
#
# Regression test for mtg-lxrqz (user bug report, fix-gameplay-bugs-4pack):
# Before the fix, `Controller::choose_targets` accepted only `&[CardId]`.
# Players have `PlayerId` not `CardId`, so the legal-targets list for
# `SP$ DealDamage | ValidTgts$ Any` (Lightning Bolt) never included
# Players. The user reported "I have 2 creatures on the battlefield
# (opponent none). I cast lightning bolt to damage opponent, but they
# are not one of the targets I am presented with! Only my own creatures
# or 'No target'."
#
# After the fix, players are encoded as sentinel CardIds in valid_targets
# (`PLAYER_TARGET_BASE - PlayerId`), decoded back to TargetRef::Player at
# effect-resolution time, and the resolve_spell fizzle check (CR 608.2b)
# learned to treat the sentinel as legal.
#
# Test scenario:
# - P0 has Lightning Bolt + 3 Mountains. NO creatures anywhere.
# - With no creatures the only legal "any target" picks are the two
#   players, so the fixed controller (which auto-picks index 0) is
#   forced to point Bolt at a Player sentinel.
# - The opponent's life must drop by 3 (20 -> 17).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Lightning Bolt Targets Opponent Player E2E ==="
echo

cd "$WORKSPACE_ROOT"

PUZZLE="$WORKSPACE_ROOT/test_puzzles/bolt_only_player_target.pzl"
LOG=/tmp/lightning_bolt_targets_opponent_player_e2e.txt

if run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE" \
    --p1=fixed --p2=zero \
    --p1-fixed-inputs="cast Lightning Bolt;pass" \
    --p2-fixed-inputs="" \
    --stop-on-choice=5 --json --seed 42 --verbosity 3 \
    > "$LOG" 2>&1; then
    echo -e "${GREEN}✓ Game completed${NC}"
else
    EXIT_STATUS=$?
    echo -e "${RED}✗ Game failed (exit $EXIT_STATUS)${NC}"
    head -80 "$LOG"
    exit 1
fi

# Required: the targeting log line shows a Player (not an "Unknown" sentinel).
if grep -qE "→ targeting Player " "$LOG"; then
    echo -e "${GREEN}✓ Bolt targeted a Player${NC}"
else
    echo -e "${RED}✗ Bolt did NOT log targeting a Player${NC}"
    grep -E "→ targeting|targeting " "$LOG" || echo "(no targeting lines)"
    exit 1
fi

# Required: the damage log line names a Player (not "Unknown").
if grep -qE "Lightning Bolt \([0-9]+\) deals 3 damage to Player " "$LOG"; then
    echo -e "${GREEN}✓ Bolt damage logged against a Player${NC}"
else
    echo -e "${RED}✗ Bolt damage was NOT routed to a Player (sentinel leak)${NC}"
    grep -E "deals .* damage to" "$LOG" || echo "(no damage line)"
    exit 1
fi

# Required: the OPPONENT (Player 2) takes the 3 damage. The --p1=fixed
# controller casts "Lightning Bolt" without an explicit target arg, so the
# engine auto-resolves to the FIRST player-target. Since mtg-p43i3 the valid
# target list is ordered opponents-first (most targeted spells aim at an
# opponent), so the first player-target is now the opponent — not the caster.
# Self-targeting is still legal (CR 115.4: "any target" includes any player,
# including its controller); we just no longer pick self by default.
if grep -qE "deals 3 damage to Player 2" "$LOG" && grep -qE "Life: 17" "$LOG"; then
    echo -e "${GREEN}✓ Opponent (Player 2) took 3 damage, life settled at 17 (opponents-first default)${NC}"
else
    echo -e "${RED}✗ Opponent did NOT take the auto-resolved Bolt damage${NC}"
    grep -E "deals .* damage to|Life:" "$LOG" | head -10
    exit 1
fi

# --- Choice-list rendering (mtg-p43i3): label + ordering ---
# Drive the same puzzle with the interactive (human stdin) controller and pin
# the rendered target menu: opponent FIRST with "(them)", caster's own player
# LAST with "(you)" — never the card-target "(theirs)"/"(yours)" labels.
MENU_LOG=/tmp/lightning_bolt_player_target_menu.txt
# stdin: cast Lightning Bolt [1], then quit-ish; we only need the target menu.
printf '1\n1\n0\n' | run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE" --p1 tui --p2 zero --seed 42 \
    > "$MENU_LOG" 2>&1 || true

if grep -qE '\[0\] Player 2 \(them\)' "$MENU_LOG" \
    && grep -qE '\[1\] Player 1 \(you\)' "$MENU_LOG"; then
    echo -e "${GREEN}✓ Target menu: opponent '[0] Player 2 (them)' before caster '[1] Player 1 (you)'${NC}"
else
    echo -e "${RED}✗ Target menu label/order wrong (expected opponent-first '(them)' then '(you)')${NC}"
    grep -E "Targeting for|\[0\]|\[1\]" "$MENU_LOG" | head -10
    exit 1
fi

if grep -qE '\(theirs\)|\(yours\)' "$MENU_LOG"; then
    echo -e "${RED}✗ Player targets mislabeled with card-target '(theirs)/(yours)'${NC}"
    grep -E '\(theirs\)|\(yours\)' "$MENU_LOG" | head
    exit 1
else
    echo -e "${GREEN}✓ No card-target '(theirs)/(yours)' labels leaked onto player targets${NC}"
fi

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "Full log: $LOG"
echo "Menu log: $MENU_LOG"
exit 0
