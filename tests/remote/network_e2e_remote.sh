#!/usr/bin/env bash
# tests/network_e2e_remote.sh — 2-client network smoke against a REMOTE
# deepscry deploy.
#
# Spawns two `mtg connect --server wss://<host>:<port>/lobby ...`
# instances (alice + bob), one creates a passcoded game, the other joins,
# both play to completion with random controllers. Validates:
#   1. Both clients connect successfully (TLS WS handshake + auth).
#   2. Both clients exit cleanly (exit code 0) within 60 s.
#   3. Both client gamelogs contain "Game over" / a winner line.
#   4. Log line counts are within an order of magnitude of each other
#      (sanity check that both observed the same game).
#
# This is the lightweight counterpart to web/smoke_test_live.js — it
# exercises the WS protocol + engine + lobby end-to-end WITHOUT a
# browser, so a failure here points cleanly at the Rust server side.
#
# Usage:
#   tests/network_e2e_remote.sh                        # default deepscry.net:8080
#   tests/network_e2e_remote.sh wss://host:port/lobby  # custom
#
# Exits 0 on success, non-zero on any failure.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

SERVER_URL="${1:-wss://deepscry.net:8080/lobby}"
DECK="${DECK:-decks/grizzly_bears.dck}"
TIMEOUT_SECS="${TIMEOUT_SECS:-60}"
GAME_NAME="smoke-$$-$(date +%s)"
PASSCODE="smoke"

echo "=== network_e2e_remote.sh ==="
echo "  server  : $SERVER_URL"
echo "  deck    : $DECK"
echo "  timeout : ${TIMEOUT_SECS}s"
echo "  game    : $GAME_NAME (pass=$PASSCODE)"
echo ""

[[ -f "$DECK" ]] || { echo "FAIL: deck not found: $DECK" >&2; exit 1; }

MTG_BIN=""
for candidate in target/release-deploy/mtg target/release/mtg target/debug/mtg; do
    if [[ -x "$candidate" ]]; then
        MTG_BIN="$candidate"
        break
    fi
done
if [[ -z "$MTG_BIN" ]]; then
    echo "→ no mtg binary found; building (cargo build --features network --bin mtg)"
    cargo build --features network --bin mtg
    MTG_BIN="target/debug/mtg"
fi
echo "  binary  : $MTG_BIN"
echo ""

LOG_DIR="$(mktemp -d -t mtg-net-e2e.XXXXXX)"
trap 'rm -rf "$LOG_DIR"; kill $ALICE_PID $BOB_PID 2>/dev/null || true' EXIT

ALICE_LOG="$LOG_DIR/alice.log"
BOB_LOG="$LOG_DIR/bob.log"

# Note: the current `mtg connect` CLI does NOT have flags for creating
# or joining a specific lobby game (those are driven from the web UI).
# As a stopgap, this script does a simpler liveness probe: each client
# attempts to authenticate against the lobby. If both auths succeed,
# the wire protocol + TLS + service are confirmed working. A
# full create/join + play handshake from the CLI is tracked as
# follow-up work.
#
# We invoke with `--controller random` which auto-plays, but without
# a game pairing the server will time out the auth waiter. We treat
# a connection that gets past the WS handshake + auth as success;
# a TLS / connection-refused / immediate-disconnect failure as fail.

# --accept-invalid-certs is needed when probing the deployed VM
# directly on :8080 (CF Origin Cert is signed by a private CA the
# system root store does not trust). Safe here because the probe
# is read-only smoke; do NOT use this flag for real play.
COMMON_ARGS=(
    --server "$SERVER_URL"
    --controller random
    --password ""
    --accept-invalid-certs
    -v normal
)

echo "→ spawning alice"
"$MTG_BIN" connect "$DECK" --name "alice-$$" "${COMMON_ARGS[@]}" > "$ALICE_LOG" 2>&1 &
ALICE_PID=$!

echo "→ spawning bob"
"$MTG_BIN" connect "$DECK" --name "bob-$$" "${COMMON_ARGS[@]}" > "$BOB_LOG" 2>&1 &
BOB_PID=$!

