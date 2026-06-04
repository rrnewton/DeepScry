#!/usr/bin/env bash
# E2E test: Network vs Local game equivalence
#
# This test validates that network and local games produce identical results when
# using the same seed, decks, and deterministic controllers.
#
# HISTORY: This test was previously disabled (mtg-252) due to library search
# state divergence in network mode. The issue was that clients didn't know which
# specific CardId was chosen by the server for library searches (like typecycling).
# This was fixed by adding library_search_result to the ChoiceAccepted message,
# allowing the client's shadow game to stay synchronized with the server.
#
# Usage: ./network_vs_local_equivalence_e2e.sh [SEED] [CONTROLLER_P1] [CONTROLLER_P2]
#
# Arguments (all optional):
#   SEED          - Game seed (default: 3)
#   CONTROLLER_P1 - Controller type for player 1: heuristic, random, zero (default: heuristic)
#   CONTROLLER_P2 - Controller type for player 2 (default: same as P1)
#
# Examples:
#   ./network_vs_local_equivalence_e2e.sh              # seed=3, both heuristic
#   ./network_vs_local_equivalence_e2e.sh 5            # seed=5, both heuristic
#   ./network_vs_local_equivalence_e2e.sh 5 random     # seed=5, both random
#   ./network_vs_local_equivalence_e2e.sh 5 random heuristic  # seed=5, p1=random, p2=heuristic
#
# This test runs the SAME game in two modes in PARALLEL:
# 1. Local mode: Single process with two AIs
# 2. Network mode: Server + two client processes with AIs
#
# Both use identical seeds, decks, and controller settings. The test verifies:
# - Both games complete successfully
# - Final action_count matches between network and local
# - GAMELOG entries match (deterministic gameplay)
#
# This test uses pre-built binaries and runs both games in parallel to minimize
# impact on validation time.

set -euo pipefail

# Get script directory and source shared test helpers
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"
# Shared gamelog filter — single source of truth (no inline sed/grep copies).
# shellcheck source=../bug_finding/lib/gamelog_filter.sh
source "$WORKSPACE_ROOT/bug_finding/lib/gamelog_filter.sh"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

echo "=== Network vs Local Game Equivalence E2E Test ==="
echo

# Use pre-built binary if available, otherwise build
if [ -f "$WORKSPACE_ROOT/target/release/mtg" ]; then
    export MTG_BIN="$WORKSPACE_ROOT/target/release/mtg"
    echo "Using pre-built binary: $MTG_BIN"
else
    echo "Building release binary..."
    ensure_mtg_binary
fi

# Verify binary has network feature
if ! "$MTG_BIN" server --help >/dev/null 2>&1; then
    echo -e "${YELLOW}Warning: Binary doesn't support network mode, rebuilding...${NC}"
    ensure_mtg_binary
fi

cd "$WORKSPACE_ROOT"

# Check for required files
if [[ ! -d "$WORKSPACE_ROOT/cardsfolder" ]]; then
    echo -e "${YELLOW}Warning: cardsfolder not found, skipping test${NC}"
    exit 0
fi

# Use the avatar draft decks (same as debug/network_heuristic_vs_random.sh)
DECK1="$WORKSPACE_ROOT/decks/booster_draft/avatar/ryan_avatar_draft.dck"
DECK2="$WORKSPACE_ROOT/decks/booster_draft/avatar/gabriel_avatar_draft.dck"

if [[ ! -f "$DECK1" ]]; then
    echo -e "${RED}Error: $DECK1 not found${NC}"
    exit 1
fi

if [[ ! -f "$DECK2" ]]; then
    echo -e "${RED}Error: $DECK2 not found${NC}"
    exit 1
fi

# Parse arguments with defaults
# ALL controllers (heuristic, random, zero) MUST produce identical gamelogs
# between LOCAL and NETWORK modes. Controllers must NEVER depend on hidden
# information (opponent hand contents, library order) - any divergence between
# local and network is a bug. See docs/NETWORK_ARCHITECTURE.md.
SEED="${1:-3}"
CONTROLLER_P1="${2:-zero}"
CONTROLLER_P2="${3:-$CONTROLLER_P1}"  # Default P2 to same as P1
CONTROLLER_SEED=3

