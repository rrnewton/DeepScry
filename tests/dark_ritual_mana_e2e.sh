#!/usr/bin/env bash
# E2E test: Dark Ritual adds {B}{B}{B}, enough (with one Swamp) to cast a
# {1}{B}{B} creature on turn 1's worth of mana.
#
# Card compat (mtg-496, 1994 Old School 'Mono Black Rogerbrand' deck mtg-560):
#   A:SP$ Mana | Produced$ B | Amount$ 3

set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"
ensure_mtg_binary
GREEN='\033[0;32m'; RED='\033[0;31m'; NC='\033[0m'
cd "$WORKSPACE_ROOT"
LOG=/tmp/dark_ritual_mana_e2e.txt

if run_mtg_with_timeout 30 tui --start-state "$WORKSPACE_ROOT/test_puzzles/dark_ritual_mana.pzl" \
    --p1=fixed --p2=zero \
    --p1-fixed-inputs="cast Dark Ritual;cast Hypnotic Specter;pass;pass" \
    --p2-fixed-inputs="" --stop-on-choice=5 --seed 42 --verbosity 3 > "$LOG" 2>&1; then
    echo -e "${GREEN}✓ Game completed${NC}"
else
    echo -e "${RED}✗ Game failed${NC}"; head -80 "$LOG"; exit 1
fi

if grep -qE "Dark Ritual \([0-9]+\) adds BBB to" "$LOG"; then
    echo -e "${GREEN}✓ Dark Ritual added {B}{B}{B}${NC}"
else
    echo -e "${RED}✗ Dark Ritual did not add BBB${NC}"; grep -E "Dark Ritual|adds" "$LOG" || true; exit 1
fi
if grep -qE "Hypnotic Specter \([0-9]+\) enters the battlefield" "$LOG"; then
    echo -e "${GREEN}✓ Ritual mana funded the Hypnotic Specter cast${NC}"
else
    echo -e "${RED}✗ Hypnotic Specter was not cast from ritual mana${NC}"; grep -E "Specter" "$LOG" || true; exit 1
fi
echo -e "${GREEN}=== Test PASSED ===${NC}"; echo "Full log: $LOG"; exit 0