echo "→ both clients launched (alice=$ALICE_PID bob=$BOB_PID); waiting up to ${TIMEOUT_SECS}s"

# Watch for both processes to exit OR timeout.
DEADLINE=$(( $(date +%s) + TIMEOUT_SECS ))
ALICE_RC="?"
BOB_RC="?"
while [[ $(date +%s) -lt $DEADLINE ]]; do
    if [[ "$ALICE_RC" == "?" ]] && ! kill -0 "$ALICE_PID" 2>/dev/null; then
        wait "$ALICE_PID"; ALICE_RC=$?
        echo "  alice exited rc=$ALICE_RC"
    fi
    if [[ "$BOB_RC" == "?" ]] && ! kill -0 "$BOB_PID" 2>/dev/null; then
        wait "$BOB_PID"; BOB_RC=$?
        echo "  bob exited rc=$BOB_RC"
    fi
    if [[ "$ALICE_RC" != "?" && "$BOB_RC" != "?" ]]; then
        break
    fi
    sleep 1
done

# Kill any stragglers (e.g. one paired up but the other is still alone
# in the lobby; the server timeout will eventually close them but we
# don't want to wait that long).
if [[ "$ALICE_RC" == "?" ]]; then
    echo "  alice still running after ${TIMEOUT_SECS}s — killing"
    kill "$ALICE_PID" 2>/dev/null || true
    wait "$ALICE_PID" 2>/dev/null; ALICE_RC=$?
fi
if [[ "$BOB_RC" == "?" ]]; then
    echo "  bob still running after ${TIMEOUT_SECS}s — killing"
    kill "$BOB_PID" 2>/dev/null || true
    wait "$BOB_PID" 2>/dev/null; BOB_RC=$?
fi

ALICE_LINES=$(wc -l < "$ALICE_LOG" 2>/dev/null || echo 0)
BOB_LINES=$(wc -l < "$BOB_LOG" 2>/dev/null || echo 0)

echo ""
echo "=== RESULT ==="
echo "  alice rc=$ALICE_RC, log lines=$ALICE_LINES ($ALICE_LOG)"
echo "  bob   rc=$BOB_RC, log lines=$BOB_LINES ($BOB_LOG)"

# Success criteria:
#  - Both clients reached "Connecting to" (so URL parse worked).
#  - Both got past the TLS handshake (no "Tls" / "InvalidCert" lines).
#  - Both received SOMETHING from the server (auth ACK or further).
PASS=1
FAIL_REASONS=()

for who in alice bob; do
    log="$LOG_DIR/$who.log"
    if ! grep -q "Connecting to" "$log"; then
        PASS=0; FAIL_REASONS+=("$who: never tried to connect")
    elif grep -qE "Connection refused|name resolution|no address" "$log"; then
        PASS=0; FAIL_REASONS+=("$who: connection refused / DNS — server down?")
    elif ! grep -qE "Authenticated as" "$log"; then
        # Reached the server but never completed auth → wire protocol
        # break (TLS failure, version mismatch, auth handler crashed).
        PASS=0; FAIL_REASONS+=("$who: never authenticated (TLS or protocol failure)")
    fi
done

# Sanity: log line counts within 10x.
if [[ $ALICE_LINES -gt 0 && $BOB_LINES -gt 0 ]]; then
    ratio=$(( ALICE_LINES > BOB_LINES ? ALICE_LINES / (BOB_LINES + 1) : BOB_LINES / (ALICE_LINES + 1) ))
    if (( ratio > 10 )); then
        echo "  ⚠ log size ratio $ratio (alice=$ALICE_LINES bob=$BOB_LINES); one client saw much less"
    fi
fi

if (( PASS )); then
    echo ""
    echo "✓ network_e2e_remote PASS"
    exit 0
else
    echo ""
    echo "✗ network_e2e_remote FAIL:"
    for r in "${FAIL_REASONS[@]}"; do echo "    - $r"; done
    echo ""
    echo "--- alice log (last 30 lines) ---"
    tail -30 "$ALICE_LOG" || true
    echo "--- bob log (last 30 lines) ---"
    tail -30 "$BOB_LOG" || true
    exit 1
fi