# Compute per-player controller seeds via the SAME derivation that production
# code uses (`game::seed_derivation::derive_player_seed`):
#   P1_SEED = master + 0x1234_5678_9ABC_DEF0  (P1_SALT)
#   P2_SEED = master + 0xFEDC_BA98_7654_3210  (P2_SALT)
# wrapped at 2^64. Network clients invoke `--seed-player=$CONTROLLER_SEED` and
# the client derives per-slot internally (see main.rs / ControllerType::Random
# in the `connect` command). Local TUI is given `--seed-p1`/`--seed-p2` which
# bypass derivation, so we MUST pre-derive here to match the network stream.
# Without this match, the LOCAL RandomController RNG diverges from the NETWORK
# RandomController RNG and the gamelog comparison fails — see mtg-458.
#
# bash arithmetic is signed 64-bit; the SALTs wrap into signed-negative values
# whose unsigned (u64) reinterpretation is what Rust's seed parser accepts.
# `printf '%u'` performs the signed→unsigned 64-bit reinterpretation we need
# (it converts negative bash integers back into their u64 bit-pattern as a
# decimal string), giving the exact value `derive_player_seed` produces.
# Seed salts come from the shared bug_finding/lib/seed_salts.sh — the ONE bash
# mirror of mtg-engine/src/game/seed_derivation.rs. No hand-copied hex here.
source "$WORKSPACE_ROOT/bug_finding/lib/seed_salts.sh"
P1_DERIVED_SEED="$(derive_p1_seed "$CONTROLLER_SEED")"
P2_DERIVED_SEED="$(derive_p2_seed "$CONTROLLER_SEED")"

# Validate controller types
for ctrl in "$CONTROLLER_P1" "$CONTROLLER_P2"; do
    if [[ "$ctrl" != "heuristic" && "$ctrl" != "random" && "$ctrl" != "zero" ]]; then
        echo -e "${RED}Error: Invalid controller type '$ctrl'. Must be: heuristic, random, or zero${NC}"
        exit 1
    fi
done

# Controller type notes:
# ALL controller types must produce identical gamelogs between local and network modes.
# Controllers must NEVER use hidden information (opponent hand, library order, RNG).
# If a controller produces different results on server (full state) vs client (shadow
# state), that controller has an information-leakage bug. See NETWORK_ARCHITECTURE.md.

# Output directories
OUTPUT_DIR="/tmp/network_vs_local_e2e_$$"
mkdir -p "$OUTPUT_DIR"

LOCAL_OUTPUT="$OUTPUT_DIR/local"
NETWORK_OUTPUT="$OUTPUT_DIR/network"
mkdir -p "$LOCAL_OUTPUT" "$NETWORK_OUTPUT"

# Cleanup function
cleanup() {
    echo
    echo "Cleaning up..."
    # Kill any background processes
    jobs -p 2>/dev/null | xargs -r kill 2>/dev/null || true
    wait 2>/dev/null || true
    echo "Logs preserved at $OUTPUT_DIR"
}
trap cleanup EXIT

echo "Configuration:"
echo "  Seed: $SEED"
echo "  Controller P1: $CONTROLLER_P1"
echo "  Controller P2: $CONTROLLER_P2"
echo "  Deck 1: $(basename "$DECK1")"
echo "  Deck 2: $(basename "$DECK2")"
echo "  Output: $OUTPUT_DIR"
echo

# Player names - must match between LOCAL and NETWORK for gamelog comparison
P1_NAME="Ryan"
P2_NAME="Gabriel"

# ============================================================================
# Start LOCAL game (single process)
# ============================================================================
echo -e "${BLUE}Starting LOCAL game...${NC}"

"$MTG_BIN" tui \
    "$DECK1" \
    "$DECK2" \
    --p1 "$CONTROLLER_P1" \
    --p2 "$CONTROLLER_P2" \
    --p1-name "$P1_NAME" \
    --p2-name "$P2_NAME" \
    --seed "$SEED" \
    --seed-p1 "$P1_DERIVED_SEED" \
    --seed-p2 "$P2_DERIVED_SEED" \
    --tag-gamelogs \
    --verbosity normal \
    > "$LOCAL_OUTPUT/game.log" 2>&1 &
LOCAL_PID=$!
echo "  Local PID: $LOCAL_PID"

# ============================================================================
# Start NETWORK game (server + 2 clients)
# ============================================================================
echo -e "${BLUE}Starting NETWORK game...${NC}"

