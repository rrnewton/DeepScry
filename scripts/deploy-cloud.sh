#!/usr/bin/env bash
# deploy-cloud.sh — deploy the mtg-forge-rs web UI + native lobby server
# to a cloud VM. Two phases:
#
#   config   Run ONCE per VM (or whenever infra changes). Bootstraps
#            the remote — creates dirs, installs the systemd unit and
#            env file, opens the firewall port, optionally configures
#            passwordless sudo for the deploy phase, kills legacy
#            (python http.server + standalone mtg) tmux sessions and
#            disables old systemd units. Idempotent.
#
#   deploy   Run on every code change. Rsyncs the release binary,
#            web/, cardsfolder/, then restarts the service. Does NOT
#            require root.
#
# NO HARDCODED HOSTNAMES, USERNAMES, OR IPS. Every site-specific
# value comes from one of:
#   1. The local config file (see scripts/deepscry-deploy.env.example;
#      copy to `<parent>/.deepscry-deploy.env`).
#   2. CLI flags (--user, --host, --port, ...).
#   3. Environment variables (REMOTE_USER, REMOTE_HOST, ...).
# CLI flags > env vars > config file > built-in defaults.
#
# USAGE:
#   scripts/deploy-cloud.sh config   [--mode user|system] [--with-sudoers] [flags...]
#   scripts/deploy-cloud.sh deploy   [flags...]
#   scripts/deploy-cloud.sh status   # show systemd status + URLs
#   scripts/deploy-cloud.sh logs     # tail the service log
#   scripts/deploy-cloud.sh          # alias for "deploy" (backwards-compat)
#
# COMMON FLAGS (both phases):
#   --user <name>          SSH login on the remote (REMOTE_USER)
#   --host <name>          Public DNS / hostname (REMOTE_HOST)
#   --ssh-host <name|ip>   SSH target if different from public host
#   --dir <path>           Remote install dir under $HOME (REMOTE_DIR)
#   --port <n>             HTTP port (REMOTE_PORT, default 8080)
#   --service <name>       systemd unit basename (SERVICE_NAME, default deepscry)
#   --tls-cert <path>      TLS cert path on the VM (TLS_CERT_PATH)
#   --tls-key <path>       TLS key path on the VM (TLS_KEY_PATH)
#   --config <file>        Override config file location
#
# DEPLOY-PHASE-ONLY FLAGS:
#   --skip-build           Don't rebuild the release binary locally
#   --skip-wasm            Don't rebuild WASM artefacts locally
#   --rebuild              Force rebuild of all artefacts

set -euo pipefail

# ---------------------------------------------------------------------------
# Resolve script location & repo root
# ---------------------------------------------------------------------------

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
# Parent dev harness (one level above the repo if the repo is the
# primary checkout); used as the first config-file search location.
PARENT_DIR="$(cd "$REPO_ROOT/.." && pwd 2>/dev/null || echo "$REPO_ROOT")"

# ---------------------------------------------------------------------------
# Argument parsing
# ---------------------------------------------------------------------------

CMD="${1:-deploy}"
case "$CMD" in
    config|deploy|status|logs|--help|-h|help)
        shift || true
        ;;
    -*|--*)
        # No subcommand — default to "deploy", keep $@ as flags.
        CMD="deploy"
        ;;
    *)
        echo "error: unknown subcommand: $CMD" >&2
        sed -n '2,/^$/p' "$0" | sed 's/^# \{0,1\}//' >&2
        exit 2
        ;;
esac

if [[ "$CMD" == "--help" || "$CMD" == "-h" || "$CMD" == "help" ]]; then
    sed -n '2,/^$/p' "$0" | sed 's/^# \{0,1\}//'
    exit 0
fi

# Defaults overridable by config file / env / flags.
CONFIG_FILE_OVERRIDE=""
SYSTEMD_MODE_OVERRIDE=""
WITH_SUDOERS=0
SKIP_BUILD=0
SKIP_WASM=0
REBUILD=0
CLI_REMOTE_USER=""
CLI_REMOTE_HOST=""
CLI_REMOTE_SSH_HOST=""
CLI_REMOTE_DIR=""
CLI_REMOTE_PORT=""
CLI_SERVICE_NAME=""
CLI_TLS_CERT=""
CLI_TLS_KEY=""

