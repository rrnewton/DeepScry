#!/usr/bin/env bash
# E2E test: Sol Ring taps for {C}{C} and helps pay for a cast.
#
# Card compat (mtg-543, 1994 Old School 'Mono Black Rogerbrand' deck mtg-560):
#   A:AB$ Mana | Cost$ T | Produced$ C | Amount$ 2
# Sol Ring's mana ability is offered while paying for a spell (mana abilities
# are not standalone priority actions). The puzzle casts a {4} Icy Manipulator
# from Sol Ring's {C}{C} plus two Swamps.

set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"
ensure_mtg_binary
GREEN='\033[0;32m'; RED='\033[0;31m'; NC='\033[0m'
cd "$WORKSPACE_ROOT"
LOG=/tmp/sol_ring_mana_e2e.txt

if run_mtg_with_timeout 30 tui --start-state "$WORKSPACE_ROOT/test_puzzles/sol_ring_mana.pzl" \
    --p1=fixed --p2=zero \
    --p1-fixed-inputs="cast Icy Manipulator;pass;pass" \
    --p2-fixed-inputs="" --stop-on-choice=4 --seed 42 --verbosity 3 > "$LOG" 2>&1; then
    echo -e "${GREEN}✓ Game completed${NC}"
else
    echo -e "${RED}✗ Game failed${NC}"; head -80 "$LOG"; exit 1
fi

if grep -qE "Tap Sol Ring for mana" "$LOG"; then
    echo -e "${GREEN}✓ Sol Ring tapped for mana${NC}"
else
    echo -e "${RED}✗ Sol Ring was not tapped for mana${NC}"; grep -E "Sol Ring" "$LOG" || true; exit 1
fi
if grep -qE "Icy Manipulator \([0-9]+\) resolves" "$LOG"; then
    echo -e "${GREEN}✓ Sol Ring mana funded the {4} cast${NC}"
else
    echo -e "${RED}✗ Icy Manipulator did not resolve${NC}"; grep -E "Icy" "$LOG" || true; exit 1
fi
echo -e "${GREEN}=== Test PASSED ===${NC}"; echo "Full log: $LOG"; exit 0
