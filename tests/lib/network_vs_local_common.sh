#!/usr/bin/env bash
# Shared logic for Network vs Local game equivalence testing
#
# This library provides the core test logic that can be called from
# wrapper scripts with different deck configurations.
#
# Usage:
#   source "$SCRIPT_DIR/lib/network_vs_local_common.sh"
#   run_network_vs_local_test <options>
#
# Options (all have defaults):
#   --deck1 PATH        First player's deck (required)
#   --deck2 PATH        Second player's deck (required)
#   --seed N            Game seed (default: 3)
#   --controller-seed N Controller seed (default: 3)
#   --controller TYPE   Controller type: heuristic, zero, random (default: heuristic)
#   --p1-name NAME      Player 1 name (default: Player1)
#   --p2-name NAME      Player 2 name (default: Player2)
#   --timeout N         Timeout in seconds (default: 120)
#   --output-dir PATH   Output directory (default: /tmp/network_vs_local_$$)
#   --skip-gamelog-check  Skip strict gamelog comparison (for debugging)
#
# Example:
#   run_network_vs_local_test \
#     --deck1 "$WORKSPACE_ROOT/decks/test/tutor_test.dck" \
#     --deck2 "$WORKSPACE_ROOT/decks/test/tutor_test.dck" \
#     --seed 42 \
#     --controller heuristic

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Main test function - runs both local and network games and compares results
run_network_vs_local_test() {
    # Parse arguments
    local DECK1=""
    local DECK2=""
    local SEED=3
    local CONTROLLER_SEED=3
    local CONTROLLER_TYPE="heuristic"
    local P1_NAME="Player1"
    local P2_NAME="Player2"
    local TIMEOUT=120
    local OUTPUT_DIR=""
    local SKIP_GAMELOG_CHECK=0

    while [[ $# -gt 0 ]]; do
        case $1 in
            --deck1)
                DECK1="$2"
                shift 2
                ;;
            --deck2)
                DECK2="$2"
                shift 2
                ;;
            --seed)
                SEED="$2"
                shift 2
                ;;
            --controller-seed)
                CONTROLLER_SEED="$2"
                shift 2
                ;;
            --controller)
                CONTROLLER_TYPE="$2"
                shift 2
                ;;
            --p1-name)
                P1_NAME="$2"
                shift 2
                ;;
            --p2-name)
                P2_NAME="$2"
                shift 2
                ;;
            --timeout)
                TIMEOUT="$2"
                shift 2
                ;;
            --output-dir)
                OUTPUT_DIR="$2"
                shift 2
                ;;
            --skip-gamelog-check)
                SKIP_GAMELOG_CHECK=1
                shift
                ;;
            *)
                echo -e "${RED}Unknown option: $1${NC}"
                return 1
                ;;
        esac
    done

    # Validate required arguments
    if [[ -z "$DECK1" ]]; then
        echo -e "${RED}Error: --deck1 is required${NC}"
        return 1
    fi
    if [[ -z "$DECK2" ]]; then
        echo -e "${RED}Error: --deck2 is required${NC}"
        return 1
    fi
    if [[ ! -f "$DECK1" ]]; then
        echo -e "${RED}Error: Deck not found: $DECK1${NC}"
        return 1
    fi
    if [[ ! -f "$DECK2" ]]; then
        echo -e "${RED}Error: Deck not found: $DECK2${NC}"
        return 1
    fi

    # Set default output directory
    if [[ -z "$OUTPUT_DIR" ]]; then
        OUTPUT_DIR="/tmp/network_vs_local_$$"
    fi

    # Export for cleanup function (nested function closure doesn't persist after function return)
    export _NVL_OUTPUT_DIR="$OUTPUT_DIR"

    mkdir -p "$OUTPUT_DIR"
    local LOCAL_OUTPUT="$OUTPUT_DIR/local"
    local NETWORK_OUTPUT="$OUTPUT_DIR/network"
    mkdir -p "$LOCAL_OUTPUT" "$NETWORK_OUTPUT"

    # Setup cleanup trap (uses exported var since local OUTPUT_DIR won't persist)
    _nvl_cleanup() {
        echo
        echo "Cleaning up..."
        jobs -p 2>/dev/null | xargs -r kill 2>/dev/null || true
        wait 2>/dev/null || true
        echo "Logs preserved at ${_NVL_OUTPUT_DIR:-/tmp/unknown}"
    }
    trap _nvl_cleanup EXIT

    echo "=== Network vs Local Game Equivalence Test ==="
    echo
    echo "Configuration:"
    echo "  Seed: $SEED"
    echo "  Controller: $CONTROLLER_TYPE (seed: $CONTROLLER_SEED)"
    echo "  Deck 1: $(basename "$DECK1")"
    echo "  Deck 2: $(basename "$DECK2")"
    echo "  Output: $OUTPUT_DIR"
    echo

    # ============================================================================
    # Start LOCAL game (single process)
    # ============================================================================
    echo -e "${BLUE}Starting LOCAL game...${NC}"

    "$MTG_BIN" tui \
        "$DECK1" \
        "$DECK2" \
        --p1 "$CONTROLLER_TYPE" \
        --p2 "$CONTROLLER_TYPE" \
        --p1-name "$P1_NAME" \
        --p2-name "$P2_NAME" \
        --seed "$SEED" \
        --seed-p1 "$CONTROLLER_SEED" \
        --seed-p2 "$CONTROLLER_SEED" \
        --tag-gamelogs \
        --verbosity normal \
        > "$LOCAL_OUTPUT/game.log" 2>&1 &
    local LOCAL_PID=$!
    echo "  Local PID: $LOCAL_PID"

    # ============================================================================
    # Start NETWORK game (server + 2 clients)
    # ============================================================================
    echo -e "${BLUE}Starting NETWORK game...${NC}"

    # Find an available port
    local PORT=17780
    while lsof -i:$PORT >/dev/null 2>&1; do
        PORT=$((PORT + 1))
    done

    # Start server with --network-debug for strict reveal validation
    "$MTG_BIN" server \
        --port "$PORT" \
        --seed "$SEED" \
        --tag-gamelogs \
        --network-debug \
        --verbosity normal \
        --no-color-logs \
        > "$NETWORK_OUTPUT/server.log" 2>&1 &
    local SERVER_PID=$!
    echo "  Server PID: $SERVER_PID (port $PORT)"

    # Wait for server to start
    sleep 2

    if ! kill -0 $SERVER_PID 2>/dev/null; then
        echo -e "${RED}Error: Server failed to start${NC}"
        cat "$NETWORK_OUTPUT/server.log"
        return 1
    fi

    # Start client 1
    "$MTG_BIN" connect \
        "$DECK1" \
        --server "localhost:$PORT" \
        --controller "$CONTROLLER_TYPE" \
        --seed-player "$CONTROLLER_SEED" \
        --name "$P1_NAME" \
        --tag-gamelogs \
        --gamelog-output "$NETWORK_OUTPUT/client1_gamelog.txt" \
        > "$NETWORK_OUTPUT/client1.log" 2>&1 &
    local CLIENT1_PID=$!
    echo "  Client 1 PID: $CLIENT1_PID ($P1_NAME - $CONTROLLER_TYPE)"

    sleep 1

    # Start client 2
    "$MTG_BIN" connect \
        "$DECK2" \
        --server "localhost:$PORT" \
        --controller "$CONTROLLER_TYPE" \
        --seed-player "$CONTROLLER_SEED" \
        --name "$P2_NAME" \
        --tag-gamelogs \
        --gamelog-output "$NETWORK_OUTPUT/client2_gamelog.txt" \
        > "$NETWORK_OUTPUT/client2.log" 2>&1 &
    local CLIENT2_PID=$!
    echo "  Client 2 PID: $CLIENT2_PID ($P2_NAME - $CONTROLLER_TYPE)"

    echo
    echo "Both games running in parallel. Waiting for completion..."
    echo

    # ============================================================================
    # Wait for both games to complete (with timeout)
    # ============================================================================
    local ELAPSED=0
    local LOCAL_DONE=0
    local NETWORK_DONE=0
    local LOCAL_EXIT=0
    local SERVER_EXIT=0

    while [ $ELAPSED -lt $TIMEOUT ]; do
        # Check local game
        if [ $LOCAL_DONE -eq 0 ] && ! kill -0 $LOCAL_PID 2>/dev/null; then
            wait $LOCAL_PID 2>/dev/null
            LOCAL_EXIT=$?
            LOCAL_DONE=1
            echo -e "  ${GREEN}Local game finished (exit $LOCAL_EXIT)${NC}"
        fi

        # Check network game (server exit means game over)
        if [ $NETWORK_DONE -eq 0 ] && ! kill -0 $SERVER_PID 2>/dev/null; then
            wait $SERVER_PID 2>/dev/null
            SERVER_EXIT=$?
            # Also wait for clients
            wait $CLIENT1_PID 2>/dev/null || true
            wait $CLIENT2_PID 2>/dev/null || true
            NETWORK_DONE=1
            echo -e "  ${GREEN}Network game finished (server exit $SERVER_EXIT)${NC}"
        fi

        # Both done?
        if [ $LOCAL_DONE -eq 1 ] && [ $NETWORK_DONE -eq 1 ]; then
            break
        fi

        sleep 1
        ELAPSED=$((ELAPSED + 1))
    done

    # Check for timeout
    if [ $LOCAL_DONE -eq 0 ]; then
        echo -e "${RED}Error: Local game timed out after ${TIMEOUT}s${NC}"
        kill $LOCAL_PID 2>/dev/null || true
        return 1
    fi

    if [ $NETWORK_DONE -eq 0 ]; then
        echo -e "${RED}Error: Network game timed out after ${TIMEOUT}s${NC}"
        kill $SERVER_PID $CLIENT1_PID $CLIENT2_PID 2>/dev/null || true
        return 1
    fi

    echo
    echo "=== Analyzing Results ==="
    echo

    # ============================================================================
    # Extract and compare results
    # ============================================================================

    # Extract turns played from local game
    local LOCAL_TURNS=$(grep -o "Turns played: [0-9]*" "$LOCAL_OUTPUT/game.log" | grep -o "[0-9]*" || echo "?")

    # Extract action_count from network game
    local NETWORK_ACTION_COUNT=$(grep -o "action_count: [0-9]*" "$NETWORK_OUTPUT/client1.log" | tail -1 | grep -o "[0-9]*" || echo "?")

    # Extract max turns from GAMELOG entries
    local LOCAL_MAX_TURN=$(grep -o '\[GAMELOG Turn[0-9]*' "$LOCAL_OUTPUT/game.log" | grep -o '[0-9]*' | sort -n | tail -1 || echo "?")
    local NETWORK_MAX_TURN=$(grep -o '\[GAMELOG Turn[0-9]*' "$NETWORK_OUTPUT/client1.log" | grep -o '[0-9]*' | sort -n | tail -1 || echo "?")

    echo "Turns/Action counts:"
    echo "  Local turns:  $LOCAL_TURNS (max turn in gamelog: $LOCAL_MAX_TURN)"
    echo "  Network:      action_count=$NETWORK_ACTION_COUNT (max turn in gamelog: $NETWORK_MAX_TURN)"

    # Extract winners
    local LOCAL_WINNER=$(grep -o "Winner: [A-Za-z0-9_-]*" "$LOCAL_OUTPUT/game.log" | head -1 | sed 's/Winner: //' || echo "?")
    local NETWORK_WINNER=$(grep -o "winner=Some([0-9])" "$NETWORK_OUTPUT/client1.log" | tail -1 | grep -o "[0-9]" || echo "?")

    echo
    echo "Winners:"
    echo "  Local:   $LOCAL_WINNER"
    echo "  Network: $NETWORK_WINNER"

    # Extract and compare GAMELOG entries
    echo
    echo "GAMELOG comparison:"

    local LOCAL_GAMELOG="$OUTPUT_DIR/local_gamelog.txt"
    local NETWORK_GAMELOG="$OUTPUT_DIR/network_gamelog.txt"

    # Extract GAMELOG entries from LOCAL (excluding noise)
    grep '^\s*\[GAMELOG' "$LOCAL_OUTPUT/game.log" 2>/dev/null | \
        grep -v 'Tap.*for {' | \
        grep -v 'resolves$' | \
        grep -v 'takes.*damage.*life:' | \
        grep -v 'deals.*damage.*life:' \
        > "$LOCAL_GAMELOG" || true

    # Extract SERVER gamelogs (authoritative)
    grep '\[GAMELOG' "$NETWORK_OUTPUT/server.log" 2>/dev/null | \
        grep -v 'Tap.*for {' | \
        grep -v 'resolves$' | \
        grep -v 'takes.*damage.*life:' | \
        grep -v 'deals.*damage.*life:' \
        > "$NETWORK_GAMELOG" || true

    local LOCAL_GAMELOG_COUNT=$(wc -l < "$LOCAL_GAMELOG" 2>/dev/null || echo "0")
    local NETWORK_GAMELOG_COUNT=$(wc -l < "$NETWORK_GAMELOG" 2>/dev/null || echo "0")

    echo "  Local GAMELOG entries:   $LOCAL_GAMELOG_COUNT"
    echo "  Server GAMELOG entries:  $NETWORK_GAMELOG_COUNT"

    # ============================================================================
    # Verify results
    # ============================================================================
    echo
    echo "=== Verification ==="
    local EXIT_CODE=0

    # Check both games completed (have max turn data)
    if [ "$LOCAL_MAX_TURN" != "?" ] && [ "$NETWORK_MAX_TURN" != "?" ]; then
        echo -e "${GREEN}✓ Both games completed (local: $LOCAL_MAX_TURN turns, network: $NETWORK_MAX_TURN turns)${NC}"
    else
        echo -e "${RED}✗ One or both games did not complete (local: $LOCAL_MAX_TURN, network: $NETWORK_MAX_TURN)${NC}"
        EXIT_CODE=1
    fi

    # Check both games have winners
    if [ "$LOCAL_WINNER" != "?" ] && [ "$NETWORK_WINNER" != "?" ]; then
        echo -e "${GREEN}✓ Both games have winners (local: $LOCAL_WINNER, network player: $NETWORK_WINNER)${NC}"
    else
        echo -e "${RED}✗ Could not determine winner for one or both games${NC}"
        EXIT_CODE=1
    fi

    # GAMELOG comparison (strict unless --skip-gamelog-check)
    if [ $SKIP_GAMELOG_CHECK -eq 1 ]; then
        echo -e "${YELLOW}⚠ Skipping strict gamelog comparison (--skip-gamelog-check)${NC}"
    elif [ "$LOCAL_GAMELOG_COUNT" -gt 0 ] && [ "$NETWORK_GAMELOG_COUNT" -gt 0 ]; then
        echo "Both games produced GAMELOG entries (local: $LOCAL_GAMELOG_COUNT, server: $NETWORK_GAMELOG_COUNT)"

        local DIFF_OUTPUT=$(diff "$LOCAL_GAMELOG" "$NETWORK_GAMELOG" 2>/dev/null || true)
        local DIFF_COUNT=0
        if [ -n "$DIFF_OUTPUT" ]; then
            DIFF_COUNT=$(echo "$DIFF_OUTPUT" | grep -c '^[<>]' 2>/dev/null || echo "0")
        fi

        if [ "$DIFF_COUNT" -eq 0 ]; then
            echo -e "${GREEN}✓ LOCAL and SERVER gamelogs are IDENTICAL${NC}"
        else
            echo -e "${RED}✗ LOCAL and SERVER gamelogs differ by $DIFF_COUNT lines${NC}"
            echo "  First differences:"
            echo "$DIFF_OUTPUT" | head -20
            EXIT_CODE=1
        fi
    elif [ "$LOCAL_GAMELOG_COUNT" -gt 0 ]; then
        echo -e "${RED}✗ Only local game produced GAMELOG entries ($LOCAL_GAMELOG_COUNT entries)${NC}"
        EXIT_CODE=1
    elif [ "$NETWORK_GAMELOG_COUNT" -gt 0 ]; then
        echo -e "${RED}✗ Only network game produced GAMELOG entries ($NETWORK_GAMELOG_COUNT entries)${NC}"
        EXIT_CODE=1
    else
        echo -e "${RED}✗ Neither game produced GAMELOG entries${NC}"
        EXIT_CODE=1
    fi

    # Check for errors in logs
    local ERRORS=""
    if grep -qE "^thread.*panicked|RUST_BACKTRACE|panicked at|fatal error" "$LOCAL_OUTPUT/game.log" 2>/dev/null; then
        ERRORS="$ERRORS local"
    fi
    if grep -qE "^thread.*panicked|RUST_BACKTRACE|panicked at|fatal error" "$NETWORK_OUTPUT/server.log" "$NETWORK_OUTPUT/client1.log" "$NETWORK_OUTPUT/client2.log" 2>/dev/null; then
        ERRORS="$ERRORS network"
    fi

    if [ -n "$ERRORS" ]; then
        echo -e "${YELLOW}⚠ Potential errors found in:$ERRORS${NC}"
    else
        echo -e "${GREEN}✓ No errors detected in logs${NC}"
    fi

    echo
    echo "=== Test Complete ==="
    echo "Full logs available at: $OUTPUT_DIR"

    return $EXIT_CODE
}