while [ $# -gt 0 ]; do
    case "$1" in
        --user) CLI_REMOTE_USER="$2"; shift 2 ;;
        --host) CLI_REMOTE_HOST="$2"; shift 2 ;;
        --ssh-host) CLI_REMOTE_SSH_HOST="$2"; shift 2 ;;
        --dir) CLI_REMOTE_DIR="$2"; shift 2 ;;
        --port) CLI_REMOTE_PORT="$2"; shift 2 ;;
        --service) CLI_SERVICE_NAME="$2"; shift 2 ;;
        --tls-cert) CLI_TLS_CERT="$2"; shift 2 ;;
        --tls-key) CLI_TLS_KEY="$2"; shift 2 ;;
        --mode) SYSTEMD_MODE_OVERRIDE="$2"; shift 2 ;;
        --with-sudoers) WITH_SUDOERS=1; shift ;;
        --skip-build) SKIP_BUILD=1; shift ;;
        --skip-wasm) SKIP_WASM=1; shift ;;
        --rebuild) REBUILD=1; shift ;;
        --config) CONFIG_FILE_OVERRIDE="$2"; shift 2 ;;
        -h|--help) sed -n '2,/^$/p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "error: unknown flag: $1" >&2; exit 2 ;;
    esac
done

# ---------------------------------------------------------------------------
# Load config file
# ---------------------------------------------------------------------------

config_search_paths=(
    "${CONFIG_FILE_OVERRIDE:-}"
    "$PARENT_DIR/.deepscry-deploy.env"
    "$REPO_ROOT/.deepscry-deploy.env"
    "$HOME/.config/deepscry/deploy.env"
)
CONFIG_FILE=""
for p in "${config_search_paths[@]}"; do
    [[ -z "$p" ]] && continue
    if [[ -f "$p" ]]; then
        CONFIG_FILE="$p"
        break
    fi
done

if [[ -n "$CONFIG_FILE" ]]; then
    # shellcheck disable=SC1090
    source "$CONFIG_FILE"
    echo "→ loaded config: $CONFIG_FILE"
fi

# Layer precedence: CLI > pre-existing env > config file > default.
REMOTE_USER="${CLI_REMOTE_USER:-${REMOTE_USER:-}}"
REMOTE_HOST="${CLI_REMOTE_HOST:-${REMOTE_HOST:-}}"
REMOTE_SSH_HOST="${CLI_REMOTE_SSH_HOST:-${REMOTE_SSH_HOST:-${REMOTE_HOST}}}"
REMOTE_DIR="${CLI_REMOTE_DIR:-${REMOTE_DIR:-mtg-forge-rs}}"
REMOTE_PORT="${CLI_REMOTE_PORT:-${REMOTE_PORT:-8080}}"
SERVICE_NAME="${CLI_SERVICE_NAME:-${SERVICE_NAME:-deepscry}}"
TLS_CERT_PATH="${CLI_TLS_CERT:-${TLS_CERT_PATH:-}}"
TLS_KEY_PATH="${CLI_TLS_KEY:-${TLS_KEY_PATH:-}}"
SYSTEMD_MODE="${SYSTEMD_MODE_OVERRIDE:-${SYSTEMD_MODE:-user}}"

if [[ -z "$REMOTE_USER" || -z "$REMOTE_HOST" ]]; then
    echo "error: REMOTE_USER and REMOTE_HOST are required." >&2
    echo "       Pass with --user / --host, set as env vars, or create" >&2
    echo "       a config file (see scripts/deepscry-deploy.env.example)." >&2
    exit 2
fi

case "$SYSTEMD_MODE" in
    user|system) ;;
    *) echo "error: SYSTEMD_MODE must be 'user' or 'system' (got '$SYSTEMD_MODE')" >&2; exit 2 ;;
esac

