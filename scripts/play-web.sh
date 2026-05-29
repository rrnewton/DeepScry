#!/usr/bin/env bash
# play-web.sh - Launch a web GUI game (vs AI or two-player PvP)
#
# Usage:
#   ./scripts/play-web.sh [OPTIONS] [DECK]
#
# Arguments:
#   DECK        Deck file for the AI opponent (default: decks/white_weenie.dck)
#               Ignored in --pvp mode.
#
# Options:
#   --port PORT             Web server port (default: 8080)
#   --server-port PORT      MTG server port (default: 17771)
#   --controller TYPE       AI controller: random, heuristic, zero (default: heuristic)
#   --pvp                   Two-player mode: no AI, two browser tabs connect as players
#   --seed N                RNG seed passed to the server (deterministic shuffles)
#   --controller-seed N     RNG seed for the native AI controller
#   --rebuild               Force `make build-network wasm-network` before launching
#                           (otherwise missing builds are an error with a hint)
#   --help                  Show this help
#
# Examples:
#   ./scripts/play-web.sh decks/monored.dck
#   ./scripts/play-web.sh --controller random decks/white_weenie.dck
#   ./scripts/play-web.sh --pvp
#   ./scripts/play-web.sh --seed 42 --controller-seed 43 decks/white_weenie.dck
#   ./scripts/play-web.sh --rebuild
#   make play-web DECK=decks/monored.dck
#   make play-web-pvp

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

