#!/usr/bin/env bash
# E2E test: Sengir Vampire — 4/4 Flying creature with DamagedCreatureDies trigger
#
# Regression test for mtg-d4da18 (Card Compatibility: Sengir Vampire).
#
# Verifies the FULL behaviour of Sengir Vampire:
# - Loads as Creature Vampire with PT 4/4
# - Has the Flying keyword (the Birds of Paradise blocker has flying so it
#   CAN block; if we used Grizzly Bears instead, the bears couldn't block)
# - Deals 4 combat damage to a 0/1 flying blocker (Birds of Paradise dies)
# - The blocked-and-killed creature triggers Sengir's
#   "Whenever a creature dealt damage by CARDNAME this turn dies,
#    put a +1/+1 counter on CARDNAME." trigger
# - +1/+1 counter is added → Sengir becomes 5/5
#
# Test scenario:
# - P1 Sengir Vampire (4/4 Flying)
# - P2 Birds of Paradise (0/1 Flying) — can legally block Sengir
# - Heuristic AI attacks with Sengir; Birds blocks; combat damage kills Birds
# - Sengir's death-trigger fires and adds a +1/+1 counter

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Sengir Vampire: 4/4 Flying + DamagedCreatureDies Trigger E2E Test ==="
echo

cd "$WORKSPACE_ROOT"

PUZZLE="$WORKSPACE_ROOT/test_puzzles/sengir_vampire_kills_creature.pzl"
LOG=/tmp/sengir_vampire_e2e.txt

if run_mtg_with_timeout 30 tui \
    --start-state "$PUZZLE" \
    --p1=heuristic --p2=zero \
    --p2-fixed-inputs="" \
    --stop-on-choice=8 --json --seed 42 --verbosity 3 \
    > "$LOG" 2>&1; then
    echo -e "${GREEN}✓ Game completed${NC}"
else
    EXIT_STATUS=$?
    echo -e "${RED}✗ Game failed (exit $EXIT_STATUS)${NC}"
    head -80 "$LOG"
    exit 1
fi

# (a) Sengir Vampire is declared as a 4/4 attacker (proves base P/T is 4/4)
if grep -qE "declares Sengir Vampire \([0-9]+\) \(4/4\) as attacker" "$LOG"; then
    echo -e "${GREEN}✓ Sengir Vampire base P/T = 4/4${NC}"
else
    echo -e "${RED}✗ Sengir Vampire base P/T not 4/4${NC}"
    grep -iE "sengir" "$LOG" | head -5
    exit 1
fi

# (b) Heuristic AI declares Sengir as attacker
if grep -qE "Player 1 declares Sengir Vampire \([0-9]+\) \(4/4\) as attacker" "$LOG"; then
    echo -e "${GREEN}✓ Sengir Vampire attacked${NC}"
else
    echo -e "${RED}✗ Sengir did not attack${NC}"
    exit 1
fi

# (c) Sengir deals 4 damage to Birds of Paradise (over its 1 toughness)
if grep -qE "Sengir Vampire \([0-9]+\) deals 4 damage to Birds of Paradise" "$LOG"; then
    echo -e "${GREEN}✓ Sengir dealt 4 damage to blocker${NC}"
else
    echo -e "${RED}✗ Sengir did not deal damage to Birds of Paradise${NC}"
    grep -iE "damage" "$LOG" | head -5
    exit 1
fi

# (d) Birds of Paradise dies
if grep -qE "Birds of Paradise \([0-9]+\) dies from combat damage" "$LOG"; then
    echo -e "${GREEN}✓ Birds of Paradise died from combat damage${NC}"
else
    echo -e "${RED}✗ Birds of Paradise did not die${NC}"
    exit 1
fi

# (e) DamagedCreatureDies trigger fires
if grep -qE "Trigger: Sengir Vampire - Whenever a creature dealt damage" "$LOG"; then
    echo -e "${GREEN}✓ Sengir's DamagedCreatureDies trigger fired${NC}"
else
    echo -e "${RED}✗ DamagedCreatureDies trigger did not fire${NC}"
    grep -iE "trigger" "$LOG" | head -5
    exit 1
fi

# (f) +1/+1 counter applied — Sengir is now 5/5
if grep -qE "Sengir Vampire \([0-9]+\) - 5/5" "$LOG"; then
    echo -e "${GREEN}✓ +1/+1 counter applied — Sengir is now 5/5${NC}"
else
    echo -e "${RED}✗ Counter not applied (Sengir still 4/4)${NC}"
    grep -E "Sengir Vampire \([0-9]+\) - " "$LOG" | head -5
    exit 1
fi

echo
echo -e "${GREEN}=== Test PASSED (full Sengir Vampire behaviour) ===${NC}"
echo "Full log: $LOG"
exit 0