# Bind a kernel-assigned EPHEMERAL port (--port 0) instead of a random fixed
# port. RANDOM % 10000 collided across concurrent test runs / leftover servers
# (mtg-ibj22 / mtg-726 port-collision false-positives); --port 0 is atomic with
# NO TOCTOU — the OS picks a free port at bind time and the server logs the
# ACTUAL bound port ("listening on HOST:PORT"), which we parse below.
# Start server with --network-debug for strict reveal validation
# Use verbosity=normal to capture GAMELOG entries (minimal suppresses them)
# Use --no-color-logs to avoid ANSI codes in output
"$MTG_BIN" server \
    --port 0 \
    --seed "$SEED" \
    --tag-gamelogs \
    --network-debug \
    --verbosity normal \
    --no-color-logs \
    > "$NETWORK_OUTPUT/server.log" 2>&1 &
SERVER_PID=$!

# Wait for the server to announce its bound port, then parse it (replaces a
# fixed `sleep 2` — both more robust AND how we learn the ephemeral port).
PORT=""
for _ in $(seq 1 50); do
    if ! kill -0 $SERVER_PID 2>/dev/null; then
        echo -e "${RED}Error: Server failed to start${NC}"
        cat "$NETWORK_OUTPUT/server.log"
        exit 1
    fi
    # `|| true`: under `set -euo pipefail` (line 38) a no-match grep (empty log on
    # early iterations) or a head-induced SIGPIPE would otherwise abort the whole
    # script at this assignment before we can retry.
    PORT=$(grep -oE 'listening on [0-9.]+:[0-9]+' "$NETWORK_OUTPUT/server.log" 2>/dev/null \
           | grep -oE '[0-9]+$' | tail -1 || true)
    [ -n "$PORT" ] && break
    sleep 0.2
done
if [ -z "$PORT" ]; then
    echo -e "${RED}Error: could not detect server's bound port${NC}"
    cat "$NETWORK_OUTPUT/server.log"
    exit 1
fi
echo "  Server PID: $SERVER_PID (port $PORT, OS-assigned)"

# Start client 1
"$MTG_BIN" connect \
    "$DECK1" \
    --server "localhost:$PORT" \
    --controller "$CONTROLLER_P1" \
    --seed-player "$CONTROLLER_SEED" \
    --name "$P1_NAME" \
    --tag-gamelogs \
    --gamelog-output "$NETWORK_OUTPUT/client1_gamelog.txt" \
    > "$NETWORK_OUTPUT/client1.log" 2>&1 &
CLIENT1_PID=$!
echo "  Client 1 PID: $CLIENT1_PID ($P1_NAME - $CONTROLLER_P1)"

# SEATING DETERMINISM (mtg-586): the server seats players by Authenticate
# arrival order (first authenticator => p1/creator). The local run always maps
# DECK1->p1 ($P1_NAME); the network run MUST seat identically or the per-player
# shuffle + first player swap and the games diverge from turn 1. A fixed `sleep`
# head-start is racy under load (each client loads the full card DB before
# authenticating). Block until the server confirms client 1 is the creator.
seat_waited=0
while ! grep -qE "created by $P1_NAME|starting $P1_NAME vs" "$NETWORK_OUTPUT/server.log" 2>/dev/null; do
    sleep 0.2; seat_waited=$((seat_waited+1))
    kill -0 $CLIENT1_PID 2>/dev/null || break
    kill -0 $SERVER_PID 2>/dev/null || break
    [ "$seat_waited" -ge 150 ] && break   # ~30s cap then proceed
done

# Start client 2
"$MTG_BIN" connect \
    "$DECK2" \
    --server "localhost:$PORT" \
    --controller "$CONTROLLER_P2" \
    --seed-player "$CONTROLLER_SEED" \
    --name "$P2_NAME" \
    --tag-gamelogs \
    --gamelog-output "$NETWORK_OUTPUT/client2_gamelog.txt" \
    > "$NETWORK_OUTPUT/client2.log" 2>&1 &
CLIENT2_PID=$!
echo "  Client 2 PID: $CLIENT2_PID ($P2_NAME - $CONTROLLER_P2)"

echo
echo "Both games running in parallel. Waiting for completion..."
echo

# ============================================================================
# Wait for both games to complete (with timeout)
# ============================================================================
TIMEOUT=180
ELAPSED=0
LOCAL_DONE=0
NETWORK_DONE=0
CLIENT1_DONE=0
CLIENT2_DONE=0

