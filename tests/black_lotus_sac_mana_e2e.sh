#!/usr/bin/env bash
# E2E test: Black Lotus taps + sacrifices for three mana of one color.
#
# Card compat (mtg-485, 1994 Old School 'Mono Black Rogerbrand' deck mtg-560):
#   A:AB$ Mana | Cost$ T Sac<1/CARDNAME> | Produced$ Any | Amount$ 3
# The puzzle casts a {1}{B}{B} Hypnotic Specter funded entirely by Black Lotus,
# which then goes to the graveyard from the sacrifice cost.

set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"
ensure_mtg_binary
GREEN='\033[0;32m'; RED='\033[0;31m'; NC='\033[0m'
cd "$WORKSPACE_ROOT"
LOG=/tmp/black_lotus_sac_mana_e2e.txt

if run_mtg_with_timeout 30 tui --start-state "$WORKSPACE_ROOT/test_puzzles/black_lotus_sac_mana.pzl" \
    --p1=fixed --p2=zero \
    --p1-fixed-inputs="cast Hypnotic Specter;pass;pass" \
    --p2-fixed-inputs="" --stop-on-choice=4 --seed 42 --verbosity 3 > "$LOG" 2>&1; then
    echo -e "${GREEN}✓ Game completed${NC}"
else
    echo -e "${RED}✗ Game failed${NC}"; head -80 "$LOG"; exit 1
fi

if grep -qE "Tap Black Lotus for \{B\}\{B\}\{B\}" "$LOG"; then
    echo -e "${GREEN}✓ Black Lotus produced three black mana${NC}"
else
    echo -e "${RED}✗ Black Lotus did not produce BBB${NC}"; grep -E "Black Lotus" "$LOG" || true; exit 1
fi
if grep -qE "Black Lotus \([0-9]+\) goes to graveyard" "$LOG"; then
    echo -e "${GREEN}✓ Black Lotus was sacrificed (to graveyard)${NC}"
else
    echo -e "${RED}✗ Black Lotus was not sacrificed${NC}"; grep -E "Black Lotus|graveyard" "$LOG" || true; exit 1
fi
if grep -qE "Hypnotic Specter \([0-9]+\) enters the battlefield" "$LOG"; then
    echo -e "${GREEN}✓ Lotus mana funded the Hypnotic Specter cast${NC}"
else
    echo -e "${RED}✗ Hypnotic Specter was not cast${NC}"; grep -E "Specter" "$LOG" || true; exit 1
fi
echo -e "${GREEN}=== Test PASSED ===${NC}"; echo "Full log: $LOG"; exit 0
