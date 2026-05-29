#!/usr/bin/env bash
# E2E test: Demonic Tutor searches the library and puts a card into hand.
#
# Card compat (mtg-497, 1994 Old School 'Mono Black Rogerbrand' deck mtg-560):
#   A:SP$ ChangeZone | Origin$ Library | Destination$ Hand | ChangeNum$ 1 | Mandatory$ True

set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"
ensure_mtg_binary
GREEN='\033[0;32m'; RED='\033[0;31m'; NC='\033[0m'
cd "$WORKSPACE_ROOT"
LOG=/tmp/demonic_tutor_search_e2e.txt

if run_mtg_with_timeout 30 tui --start-state "$WORKSPACE_ROOT/test_puzzles/demonic_tutor_search.pzl" \
    --p1=fixed --p2=zero \
    --p1-fixed-inputs="cast Demonic Tutor;0;pass;pass" \
    --p2-fixed-inputs="" --stop-on-choice=5 --seed 42 --verbosity 3 > "$LOG" 2>&1; then
    echo -e "${GREEN}✓ Game completed${NC}"
else
    echo -e "${RED}✗ Game failed${NC}"; head -80 "$LOG"; exit 1
fi

if grep -qE "Demonic Tutor \([0-9]+\) searches .* library .* and puts it into Hand" "$LOG"; then
    echo -e "${GREEN}✓ Demonic Tutor searched library into hand${NC}"
else
    echo -e "${RED}✗ Demonic Tutor did not search into hand${NC}"; grep -E "Demonic Tutor|search|found" "$LOG" || true; exit 1
fi
if grep -qE "Library search: found Black Lotus" "$LOG"; then
    echo -e "${GREEN}✓ The chosen card (Black Lotus) was retrieved${NC}"
else
    echo -e "${RED}✗ Expected to retrieve Black Lotus${NC}"; grep -E "found|search" "$LOG" || true; exit 1
fi
echo -e "${GREEN}=== Test PASSED ===${NC}"; echo "Full log: $LOG"; exit 0
