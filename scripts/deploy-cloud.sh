#!/usr/bin/env bash
# deploy-cloud.sh - Idempotent deploy of the mtg-forge-rs web UI AND the
# native Rust lobby server to a cloud VM reachable via passwordless SSH.
#
# WHAT IT DOES
#   1. Locally ensures the WASM data export and wasm-pack bundle exist
#      (runs `make wasm-export wasm-network` only if `web/data/` or
#      `web/pkg/` are missing).
#   2. Locally ensures a release `mtg` binary with --features network is
#      built (unless SKIP_NATIVE_BUILD=1).
#   3. Generates `web/server-config.js` so the landing page knows the
#      public WebSocket URL of the Rust server.
#   4. Rsyncs the `web/` directory to the remote, EXCLUDING `images/`
#      (a separate one-time rsync handles those — 4 GB), node_modules,
#      logs, screenshots, and test artefacts.
#   5. Rsyncs the `cardsfolder/` (≈130 MB) to the remote.
#   6. Rsyncs the release `mtg` binary to the remote.
#   7. (Re)starts the static web server in tmux session `mtg-server` on
#      $REMOTE_PORT (default 8080).
#   8. (Re)starts the native Rust lobby server in tmux session
#      `mtg-rust-server` on $RUST_SERVER_PORT (default 17810), listening
#      on 0.0.0.0 with cardsfolder as its card database.
#
# WHAT IT DOES NOT DO
#   - No toolchain install on the VM (no cargo, no wasm-pack). The Rust
#     binary is built LOCALLY and rsync'd.
#   - No firewall changes. Ports $REMOTE_PORT and $RUST_SERVER_PORT must
#     already be reachable. The VM currently listens on 22/80/443 only;
#     if external access on 8080 / 17810 is required, open them manually:
#       sudo ufw allow $REMOTE_PORT/tcp
#       sudo ufw allow $RUST_SERVER_PORT/tcp
#   - No copy of `web/images/` (in-flight rsync handles that separately).
#
# REMOTE ASSUMPTIONS
#   - Passwordless SSH to ${REMOTE} works.
#   - `python3` and `tmux` are installed on the VM. tmux is installed by
#     this script if missing.
#   - The remote architecture matches the local build target (both are
#     assumed x86_64 Linux). If you build on macOS/ARM, set
#     SKIP_NATIVE_BUILD=1 and provide the binary manually.
#   - Remote home contains writeable `~/mtg-forge-rs/`.
#
# IDEMPOTENCY
#   - Uses rsync, not scp. Re-running only transfers deltas.
#   - tmux sessions are killed and restarted on every run so the servers
#     pick up new artefacts.
#
# OPERATIONS
#   Start (re-deploy):      ./scripts/deploy-cloud.sh
#   Stop web server:        ssh newton@deepscry.net 'tmux kill-session -t mtg-server'
#   Stop Rust server:       ssh newton@deepscry.net 'tmux kill-session -t mtg-rust-server'
#   View Rust server log:   ssh newton@deepscry.net 'tail -f ~/mtg-forge-rs/rust-server.log'
#   Attach interactive:     ssh -t newton@deepscry.net 'tmux attach -t mtg-rust-server'
#
# ENVIRONMENT
#   REMOTE              SSH target (default: newton@deepscry.net)
#   REMOTE_DIR          Remote install dir (default: mtg-forge-rs)
#   REMOTE_PORT         Static web HTTP port  (default: 8080)
#   RUST_SERVER_PORT    Native Rust lobby port (default: 17810)
#   TMUX_SESSION        Web tmux session name  (default: mtg-server)
#   RUST_TMUX_SESSION   Rust tmux session name (default: mtg-rust-server)
#   REBUILD=1           Force `make wasm-export wasm-network` even if outputs exist
#   SKIP_NATIVE_BUILD=1 Skip local `cargo build` of `mtg` binary
#   CARDSFOLDER         Override cards source path

set -euo pipefail

REMOTE="${REMOTE:-newton@deepscry.net}"
REMOTE_DIR="${REMOTE_DIR:-mtg-forge-rs}"
REMOTE_PORT="${REMOTE_PORT:-8080}"
RUST_SERVER_PORT="${RUST_SERVER_PORT:-17810}"
TMUX_SESSION="${TMUX_SESSION:-mtg-server}"
RUST_TMUX_SESSION="${RUST_TMUX_SESSION:-mtg-rust-server}"

