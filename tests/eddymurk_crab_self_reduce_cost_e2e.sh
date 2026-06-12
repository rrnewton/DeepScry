#!/usr/bin/env bash
# E2E test: Eddymurk Crab self-ReduceCost Amount$X (graveyard-count dynamic cost reduction).
#
# Eddymurk Crab: S:Mode$ ReduceCost | ValidCard$ Card.Self | Amount$ X | EffectZone$ All
# with SVar:X:Count$ValidGraveyard Instant.YouOwn,Sorcery.YouOwn
# → costs {1} less for each instant or sorcery card in your graveyard.
#
# Before the fix (two-part bug):
#   1. Amount$X was parsed as value.parse::<u8>().unwrap_or(0) = 0 (not a number).
#      Fix: DynamicAmount::parse("X", &svars) → CostReductionAmount::Dynamic(CountExpression::ValidGraveyard).
#   2. ValidCard$ Card.Self was mapped to CostReductionTarget::AllSpells, and the
#      battlefield-scan loop only iterated over permanents already in play — so
#      the card's own ReduceCost was never applied while the card was still in hand.
#      Fix: CostReductionTarget::SelfCard + a dedicated self-ReduceCost pass in
#      calculate_effective_cost that reads the card's own static abilities before
#      the battlefield scan.
#   3. CountExpression::ValidGraveyard did not support comma-separated type filters
#      like "Instant.YouOwn,Sorcery.YouOwn".
#      Fix: count_cards_matching_filter now handles comma-split filters (union/dedup).
#
# Test scenario:
#   P0 has 5 instants/sorceries in graveyard → 5-mana reduction.
#   Eddymurk Crab base cost: {5}{U}{U} (7 mana).
#   Effective cost: {0}{U}{U} = 2 blue mana.
#   P0 has exactly 2 Islands in play → can pay {U}{U}.
#   Expected: P0 can cast Eddymurk Crab, it resolves, enters as 5/5.
#
# Reproducer:
#   ./target/release/mtg tui \
#     --start-state test_puzzles/eddymurk_crab_reduce_cost_graveyard.pzl \
#     --p1=zero --p2=zero \
#     --stop-on-choice=10 --seed 42 --verbosity 3

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

ensure_mtg_binary

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

echo "=== Eddymurk Crab: Self-ReduceCost Amount\$X (graveyard count) E2E Test ==="
echo

cd "$WORKSPACE_ROOT"

LOG=/tmp/eddymurk_crab_reduce_cost_e2e.txt

if run_mtg_with_timeout 40 tui \
    --start-state "$WORKSPACE_ROOT/test_puzzles/eddymurk_crab_reduce_cost_graveyard.pzl" \
    --p1=zero --p2=zero \
    --stop-on-choice=10 --seed 42 --verbosity 3 \
    > "$LOG" 2>&1; then
    echo -e "${GREEN}✓ Game completed${NC}"
else
    EXIT_STATUS=$?
    echo -e "${RED}✗ Game failed (exit $EXIT_STATUS)${NC}"
    head -60 "$LOG"
    exit 1
fi

# Eddymurk Crab must be castable and must resolve (with the fix).
grep -qE "casts Eddymurk Crab|Player 1 casts Eddymurk Crab" "$LOG" \
    || { echo -e "${RED}✗ Eddymurk Crab was NOT cast (ReduceCost fix did not make it affordable)${NC}"; cat "$LOG" | head -60; exit 1; }
echo -e "${GREEN}✓ Eddymurk Crab was cast (cost reduction made it affordable)${NC}"

grep -qE "Eddymurk Crab.*resolves" "$LOG" \
    || { echo -e "${RED}✗ Eddymurk Crab did NOT resolve${NC}"; exit 1; }
echo -e "${GREEN}✓ Eddymurk Crab resolved${NC}"

# P0 should only tap Islands (UU), not any 5 generic mana
TAPS=$(grep -E "Tap Island for" "$LOG" | wc -l)
if [ "$TAPS" -lt 2 ]; then
    echo -e "${RED}✗ Expected at least 2 Island taps for {U}{U}, got $TAPS${NC}"
    exit 1
fi
echo -e "${GREEN}✓ Paid with Islands only ({U}{U}, 5 generic reduced to 0)${NC}"

# Eddymurk Crab should enter as a 5/5 on the battlefield
grep -qE "Eddymurk Crab.*5/5" "$LOG" \
    || { echo -e "${RED}✗ Eddymurk Crab did not enter as 5/5${NC}"; exit 1; }
echo -e "${GREEN}✓ Eddymurk Crab enters battlefield as 5/5${NC}"

echo
echo -e "${GREEN}=== Test PASSED ===${NC}"
