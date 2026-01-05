#!/bin/bash
# Network test: heuristic AI vs random AI
# Reproduces desync issue when running AI controllers over network
#
# USAGE:
#   ./debug/network_heuristic_vs_random.sh
#
# EXPECTED BEHAVIOR (currently broken):
#   - Server should stream output to terminal with SYNC OK messages
#   - Game should complete with one player winning or a draw
#   - All processes should exit cleanly with code 0
#
# CURRENT BUG:
#   - Around action_count=259 (Turn 9), server hits "FATAL SYNC ERROR"
#   - Server hangs instead of exiting
#   - Clients timeout and print bogus "Game ended in a draw"
#   - Clients exit with code 0 instead of error code
#
# LOGS: Written to /tmp/mtg_heur_vs_rand_$$/

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_ROOT"

OUTPUT_DIR="/tmp/mtg_heur_vs_rand_$$"
mkdir -p "$OUTPUT_DIR"

cleanup() {
    echo ""
    echo "Cleaning up..."
    jobs -p | xargs -r kill 2>/dev/null || true
    wait 2>/dev/null || true
    echo "Logs preserved at $OUTPUT_DIR"
}
trap cleanup EXIT

echo "Building release with network feature..."
cargo build --release --features network 2>&1 | tail -5

echo "Output directory: $OUTPUT_DIR"
echo ""

# Start server with --network-debug flag, streaming to both terminal and log
echo "Starting server (output streams below)..."
echo "========================================"
RUST_LOG=debug ./target/release/mtg server --network-debug --seed=999 2>&1 | tee "$OUTPUT_DIR/server.log" &
SERVER_PID=$!
echo "Server PID: $SERVER_PID (backgrounded, output above)"
sleep 2

if ! kill -0 $SERVER_PID 2>/dev/null; then
    echo "ERROR: Server failed to start"
    exit 1
fi

# Start heuristic client (no stdin needed - AI makes all decisions)
echo ""
echo "Starting client 1 (Ryan - heuristic)..."
timeout 120 bash -c 'RUST_LOG=info ./target/release/mtg connect --controller heuristic -n Ryan decks/booster_draft/avatar/ryan_avatar_draft.dck 2>&1' > "$OUTPUT_DIR/heuristic.log" &
P1_PID=$!
echo "Heuristic PID: $P1_PID"

sleep 1

# Start random client (no stdin needed - AI makes all decisions)
echo "Starting client 2 (Gabriel - random)..."
timeout 120 bash -c 'RUST_LOG=info ./target/release/mtg connect --controller random -n Gabriel decks/booster_draft/avatar/gabriel_avatar_draft.dck 2>&1' > "$OUTPUT_DIR/random.log" &
P2_PID=$!
echo "Random PID: $P2_PID"

echo ""
echo "Waiting for game to complete (server output streaming above)..."
echo ""

# Wait for clients to finish
wait $P1_PID 2>/dev/null
P1_EXIT=$?
wait $P2_PID 2>/dev/null
P2_EXIT=$?

echo ""
echo "========================================"
echo "=== RESULTS ==="
echo ""

echo "Exit codes: heuristic=$P1_EXIT, random=$P2_EXIT"

echo ""
echo "=== Heuristic client (last 30 lines) ==="
tail -30 "$OUTPUT_DIR/heuristic.log" 2>/dev/null || echo "(no output)"

echo ""
echo "=== Random client (last 30 lines) ==="
tail -30 "$OUTPUT_DIR/random.log" 2>/dev/null || echo "(no output)"

echo ""
echo "=== Checking for errors ==="
ERRORS=$(grep -h "ERROR\|FATAL\|Connection error" "$OUTPUT_DIR"/*.log 2>/dev/null | head -20 || true)
if [ -n "$ERRORS" ]; then
    echo "Errors found:"
    echo "$ERRORS"
else
    echo "No errors found!"
fi

echo ""
echo "=== Checking for game end ==="
GAME_END=$(grep -h "Game ended\|wins the game\|Winner" "$OUTPUT_DIR"/*.log 2>/dev/null | head -5 || true)
if [ -n "$GAME_END" ]; then
    echo "Game end messages:"
    echo "$GAME_END"
else
    echo "No game end messages found"
fi

echo ""
echo "=== Checking for SYNC status ==="
SYNC_FAIL=$(grep -h "FATAL SYNC ERROR\|SYNC FAILED" "$OUTPUT_DIR"/*.log 2>/dev/null | head -5 || true)
if [ -n "$SYNC_FAIL" ]; then
    echo "SYNC FAILURES:"
    echo "$SYNC_FAIL"
fi
SYNCS=$(grep -h "SYNC OK" "$OUTPUT_DIR"/*.log 2>/dev/null | tail -5 || true)
if [ -n "$SYNCS" ]; then
    echo "Last successful syncs:"
    echo "$SYNCS"
else
    echo "No SYNC OK messages found"
fi

echo ""
echo "Done."