REMOTE_SSH="${REMOTE_USER}@${REMOTE_SSH_HOST}"
PUBLIC_HOST="$REMOTE_HOST"

# ---------------------------------------------------------------------------
# Banner
# ---------------------------------------------------------------------------

echo "═════════════════════════════════════════════════════════════════════"
echo "  deploy-cloud.sh  (subcommand: $CMD)"
echo "═════════════════════════════════════════════════════════════════════"
echo "  Remote user/host : $REMOTE_USER@$REMOTE_HOST"
echo "  SSH target       : $REMOTE_SSH"
echo "  Remote dir       : ~/$REMOTE_DIR"
echo "  HTTP port        : $REMOTE_PORT"
echo "  Service          : $SERVICE_NAME (systemd-$SYSTEMD_MODE)"
echo "  TLS cert/key     : ${TLS_CERT_PATH:-<none, plain HTTP>}"
echo "  Repo root        : $REPO_ROOT"
echo "═════════════════════════════════════════════════════════════════════"
echo ""

# ---------------------------------------------------------------------------
# Helper: render systemd unit file
# ---------------------------------------------------------------------------

render_systemd_unit() {
    # Args: $1 = mode (user|system). Output: full unit file body.
    local mode="$1"
    local user_line=""
    [[ "$mode" == "system" ]] && user_line="User=${REMOTE_USER}"
    cat <<UNIT
[Unit]
Description=DeepScry — mtg-forge-rs unified axum web + lobby server
After=network.target

[Service]
Type=simple
${user_line}
WorkingDirectory=%h/${REMOTE_DIR}
EnvironmentFile=-%h/.config/${SERVICE_NAME}/deploy.env
ExecStart=%h/${REMOTE_DIR}/bin/mtg server-web \\
    --bind 0.0.0.0:${REMOTE_PORT} \\
    --static-dir %h/${REMOTE_DIR}/web \\
    --cardsfolder %h/${REMOTE_DIR}/cardsfolder
Restart=on-failure
RestartSec=3s
LimitNOFILE=65536
MemoryMax=8G
Environment=RUST_LOG=info

[Install]
WantedBy=$( [[ "$mode" == "system" ]] && echo "multi-user.target" || echo "default.target" )
UNIT
}

render_env_file() {
    # The env file (systemd EnvironmentFile=) holds runtime secrets
    # (TLS paths) and lives in the user's $HOME so no root is needed
    # for either mode. systemd's `%h` expands to that.
    cat <<ENV
# AUTO-GENERATED by deploy-cloud.sh config. Holds runtime overrides
# for the ${SERVICE_NAME} systemd unit (mtg-forge-rs server-web).
RUST_LOG=info
ENV
    if [[ -n "$TLS_CERT_PATH" ]]; then
        echo "MTG_TLS_CERT=$TLS_CERT_PATH"
    fi
    if [[ -n "$TLS_KEY_PATH" ]]; then
        echo "MTG_TLS_KEY=$TLS_KEY_PATH"
    fi
}

# ---------------------------------------------------------------------------
# config subcommand
# ---------------------------------------------------------------------------