# Resolve repo root from this script's location (works from any CWD).
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

echo "=== mtg-forge-rs cloud deploy ==="
echo "Remote:       $REMOTE:~/$REMOTE_DIR"
echo "Web port:     $REMOTE_PORT  (tmux: $TMUX_SESSION)"
echo "Rust port:    $RUST_SERVER_PORT  (tmux: $RUST_TMUX_SESSION)"
echo "Repo root:    $REPO_ROOT"
echo

# Derive the public host from REMOTE (strip user@). Used for server-config.js.
PUBLIC_HOST="${REMOTE##*@}"

# --- 1. Build WASM artefacts locally if missing -----------------------------
if [[ -z "${CARDSFOLDER:-}" && ! -d cardsfolder/. ]]; then
    PRIMARY_CARDS="$(cd "$REPO_ROOT/.." 2>/dev/null && pwd)/../mtg-forge-rs/forge-java/forge-gui/res/cardsfolder"
    if [[ -d "$PRIMARY_CARDS" ]]; then
        export CARDSFOLDER="$PRIMARY_CARDS"
        echo "--- using CARDSFOLDER=$CARDSFOLDER (worktree submodule not initialised) ---"
    fi
fi

need_build=0
if [[ ! -d web/data || ! -f web/data/decks.bin || ! -d web/pkg ]]; then
    need_build=1
fi
if [[ "${REBUILD:-0}" = "1" ]]; then
    need_build=1
fi

if (( need_build )); then
    echo "--- running make wasm-export wasm-network ---"
    make wasm-export wasm-network
else
    echo "--- web/data and web/pkg present; skipping local WASM build (set REBUILD=1 to force) ---"
fi

for f in web/pkg/mtg_forge_rs_bg.wasm web/data/cards.bin web/data/decks.bin; do
    [[ -f "$f" ]] || { echo "ERROR: missing required artefact: $f" >&2; exit 1; }
done

# --- 2. Build the native release binary -------------------------------------
NATIVE_BIN="target/release/mtg"
if [[ "${SKIP_NATIVE_BUILD:-0}" != "1" ]]; then
    if [[ ! -x "$NATIVE_BIN" || "${REBUILD:-0}" = "1" ]]; then
        echo "--- building release mtg binary (--features network) ---"
        cargo build --release --bin mtg --features network
    else
        echo "--- $NATIVE_BIN present; skipping local native build (set REBUILD=1 to force) ---"
    fi
fi
[[ -x "$NATIVE_BIN" ]] || {
    echo "ERROR: $NATIVE_BIN missing. Build with: cargo build --release --bin mtg --features network" >&2
    exit 1
}

# --- 3. Generate server-config.js for the landing page ----------------------
# This file is consumed by web/index.html to know where the Rust lobby is.
cat > web/server-config.js <<EOF
// AUTO-GENERATED by scripts/deploy-cloud.sh on $(date -u +%Y-%m-%dT%H:%M:%SZ).
// Points the landing-page lobby at the deployed native Rust server.
//
// Override at runtime with ?ws=ws://host:port in the page URL, or by
// setting window.MTG_WS_URL before this script tag loads.
(function () {
    if (!window.MTG_WS_URL) {
        window.MTG_WS_URL = "ws://$PUBLIC_HOST:$RUST_SERVER_PORT";
    }
})();
EOF
echo "--- generated web/server-config.js pointing at ws://$PUBLIC_HOST:$RUST_SERVER_PORT ---"

# --- 4. Ensure remote layout and tmux exist ---------------------------------
ssh "$REMOTE" "
    set -e
    mkdir -p ~/$REMOTE_DIR/web ~/$REMOTE_DIR/cardsfolder ~/$REMOTE_DIR/bin ~/$REMOTE_DIR/bug_reports
    if ! command -v tmux >/dev/null 2>&1; then
        echo 'Installing tmux on remote...'
        sudo -n apt-get install -y tmux || {
            echo 'ERROR: tmux not installed and passwordless sudo unavailable' >&2
            exit 1
        }
    fi
"

