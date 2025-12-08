#!/usr/bin/env bash
# E2E test for gamelog equivalence between local and network modes
#
# This test verifies that the same game produces identical GAMELOG output
# when run locally vs through the network stack (server + 2 clients).
#
# Current status: Compares local mode vs server-side logs (2-way comparison)
# Future: Will extend to 4-way comparison including client shadow state logs
#
# Related issue: mtg-037fw

set -euo pipefail

# Get script directory and source shared test helpers
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

# Ensure release binary is built
ensure_mtg_binary

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo "=== Gamelog Equivalence E2E Test ==="
echo
echo "This test verifies that local and network modes produce identical game logs."
echo

cd "$WORKSPACE_ROOT"

# Check if cardsfolder exists
if [[ ! -d "$WORKSPACE_ROOT/cardsfolder" ]]; then
    echo -e "${YELLOW}Warning: cardsfolder not found, skipping test${NC}"
    exit 0
fi

# Use a simple deck for faster testing
DECK="$WORKSPACE_ROOT/decks/julian_spiderman_draft.dck"

if [[ ! -f "$DECK" ]]; then
    echo -e "${RED}Error: $DECK not found${NC}"
    exit 1
fi

# Fixed seed for deterministic comparison
SEED=12345

# Use fixed inputs for deterministic choices across both modes
# This ensures the same choices are made regardless of process boundaries
# Format: semicolon-separated commands, numeric indices (0=pass, 1=first option, etc)
# The sequence: play a land, then pass priority repeatedly for a short game
P1_FIXED="1;0;0;0;0;0;0;0;0;0;0;0;0;0;0;0;0;0;0;0;0;0;0;0;0;0;0;0;0;0"
P2_FIXED="1;0;0;0;0;0;0;0;0;0;0;0;0;0;0;0;0;0;0;0;0;0;0;0;0;0;0;0;0;0"

LOCAL_OUTPUT="/tmp/gamelog_local.txt"
NETWORK_OUTPUT="/tmp/gamelog_network.txt"
LOCAL_GAMELOG="/tmp/gamelog_local_filtered.txt"
NETWORK_GAMELOG="/tmp/gamelog_network_filtered.txt"

# Run local mode game with --tag-gamelogs
echo "Running game in local mode with --tag-gamelogs..."
if ! run_mtg_prebuilt tui \
    "$DECK" \
    --p1 fixed \
    --p2 fixed \
    --p1-fixed-inputs "$P1_FIXED" \
    --p2-fixed-inputs "$P2_FIXED" \
    --seed "$SEED" \
    --tag-gamelogs \
    --stop-when-fixed-exhausted \
    --verbosity normal \
    > "$LOCAL_OUTPUT" 2>&1; then
    echo -e "${RED}✗ Local mode game failed${NC}"
    echo "Output:"
    cat "$LOCAL_OUTPUT"
    exit 1
fi
echo -e "${GREEN}✓ Local mode game completed${NC}"

# Extract just [GAMELOG...] lines, excluding duplicates with line numbers prefix
# The raw output sometimes has "  [linenum] [GAMELOG..." duplicates at the end
# Valid lines start with whitespace then [GAMELOG directly, not [number] then [GAMELOG
grep '^ *\[GAMELOG' "$LOCAL_OUTPUT" > "$LOCAL_GAMELOG" || true
LOCAL_COUNT=$(wc -l < "$LOCAL_GAMELOG")
echo "  Local mode produced $LOCAL_COUNT GAMELOG entries"

if [[ "$LOCAL_COUNT" -eq 0 ]]; then
    echo -e "${RED}✗ No GAMELOG entries found in local mode${NC}"
    echo "First 50 lines of output:"
    head -50 "$LOCAL_OUTPUT"
    exit 1
fi

# Now run network mode with the same parameters
echo
echo "Running game in network mode with --tag-gamelogs..."

# Use the networked script directly with tag-gamelogs
# Note: Network mode with fixed controllers is simpler since choices are predetermined
if ! python3 "$WORKSPACE_ROOT/scripts/mtg_tui_networked.py" \
    "$DECK" \
    --p1 fixed \
    --p2 fixed \
    --p1-fixed-inputs "$P1_FIXED" \
    --p2-fixed-inputs "$P2_FIXED" \
    --seed "$SEED" \
    --tag-gamelogs \
    --verbosity normal \
    > "$NETWORK_OUTPUT" 2>&1; then
    # Network mode may exit with non-zero even on success, check output
    echo -e "${YELLOW}⚠ Network mode exited with non-zero, checking output...${NC}"
fi

# Extract just [GAMELOG...] lines
grep '\[GAMELOG' "$NETWORK_OUTPUT" > "$NETWORK_GAMELOG" || true
NETWORK_COUNT=$(wc -l < "$NETWORK_GAMELOG")
echo "  Network mode produced $NETWORK_COUNT GAMELOG entries"

if [[ "$NETWORK_COUNT" -eq 0 ]]; then
    echo -e "${RED}✗ No GAMELOG entries found in network mode${NC}"
    echo "First 50 lines of output:"
    head -50 "$NETWORK_OUTPUT"
    exit 1
fi

echo -e "${GREEN}✓ Network mode game completed${NC}"

# Compare the gamelogs
echo
echo "Comparing gamelogs..."

# Since network mode can't use --stop-on-choice, compare only the first N lines
# that match the local mode output
COMPARE_LINES=$LOCAL_COUNT

head -"$COMPARE_LINES" "$LOCAL_GAMELOG" > /tmp/local_head.txt
head -"$COMPARE_LINES" "$NETWORK_GAMELOG" > /tmp/network_head.txt

if diff -q /tmp/local_head.txt /tmp/network_head.txt > /dev/null 2>&1; then
    echo -e "${GREEN}✓ GAMELOGS MATCH! (first $COMPARE_LINES entries)${NC}"
    echo
    echo "Sample entries:"
    head -5 "$LOCAL_GAMELOG"
    EXIT_CODE=0
else
    echo -e "${RED}✗ GAMELOGS DIFFER${NC}"
    echo
    echo "Differences (first 20 lines):"
    diff /tmp/local_head.txt /tmp/network_head.txt | head -20 || true
    echo
    echo "Local first 10 entries:"
    head -10 "$LOCAL_GAMELOG"
    echo
    echo "Network first 10 entries:"
    head -10 "$NETWORK_GAMELOG"
    EXIT_CODE=1
fi

echo
echo "=== Test Complete ==="
echo "Full logs available at:"
echo "  Local:   $LOCAL_OUTPUT"
echo "  Network: $NETWORK_OUTPUT"
echo "  Local gamelog:   $LOCAL_GAMELOG"
echo "  Network gamelog: $NETWORK_GAMELOG"

exit $EXIT_CODE
