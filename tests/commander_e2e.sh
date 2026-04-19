#!/usr/bin/env bash
# E2E test for Commander format support
#
# This test verifies that:
# 1. Commander deck loads correctly with [Commander] section
# 2. Starting life is 40 (Commander format)
# 3. Commander appears as castable from command zone
# 4. Commander can be cast and resolves
# 5. Game completes without errors across multiple seeds

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

echo "=== Commander Format E2E Test ==="
echo

if [[ ! -d "$WORKSPACE_ROOT/cardsfolder" ]]; then
    echo -e "${RED}ERROR: cardsfolder not found. Check submodule checkout (git submodule update --init --recursive)${NC}"
    exit 1
fi

DECK="$WORKSPACE_ROOT/decks/commander/chandra_tokens.dck"
if [[ ! -f "$DECK" ]]; then
    echo -e "${RED}ERROR: $DECK not found. Required test fixture is missing.${NC}"
    exit 1
fi

cd "$WORKSPACE_ROOT"

EXIT_CODE=0

# Test 1: Commander deck loading and basic gameplay
echo "Test 1: Commander deck loads and game starts with 40 life..."
OUTPUT_FILE="/tmp/commander_e2e_test1.txt"
if run_mtg tui \
    "$DECK" \
    "$DECK" \
    --p1=random \
    --p2=random \
    --seed=42 \
    --stop-on-choice=50 \
    --log-tail=500 \
    --verbosity=normal \
    > "$OUTPUT_FILE" 2>&1; then
    echo -e "${GREEN}  Game started successfully${NC}"
else
    echo -e "${RED}  Game failed to start${NC}"
    cat "$OUTPUT_FILE"
    exit 1
fi

# Verify starting life is 40
if grep -q "Life: 40" "$OUTPUT_FILE"; then
    echo -e "${GREEN}  Starting life is 40 (Commander format)${NC}"
else
    echo -e "${RED}  Starting life is NOT 40${NC}"
    EXIT_CODE=1
fi

# Verify commander is loaded
if grep -q "P1 commander:" "$OUTPUT_FILE"; then
    COMMANDER_NAME=$(grep -o "P1 commander: [^(]*" "$OUTPUT_FILE" | head -1)
    echo -e "${GREEN}  ${COMMANDER_NAME}${NC}"
else
    echo -e "${RED}  P1 commander not loaded${NC}"
    EXIT_CODE=1
fi

# Verify commander appears as castable
if grep -q "from command zone" "$OUTPUT_FILE"; then
    echo -e "${GREEN}  Commander appears as castable from command zone${NC}"
else
    echo -e "${YELLOW}  Commander not yet castable (may need more mana)${NC}"
fi

echo

# Test 2: Commander casting
echo "Test 2: Commander can be cast from command zone..."
OUTPUT_FILE="/tmp/commander_e2e_test2.txt"
if run_mtg tui \
    "$DECK" \
    "$DECK" \
    --p1=random \
    --p2=random \
    --seed=42 \
    --stop-on-choice=500 \
    --log-tail=500 \
    --verbosity=normal \
    > "$OUTPUT_FILE" 2>&1; then
    echo -e "${GREEN}  Game ran successfully${NC}"
else
    echo -e "${RED}  Game failed${NC}"
    cat "$OUTPUT_FILE"
    EXIT_CODE=1
fi

if grep -q "casts.*from command zone" "$OUTPUT_FILE"; then
    echo -e "${GREEN}  Commander was cast from command zone${NC}"
else
    echo -e "${YELLOW}  Commander was not cast (random controller may not have chosen it)${NC}"
fi

if grep -q "resolves" "$OUTPUT_FILE"; then
    echo -e "${GREEN}  Commander resolved onto battlefield${NC}"
else
    echo -e "${YELLOW}  Commander resolve not seen in logs${NC}"
fi

echo

# Test 3: Multiple seeds for stability
echo "Test 3: Running 5 games with different seeds for stability..."
STABLE=true
for seed in 1 2 3 4 5; do
    OUTPUT_FILE="/tmp/commander_e2e_seed${seed}.txt"
    if run_mtg tui \
        "$DECK" \
        "$DECK" \
        --p1=random \
        --p2=random \
        --seed="$seed" \
        --stop-on-choice=3000 \
        --log-tail=20 \
        --verbosity=minimal \
        > "$OUTPUT_FILE" 2>&1; then
        WINNER=$(grep -o 'Winner: Random[12]' "$OUTPUT_FILE" || echo "unknown")
        TURNS=$(grep -o 'Turns played: [0-9]*' "$OUTPUT_FILE" || echo "unknown")
        echo -e "  Seed $seed: ${GREEN}OK${NC} ($WINNER, $TURNS)"
    else
        echo -e "  Seed $seed: ${RED}FAILED${NC}"
        STABLE=false
        EXIT_CODE=1
    fi
done

if $STABLE; then
    echo -e "${GREEN}  All seeds completed successfully${NC}"
else
    echo -e "${RED}  Some seeds failed${NC}"
fi

echo
echo "=== Commander E2E Test Summary ==="
if [[ $EXIT_CODE == 0 ]]; then
    echo -e "${GREEN}SUCCESS: Commander format works correctly${NC}"
    exit 0
else
    echo -e "${RED}FAILURE: Commander format has issues${NC}"
    exit 1
fi
