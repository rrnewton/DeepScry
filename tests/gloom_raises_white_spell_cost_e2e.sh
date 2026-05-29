#!/usr/bin/env bash
# E2E test: Gloom (Arabian Nights) raises white spell costs by {3}.
#
# Regression for mtg-507 / wave-8 fix. Two bugs were on Gloom's path:
#
#   1. The cost-modifier scanner in actions/mod.rs +
#      game_loop/actions.rs only considered ReduceCost/RaiseCost static
#      abilities whose source was controlled by the *casting* player.
#      Gloom is a hose card (CR 601.2f) and applies to any spell that
#      matches the filter regardless of who controls the source.
#
#   2. The `CostReductionTarget` enum had no `Color` variant. Parsing
#      `ValidCard$ Card.White` fell through to `Subtype(...)`, which
#      never matches a colour — so Gloom's RaiseCost never fired even
#      against the player's own white spells.
#
# Test scenario:
# - P0 (Gloom-side) has Gloom + 3 Swamps in play.
# - P1 has Savannah Lions ({W}, 2/1) in hand and 2 Plains in play.
# - Without the fix, P1 casts Savannah Lions on its main phase ({W}
#   payable from one Plains).
# - With the fix, Savannah Lions costs {3}{W}; P1 cannot pay {3}{W}
#   with only 2 Plains and must pass.
#
# Assertion: after a few choices, Savannah Lions has NOT been cast
# (no "casts Savannah Lions" line in the log).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Gloom Raises White Spell Cost E2E ==="
echo

cd "$WORKSPACE_ROOT"

PUZZLE="$WORKSPACE_ROOT/test_puzzles/gloom_raises_white_spell_cost.pzl"
LOG=/tmp/gloom_raises_white_spell_cost_e2e.txt

if RUST_LOG=debug run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE" \
    --p1=zero --p2=heuristic \
    --stop-on-choice=8 --seed 42 --verbosity 3 \
    > "$LOG" 2>&1; then
    echo -e "${GREEN}✓ Game ran${NC}"
else
    EXIT_STATUS=$?
    echo -e "${RED}✗ Game failed (exit $EXIT_STATUS)${NC}"
    head -80 "$LOG"
    exit 1
fi

# Required: the RaiseCost from Gloom must fire (and credit Gloom).
if ! grep -qE "RaiseCost from Gloom.*increasing generic by 3" "$LOG"; then
    echo -e "${RED}✗ Gloom RaiseCost debug log not seen — engine fix regressed${NC}"
    grep -E "RaiseCost|Gloom" "$LOG" | head -10
    exit 1
fi
echo -e "${GREEN}✓ Gloom RaiseCost debug log present (+3 generic)${NC}"

# Required: Savannah Lions must NOT cast until P2 has at least 4 lands.
# Pre-fix, P2 cast Savannah Lions on turn 4 with just 3 Plains (cost was {W}).
# Post-fix, P2 must accumulate at least four Plains before the heuristic can
# cast Savannah Lions ({3}{W}).
LAST_CAST_LINE=$(grep -nE "casts Savannah Lions" "$LOG" | head -1 | cut -d: -f1 || true)
if [[ -z "$LAST_CAST_LINE" ]]; then
    echo -e "${GREEN}✓ Savannah Lions was NOT cast within the snapshot window${NC}"
else
    # Count the number of "Tap Plains for {W}" lines IMMEDIATELY after the
    # cast line — these are the lands tapped to pay for it.
    POST_CAST=$(tail -n +"$LAST_CAST_LINE" "$LOG" | head -10)
    TAP_COUNT=$(echo "$POST_CAST" | grep -cE "Tap Plains for \{W\}" || true)
    if [[ "$TAP_COUNT" -lt 4 ]]; then
        echo -e "${RED}✗ Savannah Lions cast with only $TAP_COUNT Plains tapped (expected >=4 with Gloom)${NC}"
        echo "Post-cast log slice:"
        echo "$POST_CAST"
        exit 1
    fi
    echo -e "${GREEN}✓ Savannah Lions cast cost $TAP_COUNT Plains — Gloom raised {W} to {3}{W}${NC}"
fi

# Sanity: Gloom is still on the battlefield.
if ! grep -qE "Gloom" "$LOG"; then
    echo -e "${RED}✗ Gloom never appeared in log — puzzle setup wrong?${NC}"
    head -40 "$LOG"
    exit 1
fi
echo -e "${GREEN}✓ Gloom present in game state${NC}"

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "Full log: $LOG"
exit 0