cmd_config() {
    echo "→ Bootstrapping remote ($SYSTEMD_MODE mode)..."

    # Render artefacts locally.
    local tmp_unit tmp_env
    tmp_unit="$(mktemp)"
    tmp_env="$(mktemp)"
    render_systemd_unit "$SYSTEMD_MODE" > "$tmp_unit"
    render_env_file > "$tmp_env"

    # Upload the systemd unit + env file to a staging area on the VM.
    rsync -q "$tmp_unit" "$REMOTE_SSH:/tmp/${SERVICE_NAME}.service.staged"
    rsync -q "$tmp_env"  "$REMOTE_SSH:/tmp/${SERVICE_NAME}.env.staged"
    rm -f "$tmp_unit" "$tmp_env"

    # Remote bootstrap script.
    ssh "$REMOTE_SSH" "REMOTE_DIR='$REMOTE_DIR' SERVICE_NAME='$SERVICE_NAME' SYSTEMD_MODE='$SYSTEMD_MODE' REMOTE_PORT='$REMOTE_PORT' WITH_SUDOERS='$WITH_SUDOERS' REMOTE_USER_NAME='$REMOTE_USER' bash -se" <<'REMOTE'
set -euo pipefail

echo "  remote: ensuring directory layout"
mkdir -p ~/"$REMOTE_DIR"/{bin,web,cardsfolder,bug_reports}
mkdir -p ~/.config/"$SERVICE_NAME"

# Move staged env file into place (in $HOME so neither mode needs root).
mv /tmp/"$SERVICE_NAME".env.staged ~/.config/"$SERVICE_NAME"/deploy.env
chmod 600 ~/.config/"$SERVICE_NAME"/deploy.env

# --- Cleanup of legacy artefacts (idempotent) -----------------------------
echo "  remote: cleaning up legacy tmux sessions if present"
for sess in mtg-server mtg-rust-server; do
    if command -v tmux >/dev/null 2>&1 && tmux has-session -t "$sess" 2>/dev/null; then
        echo "    killing legacy tmux session: $sess"
        tmux kill-session -t "$sess" || true
    fi
done

if [[ "$SYSTEMD_MODE" == "user" ]]; then
    echo "  remote: installing systemd-user unit"
    mkdir -p ~/.config/systemd/user
    mv /tmp/"$SERVICE_NAME".service.staged ~/.config/systemd/user/"$SERVICE_NAME".service
    # Enable lingering so the unit runs even when the user is not logged in.
    if command -v loginctl >/dev/null 2>&1; then
        if ! loginctl show-user "$USER" 2>/dev/null | grep -q '^Linger=yes$'; then
            echo "    enabling user lingering (requires sudo, one-time)"
            sudo -n loginctl enable-linger "$USER" 2>/dev/null \
                || sudo loginctl enable-linger "$USER" \
                || echo "    WARNING: could not enable-linger; service will not auto-start at boot"
        fi
    fi
    systemctl --user daemon-reload
    systemctl --user enable "$SERVICE_NAME".service
    echo "  remote: systemd-user unit installed and enabled"
else
    echo "  remote: installing systemd-system unit (requires sudo)"
    sudo mv /tmp/"$SERVICE_NAME".service.staged /etc/systemd/system/"$SERVICE_NAME".service
    sudo chmod 644 /etc/systemd/system/"$SERVICE_NAME".service
    sudo systemctl daemon-reload
    sudo systemctl enable "$SERVICE_NAME".service

    if [[ "$WITH_SUDOERS" == "1" ]]; then
        echo "  remote: installing sudoers rule for passwordless systemctl restart"
        SUDOERS_LINE="$REMOTE_USER_NAME ALL=(root) NOPASSWD: /bin/systemctl restart $SERVICE_NAME.service, /bin/systemctl status $SERVICE_NAME.service"
        echo "$SUDOERS_LINE" | sudo tee /etc/sudoers.d/"$SERVICE_NAME"-deploy >/dev/null
        sudo chmod 0440 /etc/sudoers.d/"$SERVICE_NAME"-deploy
        sudo visudo -cf /etc/sudoers.d/"$SERVICE_NAME"-deploy
    fi
fi

# Firewall: ufw is the common case on Ubuntu. If not present, just note.
if command -v ufw >/dev/null 2>&1; then
    echo "  remote: opening firewall port $REMOTE_PORT/tcp"
    sudo ufw allow "$REMOTE_PORT"/tcp >/dev/null 2>&1 || \
        echo "    (ufw not active or sudo unavailable; open $REMOTE_PORT manually if needed)"
else
    echo "  remote: ufw not installed; ensure port $REMOTE_PORT/tcp is reachable externally"
fi

# Disable legacy systemd units that may have been left from older deploys.
for legacy in mtg-server mtg-rust-server; do
    if systemctl --user list-unit-files 2>/dev/null | grep -q "^$legacy.service"; then
        echo "  remote: disabling legacy user unit: $legacy.service"
        systemctl --user disable --now "$legacy.service" 2>/dev/null || true
    fi
    if systemctl list-unit-files 2>/dev/null | grep -q "^$legacy.service"; then
        echo "  remote: disabling legacy system unit: $legacy.service"
        sudo systemctl disable --now "$legacy.service" 2>/dev/null || true
    fi
done

echo "  remote: config phase complete"
REMOTE

    echo ""
    echo "✓ config complete. Next step: run \`deploy-cloud.sh deploy\` to push code."
}

