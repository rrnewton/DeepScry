#!/bin/bash
# Launch a network game with one native player + web server for browser client
#
# Usage: ./scripts/launch_network_game.sh
#
# After running, open browser to: http://localhost:8000/fancy.html
# Then select "Network" mode, enter password "play", and connect.

set -e

# Source test helpers for ensure_mtg_binary and run_mtg_prebuilt
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../tests/lib/test_helpers.sh"

# Configuration
GAME_PORT=17771
WEB_PORT=8000
PASSWORD="play"
DECK="decks/old_school/01_rogue_rogerbrand.dck"
CONTROLLER="random"

# Colors
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

# Build native binary with network feature
ensure_mtg_binary

# Verify binary has network features (connect subcommand)
if ! "$MTG_BIN" --help 2>&1 | grep -q "connect"; then
    echo -e "${YELLOW}Binary missing network features, rebuilding...${NC}"
    cargo build --release --bin mtg --features network
fi

# Build WASM with network feature using Makefile target
echo -e "${YELLOW}Building WASM with network feature...${NC}"
make wasm-network

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

# Start game server using run_mtg_prebuilt
echo -e "${YELLOW}Starting game server...${NC}"
run_mtg_prebuilt server \
    --port $GAME_PORT \
    --password "$PASSWORD" \
    --cardsfolder mtg-engine/cardsfolder \
    --verbosity normal &
SERVER_PID=$!
sleep 1

# Start native client with random controller using run_mtg_prebuilt
echo -e "${YELLOW}Starting native client (random AI)...${NC}"
run_mtg_prebuilt connect \
    --server "localhost:$GAME_PORT" \
    --password "$PASSWORD" \
    --name "RogueAI" \
    --controller "$CONTROLLER" \
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
