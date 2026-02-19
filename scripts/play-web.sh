#!/usr/bin/env bash
# play-web.sh - Launch a web GUI game against a native AI opponent
#
# Usage:
#   ./scripts/play-web.sh [OPTIONS] [DECK]
#
# Arguments:
#   DECK        Deck file for the AI opponent (default: decks/white_weenie.dck)
#
# Options:
#   --port PORT         Web server port (default: 8080)
#   --server-port PORT  MTG server port (default: 17771)
#   --controller TYPE   AI controller: random, heuristic, zero (default: heuristic)
#   --help              Show this help
#
# Example:
#   ./scripts/play-web.sh decks/monored.dck
#   ./scripts/play-web.sh --controller random decks/white_weenie.dck
#   make play-web DECK=decks/monored.dck

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

usage() {
    cat <<EOF
play-web.sh - Launch a web GUI game against a native AI opponent

Usage:
  ./scripts/play-web.sh [OPTIONS] [DECK]

Arguments:
  DECK        Deck file for the AI opponent (default: decks/white_weenie.dck)

Options:
  --port PORT         Web server port (default: 8080)
  --server-port PORT  MTG server port (default: 17771)
  --controller TYPE   AI controller: random, heuristic, zero (default: heuristic)
  --help              Show this help

Examples:
  ./scripts/play-web.sh decks/monored.dck
  ./scripts/play-web.sh --controller random decks/white_weenie.dck
  make play-web DECK=decks/monored.dck
EOF
}

# Defaults
DECK="decks/white_weenie.dck"
WEB_PORT=8080
SERVER_PORT=17771
CONTROLLER="heuristic"

# Parse arguments
while [[ $# -gt 0 ]]; do
    case "$1" in
        --port)
            WEB_PORT="$2"; shift 2 ;;
        --server-port)
            SERVER_PORT="$2"; shift 2 ;;
        --controller)
            CONTROLLER="$2"; shift 2 ;;
        --help|-h)
            usage; exit 0 ;;
        -*)
            echo "Unknown option: $1" >&2; exit 1 ;;
        *)
            DECK="$1"; shift ;;
    esac
done

# Resolve deck path
if [[ ! "$DECK" = /* ]]; then
    DECK="${REPO_ROOT}/${DECK}"
fi

MTG_BIN="${REPO_ROOT}/target/release/mtg"

# Pre-flight checks
if [[ ! -f "$MTG_BIN" ]]; then
    echo "Error: release binary not found. Build with: make build-network" >&2
    exit 1
fi

# Check that the binary was built with network support (server/connect subcommands)
if ! "$MTG_BIN" --help 2>&1 | grep -q "server"; then
    echo "Error: release binary does not include network support (missing 'server' command)." >&2
    echo "Rebuild with: make build-network" >&2
    exit 1
fi
if [[ ! -f "$DECK" ]]; then
    echo "Error: deck file not found: $DECK" >&2
    echo "Available decks:" >&2
    ls "${REPO_ROOT}/decks/"*.dck 2>/dev/null | sed 's|.*/||' >&2
    exit 1
fi
if [[ ! -f "${REPO_ROOT}/web/pkg/mtg_forge_rs_bg.wasm" ]]; then
    echo "Error: WASM build not found." >&2
    echo "Build with: make wasm-network" >&2
    exit 1
fi

# Check that the WASM build includes network support (network_init export)
if ! grep -q "network_init" "${REPO_ROOT}/web/pkg/mtg_forge_rs.js" 2>/dev/null; then
    echo "Error: WASM build does not include network support." >&2
    echo "Rebuild with: make wasm-network" >&2
    exit 1
fi

SERVER_LOG="/tmp/mtg-web-server.log"
AI_LOG="/tmp/mtg-web-ai.log"
WEB_LOG="/tmp/mtg-web-httpd.log"

# Clean up all background processes on exit (idempotent - runs once)
_CLEANED_UP=false
cleanup() {
    $_CLEANED_UP && return
    _CLEANED_UP=true
    echo ""
    echo "Shutting down..."
    kill "${SERVER_PID:-}" "${CLIENT_PID:-}" "${WEB_PID:-}" 2>/dev/null || true
    wait "${SERVER_PID:-}" "${CLIENT_PID:-}" "${WEB_PID:-}" 2>/dev/null || true
    echo "Done."
}
trap cleanup EXIT INT TERM

cd "$REPO_ROOT"

# 1. Start the MTG game server
echo "Starting MTG server on port $SERVER_PORT..."
"$MTG_BIN" server --port "$SERVER_PORT" > "$SERVER_LOG" 2>&1 &
SERVER_PID=$!
sleep 1.5  # Wait for server to be ready

# 2. Connect AI opponent with the specified deck
echo "Connecting AI opponent ($CONTROLLER controller, deck: $(basename "$DECK"))..."
"$MTG_BIN" connect "$DECK" \
    --server "localhost:$SERVER_PORT" \
    --controller "$CONTROLLER" \
    --name "AI" \
    > "$AI_LOG" 2>&1 &
CLIENT_PID=$!

# 3. Start the HTTP server for the web GUI
echo "Starting web server on port $WEB_PORT..."
python3 -m http.server "$WEB_PORT" --directory "${REPO_ROOT}/web" \
    > "$WEB_LOG" 2>&1 &
WEB_PID=$!
sleep 0.5  # Wait for web server to be ready

# 4. Print instructions
echo ""
echo "╔══════════════════════════════════════════════════════╗"
echo "║            MTG Web GUI Game Ready!                   ║"
echo "╠══════════════════════════════════════════════════════╣"
echo "║                                                      ║"
echo "║  1. Open your browser and go to:                     ║"
printf "║     \033[1;36mhttp://localhost:%d/fancy.html\033[0m" "$WEB_PORT"
# Pad to fill the box
printf "%*s" $((49 - ${#WEB_PORT} - 19)) "║"
echo ""
echo "║                                                      ║"
echo "║  2. In the 'Server URL' field, enter:                ║"
printf "║     \033[1;33mws://localhost:%d\033[0m" "$SERVER_PORT"
printf "%*s" $((53 - ${#SERVER_PORT} - 16)) "║"
echo ""
echo "║                                                      ║"
echo "║  3. Choose your deck and click Connect!              ║"
echo "║                                                      ║"
echo "╠══════════════════════════════════════════════════════╣"
printf "║  AI opponent: %-38s║\n" "$(basename "$DECK") ($CONTROLLER)"
printf "║  Server log:  %-38s║\n" "$SERVER_LOG"
printf "║  AI log:      %-38s║\n" "$AI_LOG"
echo "╠══════════════════════════════════════════════════════╣"
echo "║  Press Ctrl+C to stop.                               ║"
echo "╚══════════════════════════════════════════════════════╝"
echo ""

# Keep running until any critical process exits
wait "${SERVER_PID}" "${CLIENT_PID}" 2>/dev/null || true