# ---------------------------------------------------------------------------
# deploy subcommand
# ---------------------------------------------------------------------------

cmd_deploy() {
    cd "$REPO_ROOT"

    # --- 1. Local WASM build if needed ---
    local need_wasm=0
    if [[ "$SKIP_WASM" != "1" ]]; then
        if [[ ! -d web/data || ! -f web/data/decks.bin || ! -f web/data/sets/index.json || ! -d web/pkg ]]; then
            need_wasm=1
        fi
        [[ "$REBUILD" == "1" ]] && need_wasm=1
    fi
    if (( need_wasm )); then
        echo "→ building WASM artefacts (make wasm-export wasm-network)"
        make wasm-export wasm-network
    else
        echo "→ WASM artefacts present (or --skip-wasm); not rebuilding"
    fi
    for f in web/pkg/mtg_forge_rs_bg.wasm web/data/decks.bin web/data/sets/index.json; do
        [[ -f "$f" ]] || { echo "error: missing required artefact: $f (run 'cargo run --bin mtg -- export-wasm' to (re)generate)" >&2; exit 1; }
    done

    # --- 2. Local native release binary ---
    # Use the slim `release-deploy` profile: strip + lto=fat + panic=abort,
    # produces a ~25 MB binary vs ~430 MB from `release` (which keeps debug
    # symbols for local profiling). Profiles cannot enable features, so we
    # always pass `--features network` explicitly on the build invocation.
    local native_bin="target/release-deploy/mtg"
    if [[ "$SKIP_BUILD" != "1" ]]; then
        if [[ ! -x "$native_bin" || "$REBUILD" == "1" ]]; then
            echo "→ building release-deploy mtg binary (--features network)"
            cargo build --profile release-deploy --bin mtg --features network
        else
            echo "→ $native_bin present; skipping native build (--rebuild to force)"
        fi
    fi
    [[ -x "$native_bin" ]] || {
        echo "error: $native_bin missing. Build with: cargo build --profile release-deploy --bin mtg --features network" >&2
        exit 1
    }

    # --- 3. server-config.js: NO LONGER OVERRIDDEN at deploy time. ---
    # The committed web/server-config.js self-detects the WS URL from
    # window.location ("wss://" + host + "/lobby") which works for every
    # deploy context (local dev, direct VM IP, CF-fronted) without
    # baking in port numbers or protocol assumptions that break behind
    # reverse proxies. Earlier deploys hardcoded "wss://<host>:8080"
    # which broke when CF proxied 443 → origin:8080 (browser tried 8080
    # directly and got SSL errors); see mtg-vevb7.
    echo "→ using committed web/server-config.js (self-detects ws/wss + /lobby path)"

    # --- 4. Pre-flight: check remote layout exists ---
    if ! ssh "$REMOTE_SSH" "[ -d ~/$REMOTE_DIR/bin ]"; then
        cat >&2 <<EOF
error: remote ~/${REMOTE_DIR}/bin does not exist on ${REMOTE_SSH}.
       The VM has not been bootstrapped. Run:
           scripts/deploy-cloud.sh config
       first (idempotent; safe to re-run).
EOF
        exit 1
    fi

    # --- 5. Rsync web/ ---
    echo "→ rsyncing web/"
    rsync -avh --delete \
        --exclude='images/' --exclude='images' \
        --exclude='node_modules/' --exclude='screenshots/' \
        --exclude='server.log' --exclude='*.log' \
        --exclude='package-lock.json' --exclude='test_*.js' \
        --exclude='network_*_test_results.json' \
        web/ "$REMOTE_SSH:$REMOTE_DIR/web/"

    # --- 6. Rsync cardsfolder/ ---
    local cards_src=""
    if [[ -d cardsfolder/. ]]; then
        cards_src="cardsfolder/"
    elif [[ -n "${CARDSFOLDER:-}" && -d "${CARDSFOLDER}" ]]; then
        cards_src="${CARDSFOLDER}/"
    fi
    if [[ -n "$cards_src" ]]; then
        echo "→ rsyncing $cards_src"
        rsync -avh --delete --copy-links "$cards_src" "$REMOTE_SSH:$REMOTE_DIR/cardsfolder/"
    else
        echo "error: no cardsfolder/ available locally; set CARDSFOLDER or init the forge-java submodule" >&2
        exit 1
    fi

    # --- 7. Rsync the native binary ---
    echo "→ rsyncing $native_bin → ~/${REMOTE_DIR}/bin/mtg"
    rsync -avh "$native_bin" "$REMOTE_SSH:$REMOTE_DIR/bin/mtg"

    # --- 8. Restart the service ---
    echo "→ restarting service ${SERVICE_NAME} (systemd-$SYSTEMD_MODE)"
    if [[ "$SYSTEMD_MODE" == "user" ]]; then
        ssh "$REMOTE_SSH" "chmod +x ~/$REMOTE_DIR/bin/mtg && systemctl --user restart $SERVICE_NAME.service"
        # Give it a moment to come up
        sleep 2
        ssh "$REMOTE_SSH" "systemctl --user status $SERVICE_NAME.service --no-pager -n 10" || true
    else
        ssh "$REMOTE_SSH" "chmod +x ~/$REMOTE_DIR/bin/mtg && sudo -n systemctl restart $SERVICE_NAME.service" || {
            echo "warning: sudo restart failed; the service file may need 'config --with-sudoers'." >&2
            echo "         Attempting passworded restart..."
            ssh -t "$REMOTE_SSH" "sudo systemctl restart $SERVICE_NAME.service"
        }
        sleep 2
        ssh "$REMOTE_SSH" "sudo systemctl status $SERVICE_NAME.service --no-pager -n 10" || true
    fi

    local url_scheme="http"
    [[ -n "$TLS_CERT_PATH" ]] && url_scheme="https"
    echo ""
    echo "═════════════════════════════════════════════════════════════════════"
    echo "  ✓ deploy complete"
    echo "═════════════════════════════════════════════════════════════════════"
    echo "  Landing page : ${url_scheme}://${PUBLIC_HOST}/  (CF-proxied; origin port ${REMOTE_PORT})"
    echo "  Lobby WS URL : derived in-browser from window.location → /lobby on same origin"
    echo "  Logs         : scripts/deploy-cloud.sh logs"
    echo "  Status       : scripts/deploy-cloud.sh status"
    echo "═════════════════════════════════════════════════════════════════════"
}

# ---------------------------------------------------------------------------
# status / logs subcommands
# ---------------------------------------------------------------------------

cmd_status() {
    if [[ "$SYSTEMD_MODE" == "user" ]]; then
        ssh "$REMOTE_SSH" "systemctl --user status $SERVICE_NAME.service --no-pager -n 20"
    else
        ssh "$REMOTE_SSH" "sudo systemctl status $SERVICE_NAME.service --no-pager -n 20"
    fi
}

cmd_logs() {
    if [[ "$SYSTEMD_MODE" == "user" ]]; then
        ssh -t "$REMOTE_SSH" "journalctl --user -u $SERVICE_NAME.service -f"
    else
        ssh -t "$REMOTE_SSH" "sudo journalctl -u $SERVICE_NAME.service -f"
    fi
}

# ---------------------------------------------------------------------------
# Dispatch
# ---------------------------------------------------------------------------

case "$CMD" in
    config) cmd_config ;;
    deploy) cmd_deploy ;;
    status) cmd_status ;;
    logs)   cmd_logs ;;
    *) echo "internal error: unhandled subcommand $CMD" >&2; exit 99 ;;
esac
