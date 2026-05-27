#!/usr/bin/env bash
# deploy-cloud.sh - Idempotent deploy of the DeepScry unified web+lobby
# server (axum) to a cloud VM reachable via passwordless SSH.
#
# WHAT IT DOES
#   1. Locally ensures the WASM data export and wasm-pack bundle exist
#      (runs `make wasm-export wasm-network` only if `web/data/` or
#      `web/pkg/` are missing).
#   2. Locally ensures a release `mtg` binary with --features network is
#      built (unless SKIP_NATIVE_BUILD=1). The `network` feature pulls
#      in the new `web-server` (axum + tower + rustls) sub-feature.
#   3. Rsyncs `web/`, `cardsfolder/`, and the release binary to the VM.
#   4. Installs (or refreshes) the systemd unit `infra/deepscry.service`
#      to /etc/systemd/system/ and runs `daemon-reload` + `enable --now`
#      + `restart` so the service picks up the new artefacts.
#
# WHAT IT DOES NOT DO
#   - No toolchain install on the VM (no cargo, no wasm-pack). The Rust
#     binary is built LOCALLY and rsync'd.
#   - No firewall changes. The single public port ($REMOTE_PORT, default
#     8080) must already be reachable:
#         sudo ufw allow $REMOTE_PORT/tcp
#   - No copy of `web/images/` (in-flight rsync handles that separately).
#   - No TLS cert provisioning. To enable TLS at the origin write
#     /etc/default/deepscry with
#         MTG_TLS_CERT=/etc/ssl/deepscry/deepscry.crt
#         MTG_TLS_KEY=/etc/ssl/deepscry/deepscry.key
#     and `sudo systemctl restart deepscry`.
#
# REMOTE ASSUMPTIONS
#   - Passwordless SSH to ${REMOTE} works.
#   - Passwordless `sudo` is available for systemctl + cp (only needed
#     to install/refresh the unit file).
#   - The remote architecture matches the local build target (both are
#     assumed x86_64 Linux).
#   - Remote home contains writeable `~/mtg-forge-rs/`.
#
# IDEMPOTENCY
#   - Uses rsync; re-running only transfers deltas.
#   - The systemd unit is the single source of truth for service state;
#     restarts pick up new binary + assets on disk.
#
# OPERATIONS
#   Start (re-deploy):      ./scripts/deploy-cloud.sh
#   Stop service:           ssh newton@deepscry.net 'sudo systemctl stop deepscry'
#   View logs:              ssh newton@deepscry.net 'sudo journalctl -u deepscry -f'
#   Service status:         ssh newton@deepscry.net 'systemctl status deepscry'
#
# ENVIRONMENT
#   REMOTE              SSH target (default: newton@deepscry.net)
#   REMOTE_DIR          Remote install dir (default: mtg-forge-rs)
#   REMOTE_PORT         Public bind port (default: 8080)
#   REBUILD=1           Force `make wasm-export wasm-network` even if outputs exist
#   SKIP_NATIVE_BUILD=1 Skip local `cargo build` of `mtg` binary
#   CARDSFOLDER         Override cards source path
#   SKIP_SYSTEMD=1      Skip the systemd install/restart step (for dry runs)

set -euo pipefail

REMOTE="${REMOTE:-newton@deepscry.net}"
REMOTE_DIR="${REMOTE_DIR:-mtg-forge-rs}"
REMOTE_PORT="${REMOTE_PORT:-8080}"

# Resolve repo root from this script's location (works from any CWD).
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

echo "=== DeepScry cloud deploy (unified axum server) ==="
echo "Remote:       $REMOTE:~/$REMOTE_DIR"
echo "Public port:  $REMOTE_PORT"
echo "Repo root:    $REPO_ROOT"
echo

# Derive the public host from REMOTE (strip user@). Used for the final
# summary message. The runtime no longer needs PUBLIC_HOST baked in
# anywhere — the web page derives its WS URL from window.location.
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

# --- 3. Ensure remote layout exists -----------------------------------------
ssh "$REMOTE" "
    set -e
    mkdir -p ~/$REMOTE_DIR/web ~/$REMOTE_DIR/cardsfolder ~/$REMOTE_DIR/bin ~/$REMOTE_DIR/bug_reports
"

# --- 4. Rsync web/ ----------------------------------------------------------
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

# --- 5. Rsync cardsfolder/ (≈130 MB, follow symlink) ------------------------
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
    echo "--- cardsfolder/ unavailable; the embedded lobby requires it; aborting." >&2
    exit 1
fi

# --- 6. Rsync the native release binary -------------------------------------
echo "--- rsyncing $NATIVE_BIN to $REMOTE:~/$REMOTE_DIR/bin/mtg ---"
rsync -avh "$NATIVE_BIN" "$REMOTE:$REMOTE_DIR/bin/mtg"

# --- 7. Install / refresh the systemd unit and restart the service ---------
if [[ "${SKIP_SYSTEMD:-0}" = "1" ]]; then
    echo "--- SKIP_SYSTEMD=1; not touching systemd (binary + assets are in place) ---"
else
    echo "--- installing infra/deepscry.service and (re)starting deepscry ---"
    rsync -avh infra/deepscry.service "$REMOTE:/tmp/deepscry.service"
    ssh "$REMOTE" "
        set -e
        chmod +x ~/$REMOTE_DIR/bin/mtg
        # Refresh the unit file if it changed; harmless cp + reload otherwise.
        sudo cp /tmp/deepscry.service /etc/systemd/system/deepscry.service
        sudo systemctl daemon-reload
        sudo systemctl enable --now deepscry
        sudo systemctl restart deepscry
        sleep 1
        sudo systemctl --no-pager --lines=10 status deepscry || true
        echo 'Listening sockets:'
        ss -tlnp 2>/dev/null | grep -E \":$REMOTE_PORT\\b\" || echo '  (none on $REMOTE_PORT — check journalctl)'
    "
fi

echo
echo "=== Deploy complete ==="
echo "Landing page:   http://$PUBLIC_HOST:$REMOTE_PORT/"
echo "Lobby WS URL:   derived in-browser as ws(s)://<host>/lobby (same origin)"
echo "Service logs:   ssh $REMOTE 'sudo journalctl -u deepscry -f'"
echo "Stop service:   ssh $REMOTE 'sudo systemctl stop deepscry'"
