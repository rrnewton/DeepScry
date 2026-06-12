#!/usr/bin/env bash
# E2E test: Donatello, the Brains (mtg-452) TokenCreationBonus replacement effect.
#
# When tokens would be created under Donatello's controller, those tokens plus
# one additional Mutagen Token are created instead (CR 614 replacement effect).
# StaticAbility::TokenCreationBonus parses from:
#   R:Event$ CreateToken | ValidToken$ Card.YouCtrl | ReplaceWith$ DBReplace
#   SVar:DBReplace:DB$ ReplaceToken | Type$ AddToken | Amount$ 1 | TokenScript$ c_a_mutagen_sac
#
# The puzzle has Donatello on P1's battlefield + mana + Raise the Alarm in hand.
# Heuristic P1 casts Raise the Alarm (creates 2 Soldier tokens); the engine
# must ALSO create 1 Mutagen Token via the replacement effect.

set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"
ensure_mtg_binary
GREEN='\033[0;32m'; RED='\033[0;31m'; NC='\033[0m'
cd "$WORKSPACE_ROOT"
LOG=/tmp/donatello_token_bonus_e2e.txt

if run_mtg_with_timeout 30 tui \
    --start-state "$WORKSPACE_ROOT/test_puzzles/donatello_token_replacement.pzl" \
    --p1=heuristic --p2=heuristic \
    --seed 42 --verbosity 3 > "$LOG" 2>&1; then
    echo -e "${GREEN}✓ Game completed${NC}"
else
    echo -e "${RED}✗ Game failed${NC}"; head -80 "$LOG"; exit 1
fi

# Verify Soldier tokens created (Raise the Alarm's direct tokens)
if grep -qE "Created Soldier Token under" "$LOG"; then
    echo -e "${GREEN}✓ Soldier tokens created (Raise the Alarm effect)${NC}"
else
    echo -e "${RED}✗ No Soldier tokens in log${NC}"; grep -E "token|Token" "$LOG" || true; exit 1
fi

# The critical assertion: Mutagen Token created via Donatello's replacement
if grep -qE "Created Mutagen Token under" "$LOG"; then
    echo -e "${GREEN}✓ Mutagen Token created via Donatello's TokenCreationBonus replacement${NC}"
else
    echo -e "${RED}✗ No Mutagen Token created — replacement effect broken${NC}"
    grep -E "Donatello|Mutagen|token|WARN" "$LOG" || true
    exit 1
fi

# Ensure no WARN about missing token definition
if grep -qE "WARN.*c_a_mutagen_sac" "$LOG"; then
    echo -e "${RED}✗ Mutagen token definition c_a_mutagen_sac was not loaded${NC}"
    grep "WARN" "$LOG" || true
    exit 1
fi

echo -e "${GREEN}=== Test PASSED ===${NC}"; echo "Full log: $LOG"; exit 0