# --- 5. Rsync web/ ----------------------------------------------------------
echo "--- rsyncing web/ to $REMOTE:~/$REMOTE_DIR/web/ ---"
rsync -avh --delete \
    --exclude='images/' \
    --exclude='images' \
    --exclude='node_modules/' \
    --exclude='screenshots/' \
    --exclude='server.log' \
    --exclude='*.log' \
    --exclude='package-lock.json' \
    --exclude='test_*.js' \
    --exclude='network_*_test_results.json' \
    web/ "$REMOTE:$REMOTE_DIR/web/"

# --- 6. Rsync cardsfolder/ (≈130 MB, follow symlink) ------------------------
CARDS_SRC=""
if [[ -d cardsfolder/. ]]; then
    CARDS_SRC="cardsfolder/"
elif [[ -n "${CARDSFOLDER:-}" && -d "$CARDSFOLDER" ]]; then
    CARDS_SRC="$CARDSFOLDER/"
fi
if [[ -n "$CARDS_SRC" ]]; then
    echo "--- rsyncing $CARDS_SRC to $REMOTE:~/$REMOTE_DIR/cardsfolder/ ---"
    rsync -avh --delete --copy-links \
        "$CARDS_SRC" "$REMOTE:$REMOTE_DIR/cardsfolder/"
else
    echo "--- cardsfolder/ unavailable; web/data/cards.bin still works for the WASM demo but the Rust server requires cardsfolder; aborting." >&2
    exit 1
fi

# --- 7. Rsync the native release binary -------------------------------------
echo "--- rsyncing $NATIVE_BIN to $REMOTE:~/$REMOTE_DIR/bin/mtg ---"
rsync -avh "$NATIVE_BIN" "$REMOTE:$REMOTE_DIR/bin/mtg"

# --- 8. (Re)start the web server in a detached tmux session -----------------
echo "--- (re)starting static web server on $REMOTE port $REMOTE_PORT ---"
ssh "$REMOTE" "
    set -e
    tmux kill-session -t $TMUX_SESSION 2>/dev/null || true
    cd ~/$REMOTE_DIR/web
    tmux new-session -d -s $TMUX_SESSION \
        \"python3 -m http.server $REMOTE_PORT 2>&1 | tee ~/$REMOTE_DIR/server.log\"
    sleep 1
    if ! tmux has-session -t $TMUX_SESSION 2>/dev/null; then
        echo 'ERROR: web tmux session failed to start' >&2
        exit 1
    fi
"

# --- 9. (Re)start the native Rust lobby server ------------------------------
echo "--- (re)starting native Rust lobby on $REMOTE port $RUST_SERVER_PORT ---"
ssh "$REMOTE" "
    set -e
    tmux kill-session -t $RUST_TMUX_SESSION 2>/dev/null || true
    cd ~/$REMOTE_DIR
    chmod +x ~/$REMOTE_DIR/bin/mtg
    tmux new-session -d -s $RUST_TMUX_SESSION \
        \"./bin/mtg server --port $RUST_SERVER_PORT --cardsfolder ./cardsfolder 2>&1 | tee ~/$REMOTE_DIR/rust-server.log\"
    sleep 2
    if ! tmux has-session -t $RUST_TMUX_SESSION 2>/dev/null; then
        echo 'ERROR: Rust server tmux session failed to start' >&2
        tail -50 ~/$REMOTE_DIR/rust-server.log 2>/dev/null || true
        exit 1
    fi
    echo 'Listening sockets:'
    ss -tlnp 2>/dev/null | grep -E ':($REMOTE_PORT|$RUST_SERVER_PORT)' || echo '  (none — check the logs)'
"

echo
echo "=== Deploy complete ==="
echo "Landing page:   http://$PUBLIC_HOST:$REMOTE_PORT/"
echo "Lobby WS URL:   ws://$PUBLIC_HOST:$RUST_SERVER_PORT"
echo "Web logs:       ssh $REMOTE 'tail -f ~/$REMOTE_DIR/server.log'"
echo "Rust logs:      ssh $REMOTE 'tail -f ~/$REMOTE_DIR/rust-server.log'"
echo "Stop web:       ssh $REMOTE 'tmux kill-session -t $TMUX_SESSION'"
echo "Stop Rust:      ssh $REMOTE 'tmux kill-session -t $RUST_TMUX_SESSION'"
