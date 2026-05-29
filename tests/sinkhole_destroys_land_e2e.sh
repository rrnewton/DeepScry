#!/usr/bin/env bash
# E2E test: Sinkhole destroys a target land.
#
# Card compat (mtg-542, 1994 Old School 'Mono Black Rogerbrand' deck mtg-560):
#   A:SP$ Destroy | ValidTgts$ Land
# The puzzle makes the opponent's Plains the only legal land target.

set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"
ensure_mtg_binary
GREEN='\033[0;32m'; RED='\033[0;31m'; NC='\033[0m'
cd "$WORKSPACE_ROOT"
LOG=/tmp/sinkhole_destroys_land_e2e.txt

if run_mtg_with_timeout 30 tui --start-state "$WORKSPACE_ROOT/test_puzzles/sinkhole_destroys_land.pzl" \
    --p1=fixed --p2=zero \
    --p1-fixed-inputs="cast Sinkhole;pass;pass" \
    --p2-fixed-inputs="" --stop-on-choice=4 --seed 42 --verbosity 3 > "$LOG" 2>&1; then
    echo -e "${GREEN}✓ Game completed${NC}"
else
    echo -e "${RED}✗ Game failed${NC}"; head -80 "$LOG"; exit 1
fi

if grep -qE "Sinkhole \([0-9]+\) destroys Plains" "$LOG" && grep -qE "Plains \([0-9]+\) goes to graveyard" "$LOG"; then
    echo -e "${GREEN}✓ Sinkhole destroyed the opponent's land${NC}"
else
    echo -e "${RED}✗ Sinkhole did not destroy the land${NC}"; grep -E "Sinkhole|Plains|graveyard" "$LOG" || true; exit 1
fi
echo -e "${GREEN}=== Test PASSED ===${NC}"; echo "Full log: $LOG"; exit 0