usage() {
    cat <<EOF
play-web.sh - Launch a web GUI game (vs AI or two-player PvP)

Usage:
  ./scripts/play-web.sh [OPTIONS] [DECK]

Arguments:
  DECK        Deck file for the AI opponent (default: decks/white_weenie.dck)
              Ignored in --pvp mode.

Options:
  --port PORT             Web server port (default: 8080)
  --server-port PORT      MTG server port (default: 17771)
  --controller TYPE       AI controller: random, heuristic, zero (default: heuristic)
  --pvp                   Two-player mode: no AI, two browser tabs connect as players
  --seed N                RNG seed passed to the server (deterministic shuffles)
  --controller-seed N     RNG seed for the native AI controller
  --rebuild               Force \`make build-network wasm-network\` before launching
                          (otherwise missing builds are an error with a hint)
  --help                  Show this help

Examples:
  ./scripts/play-web.sh decks/monored.dck
  ./scripts/play-web.sh --controller random decks/white_weenie.dck
  ./scripts/play-web.sh --pvp
  ./scripts/play-web.sh --seed 42 --controller-seed 43 decks/white_weenie.dck
  ./scripts/play-web.sh --rebuild
  make play-web DECK=decks/monored.dck
  make play-web-pvp
EOF
}

# Defaults
DECK="decks/white_weenie.dck"
WEB_PORT=8080
SERVER_PORT=17771
CONTROLLER="heuristic"
PVP_MODE=false
SEED=""
CONTROLLER_SEED=""
FORCE_REBUILD=false

# Parse arguments
while [[ $# -gt 0 ]]; do
    case "$1" in
        --port)
            WEB_PORT="$2"; shift 2 ;;
        --server-port)
            SERVER_PORT="$2"; shift 2 ;;
        --controller)
            CONTROLLER="$2"; shift 2 ;;
        --pvp|--no-ai)
            PVP_MODE=true; shift ;;
        --seed)
            SEED="$2"; shift 2 ;;
        --controller-seed)
            CONTROLLER_SEED="$2"; shift 2 ;;
        --rebuild)
            FORCE_REBUILD=true; shift ;;
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

# Optional forced rebuild before preflight (--rebuild)
if $FORCE_REBUILD; then
    echo "Rebuilding native binary (network feature) and WASM bundle..."
    (cd "$REPO_ROOT" && make build-network wasm-network)
fi

# Pre-flight checks
if [[ ! -f "$MTG_BIN" ]]; then
    echo "Error: release binary not found. Build with: make build-network" >&2
    echo "       (or rerun this script with --rebuild)" >&2
    exit 1
fi

# Check that the binary was built with network support (server/connect subcommands)
if ! "$MTG_BIN" --help 2>&1 | grep -q "server"; then
    echo "Error: release binary does not include network support (missing 'server' command)." >&2
    echo "Rebuild with: make build-network" >&2
    echo "       (or rerun this script with --rebuild)" >&2
    exit 1
fi
if ! $PVP_MODE; then
    if [[ ! -f "$DECK" ]]; then
        echo "Error: deck file not found: $DECK" >&2
        echo "Available decks:" >&2
        ls "${REPO_ROOT}/decks/"*.dck 2>/dev/null | sed 's|.*/||' >&2
        exit 1
    fi
fi
if [[ ! -f "${REPO_ROOT}/web/pkg/mtg_engine_bg.wasm" ]]; then
    echo "Error: WASM build not found." >&2
    echo "Build with: make wasm-network" >&2
    echo "       (or rerun this script with --rebuild)" >&2
    exit 1
fi

# Check that the WASM build includes network support (network_init export)
if ! grep -q "network_init" "${REPO_ROOT}/web/pkg/mtg_engine.js" 2>/dev/null; then
    echo "Error: WASM build does not include network support." >&2
    echo "Rebuild with: make wasm-network" >&2
    echo "       (or rerun this script with --rebuild)" >&2
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

# Optional seed args (only forwarded when the user passed --seed / --controller-seed)
SERVER_SEED_ARGS=()
if [[ -n "$SEED" ]]; then
    SERVER_SEED_ARGS=(--seed "$SEED")
fi
CLIENT_SEED_ARGS=()
if [[ -n "$CONTROLLER_SEED" ]]; then
    CLIENT_SEED_ARGS=(--seed-player "$CONTROLLER_SEED")
fi

# 1. Start the MTG game server (loop mode: accepts new games after each one ends)
if [[ -n "$SEED" ]]; then
    echo "Starting MTG server on port $SERVER_PORT (loop mode, seed=$SEED)..."
else
    echo "Starting MTG server on port $SERVER_PORT (loop mode)..."
fi
"$MTG_BIN" server --port "$SERVER_PORT" --network-debug --loop "${SERVER_SEED_ARGS[@]}" \
    > "$SERVER_LOG" 2>&1 &
SERVER_PID=$!
sleep 1.5  # Wait for server to be ready

# 2. Connect AI opponent (skip in PvP mode)
if $PVP_MODE; then
    echo "PvP mode: waiting for two browser clients to connect..."
else
    if [[ -n "$CONTROLLER_SEED" ]]; then
        echo "Connecting AI opponent ($CONTROLLER controller, deck: $(basename "$DECK"), seed=$CONTROLLER_SEED, reconnect mode)..."
    else
        echo "Connecting AI opponent ($CONTROLLER controller, deck: $(basename "$DECK"), reconnect mode)..."
    fi
    "$MTG_BIN" connect "$DECK" \
        --server "localhost:$SERVER_PORT" \
        --controller "$CONTROLLER" \
        --name "AI" \
        --reconnect \
        "${CLIENT_SEED_ARGS[@]}" \
        > "$AI_LOG" 2>&1 &
    CLIENT_PID=$!
fi

# 3. Start the HTTP server for the web GUI
echo "Starting web server on port $WEB_PORT..."
python3 -m http.server "$WEB_PORT" --directory "${REPO_ROOT}/web" \
    > "$WEB_LOG" 2>&1 &
WEB_PID=$!
sleep 0.5  # Wait for web server to be ready

# 4. Print instructions
URL="http://localhost:${WEB_PORT}/tui_game.html"
WS_URL="ws://localhost:${SERVER_PORT}"

echo ""
if $PVP_MODE; then
    echo "╔══════════════════════════════════════════════════════╗"
    echo "║         MTG Web PvP Game Ready!                      ║"
    echo "╠══════════════════════════════════════════════════════╣"
    echo "║                                                      ║"
    echo "║  Open TWO browser tabs and go to:                    ║"
    printf "║     \033[1;36m%s\033[0m" "$URL"
    printf "%*s║\n" $((53 - ${#URL})) ""
    echo "║                                                      ║"
    echo "║  In each tab, enter the server URL:                  ║"
    printf "║     \033[1;33m%s\033[0m" "$WS_URL"
    printf "%*s║\n" $((53 - ${#WS_URL})) ""
    echo "║                                                      ║"
    echo "║  Each player chooses a deck and clicks Connect.      ║"
    echo "║  The game starts when both players have connected.   ║"
    echo "║                                                      ║"
    echo "╠══════════════════════════════════════════════════════╣"
    printf "║  Server log:  %-38s║\n" "$SERVER_LOG"
    echo "╠══════════════════════════════════════════════════════╣"
    echo "║  Press Ctrl+C to stop.                               ║"
    echo "╚══════════════════════════════════════════════════════╝"
else
    echo "╔══════════════════════════════════════════════════════╗"
    echo "║            MTG Web GUI Game Ready!                   ║"
    echo "╠══════════════════════════════════════════════════════╣"
    echo "║                                                      ║"
    echo "║  1. Open your browser and go to:                     ║"
    printf "║     \033[1;36m%s\033[0m" "$URL"
    printf "%*s║\n" $((53 - ${#URL})) ""
    echo "║                                                      ║"
    echo "║  2. In the 'Server URL' field, enter:                ║"
    printf "║     \033[1;33m%s\033[0m" "$WS_URL"
    printf "%*s║\n" $((53 - ${#WS_URL})) ""
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
fi
echo ""

# Keep running until server exits
wait "${SERVER_PID}" 2>/dev/null || true