# Network-game completion: both CLIENTS must exit. Since commit 67f046f0
# (multi-game lobby), the server process is long-lived and intentionally
# outlives any single game, so `kill -0 $SERVER_PID` is no longer a valid
# "game done" signal. Clients have authoritative end-of-game knowledge:
# they exit cleanly once they receive GameEnded. We poll both client PIDs
# and then shut the server down ourselves (we started it).
while [ $ELAPSED -lt $TIMEOUT ]; do
    # Check local game
    if [ $LOCAL_DONE -eq 0 ] && ! kill -0 $LOCAL_PID 2>/dev/null; then
        wait $LOCAL_PID 2>/dev/null
        LOCAL_EXIT=$?
        LOCAL_DONE=1
        echo -e "  ${GREEN}Local game finished (exit $LOCAL_EXIT)${NC}"
    fi

    # Check network game: both clients exiting == game over.
    if [ $NETWORK_DONE -eq 0 ]; then
        if [ $CLIENT1_DONE -eq 0 ] && ! kill -0 $CLIENT1_PID 2>/dev/null; then
            wait $CLIENT1_PID 2>/dev/null
            CLIENT1_EXIT=$?
            CLIENT1_DONE=1
            echo -e "  ${GREEN}Network client 1 finished (exit $CLIENT1_EXIT)${NC}"
        fi
        if [ $CLIENT2_DONE -eq 0 ] && ! kill -0 $CLIENT2_PID 2>/dev/null; then
            wait $CLIENT2_PID 2>/dev/null
            CLIENT2_EXIT=$?
            CLIENT2_DONE=1
            echo -e "  ${GREEN}Network client 2 finished (exit $CLIENT2_EXIT)${NC}"
        fi
        if [ $CLIENT1_DONE -eq 1 ] && [ $CLIENT2_DONE -eq 1 ]; then
            # Both clients have GameEnded. Shut down the lobby server we spawned.
            kill $SERVER_PID 2>/dev/null || true
            wait $SERVER_PID 2>/dev/null || true
            NETWORK_DONE=1
            echo -e "  ${GREEN}Network game finished (clients done; server shut down)${NC}"
        fi
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
    exit 1
fi

if [ $NETWORK_DONE -eq 0 ]; then
    echo -e "${RED}Error: Network game timed out after ${TIMEOUT}s (client1_done=$CLIENT1_DONE, client2_done=$CLIENT2_DONE)${NC}"
    kill $SERVER_PID $CLIENT1_PID $CLIENT2_PID 2>/dev/null || true
    exit 1
fi

echo
echo "=== Analyzing Results ==="
echo

# ============================================================================
# Extract and compare results
# ============================================================================

# Extract turns played from local game
LOCAL_TURNS=$(grep -o "Turns played: [0-9]*" "$LOCAL_OUTPUT/game.log" | grep -o "[0-9]*" || echo "?")

# Extract action_count from network game (client log has it)
NETWORK_ACTION_COUNT=$(grep -o "action_count: [0-9]*" "$NETWORK_OUTPUT/client1.log" | tail -1 | grep -o "[0-9]*" || echo "?")

# Also extract turns from GAMELOG entries (count max Turn number)
LOCAL_MAX_TURN=$(grep -o '\[GAMELOG Turn[0-9]*' "$LOCAL_OUTPUT/game.log" | grep -o '[0-9]*' | sort -n | tail -1 || echo "?")
NETWORK_MAX_TURN=$(grep -o '\[GAMELOG Turn[0-9]*' "$NETWORK_OUTPUT/client1.log" | grep -o '[0-9]*' | sort -n | tail -1 || echo "?")

echo "Turns/Action counts:"
echo "  Local turns:  $LOCAL_TURNS (max turn in gamelog: $LOCAL_MAX_TURN)"
echo "  Network:      action_count=$NETWORK_ACTION_COUNT (max turn in gamelog: $NETWORK_MAX_TURN)"

# Extract winners
LOCAL_WINNER=$(grep -o "Winner: [A-Za-z0-9_-]*" "$LOCAL_OUTPUT/game.log" | head -1 | sed 's/Winner: //' || echo "?")
# Network client logs format: "winner=Some(1)"
NETWORK_WINNER=$(grep -o "winner=Some([0-9])" "$NETWORK_OUTPUT/client1.log" | tail -1 | grep -o "[0-9]" || echo "?")

