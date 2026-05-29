#!/usr/bin/env bash
# E2E test: Greed's "{B}, Pay 2 life: Draw a card" activated ability draws.
#
# Card compat (mtg-508, 1994 Old School 'Mono Black Rogerbrand' deck mtg-560):
#   A:AB$ Draw | Cost$ B PayLife<2> | NumCards$ 1
# The pay-life portion of the cost is deducted by the engine cost-payment path
# (Cost::PayLife); this test asserts the activation + draw in real play.

set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"
ensure_mtg_binary
GREEN='\033[0;32m'; RED='\033[0;31m'; NC='\033[0m'
cd "$WORKSPACE_ROOT"
LOG=/tmp/greed_draw_e2e.txt

if run_mtg_with_timeout 30 tui --start-state "$WORKSPACE_ROOT/test_puzzles/greed_draw.pzl" \
    --p1=fixed --p2=zero \
    --p1-fixed-inputs="activate Greed;pass;pass" \
    --p2-fixed-inputs="" --stop-on-choice=2 --seed 42 --verbosity 3 > "$LOG" 2>&1; then
    echo -e "${GREEN}✓ Game completed${NC}"
else
    echo -e "${RED}✗ Game failed${NC}"; head -80 "$LOG"; exit 1
fi

if grep -qE "Greed activates ability: Draw a card" "$LOG"; then
    echo -e "${GREEN}✓ Greed activated its draw ability${NC}"
else
    echo -e "${RED}✗ Greed did not activate${NC}"; grep -E "Greed" "$LOG" || true; exit 1
fi
if grep -qE "Player 1 draws " "$LOG"; then
    echo -e "${GREEN}✓ A card was drawn${NC}"
else
    echo -e "${RED}✗ No card drawn${NC}"; grep -E "draws" "$LOG" || true; exit 1
fi
echo -e "${GREEN}=== Test PASSED ===${NC}"; echo "Full log: $LOG"; exit 0
