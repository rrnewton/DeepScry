#!/usr/bin/env bash
# E2E test: Will-o'-the-Wisp's {B}: Regenerate ability grants a regeneration
# shield when activated in a real game.
#
# Card compat (mtg-557, 1994 Old School 'Mono Black Rogerbrand' deck mtg-560):
#   K:Flying
#   A:AB$ Regenerate | Cost$ B
#
# The shield's destruction-prevention semantics (prevents one destroy, consumed
# once, removed from combat) are exercised by the engine regeneration unit
# tests (test_regeneration_shield_prevents_destroy_effect et al. in
# keywords.rs). This test confirms the activated ability is offered and grants
# a shield during actual play.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Will-o'-the-Wisp Regenerate E2E ==="
echo

cd "$WORKSPACE_ROOT"

PUZZLE="$WORKSPACE_ROOT/test_puzzles/will_o_the_wisp_regenerate.pzl"
LOG=/tmp/will_o_the_wisp_regenerate_e2e.txt

if run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE" \
    --p1=fixed --p2=zero \
    --p1-fixed-inputs="activate Will;pass;pass" \
    --p2-fixed-inputs="" \
    --stop-on-choice=4 --seed 42 --verbosity 3 \
    > "$LOG" 2>&1; then
    echo -e "${GREEN}✓ Game completed${NC}"
else
    EXIT_STATUS=$?
    echo -e "${RED}✗ Game failed (exit $EXIT_STATUS)${NC}"
    head -80 "$LOG"
    exit 1
fi

# Required: the regenerate ability resolved.
if grep -qE "Will-o'-the-Wisp activates ability: Regenerate" "$LOG"; then
    echo -e "${GREEN}✓ Regenerate ability activated${NC}"
else
    echo -e "${RED}✗ Regenerate ability not activated${NC}"
    grep -E "Will-o" "$LOG" || echo "(none)"
    exit 1
fi

# Required: a regeneration shield was granted.
if grep -qE "gains a regeneration shield" "$LOG"; then
    echo -e "${GREEN}✓ Regeneration shield granted${NC}"
else
    echo -e "${RED}✗ No regeneration shield granted${NC}"
    grep -E "shield|regen" "$LOG" || echo "(none)"
    exit 1
fi

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
echo "Full log: $LOG"
exit 0