echo
echo "Winners:"
echo "  Local:   $LOCAL_WINNER"
echo "  Network: $NETWORK_WINNER"

# Extract and compare GAMELOG entries
echo
echo "GAMELOG comparison:"

LOCAL_GAMELOG="$OUTPUT_DIR/local_gamelog.txt"
NETWORK_GAMELOG="$OUTPUT_DIR/network_gamelog.txt"

# Extract GAMELOG entries from LOCAL (excluding noise: Tap, resolves, damage messages)
# Damage messages are filtered because SERVER logs damage from GameLoop while clients
# may have slight timing differences in when damage is observed
# CRITICAL: strip ANSI color escapes FIRST, on BOTH logs, before filtering.
# The local game.log is colorized; the server runs --no-color-logs. If you
# grep before stripping, `^\s*\[GAMELOG` never matches a colored local line
# (its leading bytes are ESC, not whitespace), so colored GAMELOG lines get
# dropped from LOCAL only -> false line-count divergence -> phantom "desync"
# for any game that emits colored damage lines (e.g. Iroh's Demonstration
# DamageAll). Stripping both sides first makes the extraction symmetric.
# (mtg-eufuc was a misdiagnosis of exactly this harness bug.)
STRIP_ANSI='s/\x1b\[[0-9;]*m//g'
sed -E "$STRIP_ANSI" "$LOCAL_OUTPUT/game.log" 2>/dev/null | \
    grep '^\s*\[GAMELOG' | \
    grep -v 'Tap.*for {' | \
    grep -v 'resolves$' | \
    grep -v 'takes.*damage.*life:' | \
    grep -v 'deals.*damage.*life:' \
    > "$LOCAL_GAMELOG" || true

# Extract SERVER gamelogs (authoritative, has full card info). Same ANSI strip
# applied for symmetry even though the server already runs --no-color-logs.
sed -E "$STRIP_ANSI" "$NETWORK_OUTPUT/server.log" 2>/dev/null | \
    grep '\[GAMELOG' | \
    grep -v 'Tap.*for {' | \
    grep -v 'resolves$' | \
    grep -v 'takes.*damage.*life:' | \
    grep -v 'deals.*damage.*life:' \
    > "$NETWORK_GAMELOG" || true

LOCAL_GAMELOG_COUNT=$(wc -l < "$LOCAL_GAMELOG" 2>/dev/null || echo "0")
NETWORK_GAMELOG_COUNT=$(wc -l < "$NETWORK_GAMELOG" 2>/dev/null || echo "0")

echo "  Local GAMELOG entries:   $LOCAL_GAMELOG_COUNT"
echo "  Server GAMELOG entries:  $NETWORK_GAMELOG_COUNT"

# ============================================================================
# Verify results
# ============================================================================
echo
echo "=== Verification ==="
EXIT_CODE=0

# Check both games completed (have max turn data)
if [ "$LOCAL_MAX_TURN" != "?" ] && [ "$NETWORK_MAX_TURN" != "?" ]; then
    echo -e "${GREEN}✓ Both games completed (local: $LOCAL_MAX_TURN turns, network: $NETWORK_MAX_TURN turns)${NC}"
    # Note: Games may have different lengths due to different information visibility
    # affecting heuristic AI decisions
else
    echo -e "${RED}✗ One or both games did not complete (local: $LOCAL_MAX_TURN, network: $NETWORK_MAX_TURN)${NC}"
    EXIT_CODE=1
fi

# Check both games have winners
if [ "$LOCAL_WINNER" != "?" ] && [ "$NETWORK_WINNER" != "?" ]; then
    echo -e "${GREEN}✓ Both games have winners (local: $LOCAL_WINNER, network player: $NETWORK_WINNER)${NC}"
    # Note: Winners may differ due to different information visibility
else
    echo -e "${RED}✗ Could not determine winner for one or both games${NC}"
    EXIT_CODE=1
fi

