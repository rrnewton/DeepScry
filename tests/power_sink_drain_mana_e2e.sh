#!/usr/bin/env bash
# E2E test: Power Sink ({X}{U} Instant — "Counter target spell unless its
# controller pays {X}. If that player doesn't, they tap all lands with mana
# abilities they control and lose all unspent mana."). Regression test for
# "Card Compatibility: Power Sink" (mtg-532) and the general `DB$ DrainMana`
# effect.
#
# Card script:
#   A:SP$ Counter | UnlessCost$ X | TargetType$ Spell | SubAbility$ TapLands
#   SVar:TapLands:DB$ TapAll | ValidCards$ Land.hasManaAbility
#       | Defined$ TargetedController | SubAbility$ ManaLose
#   SVar:ManaLose:DB$ DrainMana | Defined$ TargetedController
#
# Before the fix, `DB$ DrainMana` parsed to ApiType::Unknown("DrainMana") and
# resolved as a logged no-op ("Unimplemented effect 'DrainMana'"), so the
# "lose all unspent mana" rider never fired. After the fix it is a concrete
# Effect::DrainMana that empties the countered spell's controller's pool.
#
# The mechanical drain of a non-empty pool (and that the OTHER player's pool is
# untouched) is proven by the unit tests test_drain_mana_empties_pool and
# test_card_compat_power_sink. This e2e proves the in-game integration: Power
# Sink counters the spell, taps the opponent's mana lands, and emits the
# "loses all unspent mana" log line with NO "Unimplemented effect 'DrainMana'"
# warning.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Power Sink DrainMana E2E Test ==="
echo

cd "$WORKSPACE_ROOT"

PUZZLE="$WORKSPACE_ROOT/test_puzzles/power_sink_drain_mana.pzl"
LOG=/tmp/power_sink_drain_mana_e2e.txt

# P1 (Player 1) casts Lightning Bolt; P2 (Player 2) responds with Power Sink,
# X=4. P1 cannot pay {4}, so Power Sink counters the Bolt, taps P1's mana lands,
# and drains P1's unspent mana.
if run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE" \
    --p1=fixed --p2=fixed \
    --p1-fixed-inputs="cast Lightning Bolt;*;*" \
    --p2-fixed-inputs="cast Power Sink;4;*;*;*" \
    --stop-on-choice=3 --seed 42 --verbosity 3 \
    > "$LOG" 2>&1; then
    echo -e "${GREEN}✓ Game completed${NC}"
else
    EXIT_STATUS=$?
    echo -e "${RED}✗ Game failed (exit $EXIT_STATUS)${NC}"
    head -80 "$LOG"
    exit 1
fi

# 1. DrainMana must NOT be unimplemented anymore.
if grep -qiE "Unimplemented effect 'DrainMana'|unimplemented effect .DrainMana" "$LOG"; then
    echo -e "${RED}✗ DrainMana still resolves as an unimplemented no-op${NC}"
    grep -iE "drain|unimplemented" "$LOG" | head
    exit 1
fi
echo -e "${GREEN}✓ DrainMana is no longer an unimplemented no-op${NC}"

# 2. The counter + tap-all-mana-lands + drain rider must all fire.
if grep -qE "Power Sink \([0-9]+\) counters Lightning Bolt" "$LOG" \
   && grep -qE "taps all matching permanents" "$LOG" \
   && grep -qE "loses all unspent mana" "$LOG"; then
    echo -e "${GREEN}✓ Power Sink counters the spell, taps mana lands, and drains unspent mana${NC}"
else
    echo -e "${RED}✗ Power Sink resolution chain incomplete${NC}"
    grep -iE "power sink|counter|tap|unspent|drain" "$LOG" | head -15
    exit 1
fi

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "Log: $LOG"
exit 0
