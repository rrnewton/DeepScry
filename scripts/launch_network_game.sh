#!/bin/bash
# Launch a network game with one native player + web server for browser client
#
# Usage: ./scripts/launch_network_game.sh [--rebuild]
#
# Options:
#   --rebuild    Force rebuild of WASM and native binary (ignores cached builds)
#
# After running, open browser to: http://localhost:8000/fancy.html
# Then select "Remote Network Game" mode, enter password "play", and connect.

set -e

# Parse arguments
FORCE_REBUILD=false
while [[ $# -gt 0 ]]; do
    case $1 in
        --rebuild)
            FORCE_REBUILD=true
            shift
            ;;
        *)
            echo "Unknown option: $1"
            echo "Usage: $0 [--rebuild]"
            exit 1
            ;;
    esac
done

# Source test helpers for ensure_mtg_binary and run_mtg_prebuilt
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../tests/lib/test_helpers.sh"

# Configuration
GAME_PORT=17771
WEB_PORT=8000
PASSWORD="play"
DECK="decks/old_school/01_rogue_rogerbrand.dck"
CONTROLLER="random"
# Fixed seeds for deterministic behavior (WASM uses seed=42, native should match)
GAME_SEED=42
CONTROLLER_SEED=43  # Different from game seed for independent randomness

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

cd "$WORKSPACE_ROOT"

# Cleanup function
cleanup() {
    echo -e "\n${YELLOW}Shutting down...${NC}"
    [ -n "$SERVER_PID" ] && kill $SERVER_PID 2>/dev/null || true
    [ -n "$CLIENT_PID" ] && kill $CLIENT_PID 2>/dev/null || true
    [ -n "$WEB_PID" ] && kill $WEB_PID 2>/dev/null || true
    wait 2>/dev/null || true
    echo -e "${GREEN}Done.${NC}"
}
trap cleanup EXIT INT TERM

# Check if we can skip builds
NEED_WASM_BUILD=true
NEED_NATIVE_BUILD=true

# Always define MTG_BIN path
export MTG_BIN="$WORKSPACE_ROOT/target/release/mtg"

if [ "$FORCE_REBUILD" = true ]; then
    echo -e "${YELLOW}--rebuild flag set, forcing full rebuild${NC}"
else
    # Check if WASM is already built with network features
    if [ -f "$WORKSPACE_ROOT/web/pkg/mtg_forge_rs.js" ] && grep -q "network_init" "$WORKSPACE_ROOT/web/pkg/mtg_forge_rs.js"; then
        echo -e "${GREEN}WASM already built with network features ✓${NC}"
        NEED_WASM_BUILD=false
    fi

    # Check if native binary already has network features
    if [ -f "$MTG_BIN" ] && "$MTG_BIN" --help 2>&1 | grep -q "connect"; then
        echo -e "${GREEN}Native binary has network features ✓${NC}"
        NEED_NATIVE_BUILD=false
    fi
fi

# Build WASM if needed (skip export if data exists to avoid clobbering native binary)
if [ "$NEED_WASM_BUILD" = true ]; then
    echo -e "${YELLOW}Building WASM with network feature...${NC}"
    # Skip export if data exists (uses existing binary, avoids clobbering)
    if [ -f "$WORKSPACE_ROOT/web/data/cards.bin" ] && [ "$FORCE_REBUILD" != true ]; then
        echo -e "${GREEN}WASM export data exists, skipping export${NC}"
        export MTG_SKIP_WASM_EXPORT=1
    fi
    make wasm-network
    # WASM build may have clobbered native binary, force rebuild
    NEED_NATIVE_BUILD=true
fi

# Build native binary with network feature if needed
if [ "$NEED_NATIVE_BUILD" = true ]; then
    echo -e "${YELLOW}Building native binary with network features...${NC}"
    cargo build --release --bin mtg --features network
fi

# Verify binary has network features
if ! "$MTG_BIN" --help 2>&1 | grep -q "connect"; then
    echo -e "${RED}ERROR: Binary missing network features${NC}"
    echo "Check that Cargo.toml has the 'network' feature defined"
    exit 1
fi
echo -e "${GREEN}All builds ready ✓${NC}"

echo -e "${CYAN}======================================${NC}"
echo -e "${CYAN}  MTG Forge Network Game Launcher${NC}"
echo -e "${CYAN}======================================${NC}"
echo ""
echo -e "Game server:  ${GREEN}ws://localhost:$GAME_PORT${NC}"
echo -e "Web server:   ${GREEN}http://localhost:$WEB_PORT${NC}"
echo -e "Password:     ${GREEN}$PASSWORD${NC}"
echo -e "Native deck:  ${GREEN}$DECK${NC}"
echo -e "Native AI:    ${GREEN}$CONTROLLER${NC}"
echo ""

# Start web server in background
echo -e "${YELLOW}Starting web server...${NC}"
cd "$WORKSPACE_ROOT/web"
python3 -m http.server $WEB_PORT > /dev/null 2>&1 &
WEB_PID=$!
cd "$WORKSPACE_ROOT"
sleep 0.5

# Start game server using run_mtg_prebuilt (with fixed seed for determinism)
echo -e "${YELLOW}Starting game server...${NC}"
run_mtg_prebuilt server \
    --port $GAME_PORT \
    --password "$PASSWORD" \
    --cardsfolder mtg-engine/cardsfolder \
    --seed $GAME_SEED \
    --network-debug \
    --verbosity normal &
SERVER_PID=$!
sleep 1

# Start native client with random controller using run_mtg_prebuilt (with fixed seed)
echo -e "${YELLOW}Starting native client (random AI)...${NC}"
run_mtg_prebuilt connect \
    --server "localhost:$GAME_PORT" \
    --password "$PASSWORD" \
    --name "RogueAI" \
    --controller "$CONTROLLER" \
    --seed-player $CONTROLLER_SEED \
    --cardsfolder mtg-engine/cardsfolder \
    "$DECK" &
CLIENT_PID=$!

echo ""
echo -e "${GREEN}======================================${NC}"
echo -e "${GREEN}  Ready! Open browser to:${NC}"
echo -e "${GREEN}  http://localhost:$WEB_PORT/fancy.html${NC}"
echo -e "${GREEN}======================================${NC}"
echo ""
echo -e "In the browser:"
echo -e "  1. Select a deck"
echo -e "  2. Click ${CYAN}'Network'${NC} in P1 controller dropdown"
echo -e "  3. Server URL: ${CYAN}ws://localhost:$GAME_PORT${NC}"
echo -e "  4. Password: ${CYAN}$PASSWORD${NC}"
echo -e "  5. Click ${CYAN}'Start Game'${NC}"
echo ""
echo -e "${YELLOW}Press Ctrl+C to stop.${NC}"
echo ""

wait $CLIENT_PID 2>/dev/null || true
echo -e "${GREEN}Game finished!${NC}"