# STRICT REQUIREMENT: Gamelogs must be IDENTICAL
if [ "$LOCAL_GAMELOG_COUNT" -gt 0 ] && [ "$NETWORK_GAMELOG_COUNT" -gt 0 ]; then
    echo "Both games produced GAMELOG entries (local: $LOCAL_GAMELOG_COUNT, server: $NETWORK_GAMELOG_COUNT)"

    # Compare LOCAL vs SERVER gamelogs - STRICT: must be identical
    DIFF_OUTPUT=$(diff "$LOCAL_GAMELOG" "$NETWORK_GAMELOG" 2>/dev/null || true)
    if [ -z "$DIFF_OUTPUT" ]; then
        DIFF_COUNT=0
    else
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

# ============================================================================
# PERSPECTIVE-AWARE server ↔ client ↔ client GAMELOG comparison
# ============================================================================
# The local↔server check above is EXACT (both have full information). A client
# only has shadow state and legitimately masks hidden-zone identities, so its
# log is compared against the server with a WEAKER, perspective-aware oracle:
# public-zone events must be byte-identical across server and every client;
# per-perspective hidden-zone masking (drawn-card names, unresolved `Unknown`
# card names, the lethal-check loss-line timing) is tolerated. A real
# public-zone divergence here is a desync / info-leak finding (DIVERGED>0).
#
# The oracle itself lives in bug_finding/network_test_lib.py
# (compare_gamelogs_perspective) — single source of truth, shared with the
# python harness. We invoke it here rather than re-implementing in bash.
echo
echo "=== Perspective-aware server↔client↔client GAMELOG comparison ==="
# Gate the oracle itself first: assert it still tolerates hidden-zone masking
# AND still catches a real public-zone divergence (guards against the oracle
# silently degrading into a no-op).
if ! WORKSPACE_ROOT="$WORKSPACE_ROOT" python3 -c \
    "import sys,os; sys.path.insert(0, os.path.join(os.environ['WORKSPACE_ROOT'],'bug_finding')); import network_test_lib as L; L.oracle_self_test(); print('  oracle self-test: PASS')" 2>&1; then
    echo -e "${RED}✗ Perspective oracle self-test FAILED — oracle logic is broken${NC}"
    EXIT_CODE=1
fi
PERSP_OUT="$(WORKSPACE_ROOT="$WORKSPACE_ROOT" python3 - \
                       "$NETWORK_OUTPUT/server.log" \
                       "$NETWORK_OUTPUT/client1.log" \
                       "$NETWORK_OUTPUT/client2.log" <<'PYEOF'
import sys, os
server_log, client1_log, client2_log = sys.argv[1], sys.argv[2], sys.argv[3]
sys.path.insert(0, os.path.join(os.environ['WORKSPACE_ROOT'], 'bug_finding'))
from network_test_lib import (extract_gamelog_perspective,
                              compare_gamelogs_perspective)
srv = extract_gamelog_perspective(server_log)
c1 = extract_gamelog_perspective(client1_log)
c2 = extract_gamelog_perspective(client2_log)
total = 0
samples = []
print(f"  server entries: {len(srv)}  client1: {len(c1)}  client2: {len(c2)}")
for label, cl in (("client1", c1), ("client2", c2)):
    if not srv or not cl:
        print(f"  WARN: empty gamelog for server or {label}; skipping")
        continue
    n, sample = compare_gamelogs_perspective(srv, cl, label)
    print(f"  server vs {label}: {n} public-zone divergence(s)")
    total += n
    if sample:
        samples.append(sample)
print(f"DIVERGED:{total}")
for s in samples:
    print(s)
PYEOF
)"
echo "$PERSP_OUT"
PERSP_DIVERGED="$(echo "$PERSP_OUT" | grep -oE 'DIVERGED:[0-9]+' | grep -oE '[0-9]+' | tail -1 || echo "?")"
if [ "$PERSP_DIVERGED" = "0" ]; then
    echo -e "${GREEN}✓ SERVER and CLIENTS agree on all PUBLIC-zone events (perspective-aware)${NC}"
elif [ "$PERSP_DIVERGED" = "?" ] || [ -z "$PERSP_DIVERGED" ]; then
    echo -e "${RED}✗ Perspective comparison did not run (no DIVERGED marker)${NC}"
    EXIT_CODE=1
else
    echo -e "${RED}✗ SERVER↔CLIENT public-zone divergence: $PERSP_DIVERGED line(s) — real desync/info-leak${NC}"
    EXIT_CODE=1
fi

# Check for errors in logs (look for panic/crash indicators, avoid card name false positives)
ERRORS=""
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

exit $EXIT_CODE
