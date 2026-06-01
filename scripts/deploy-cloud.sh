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
    # Reads TRUSTED_BUG_REPORT_PASSWORD from the environment (set by the
    # calling shell, which loaded it from the config file). When unset or
    # empty, the flag is omitted and bug reports are stored as untrusted.
    local mode="$1"
    local user_line=""
    [[ "$mode" == "system" ]] && user_line="User=${REMOTE_USER}"

    # Build the optional --trusted-bug-report-password line.
    # WARNING: never echo the secret to stdout in the success path below.
    local trusted_pw_line=""
    if [[ -n "${TRUSTED_BUG_REPORT_PASSWORD:-}" ]]; then
        trusted_pw_line="    --trusted-bug-report-password \${TRUSTED_BUG_REPORT_PASSWORD} \\"
    fi

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
    --cardsfolder %h/${REMOTE_DIR}/cardsfolder${trusted_pw_line:+ \\
${trusted_pw_line}}
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
    # (TLS paths, trusted bug-report password) and lives in the user's
    # $HOME so no root is needed for either mode. systemd's `%h` expands
    # to $HOME on the remote.
    #
    # TRUSTED_BUG_REPORT_PASSWORD is OPTIONAL. If unset/empty, bug reports
    # are still stored but marked untrusted (no automated GitHub issue filing
    # or Claude autofix). A warning is printed but the deploy continues.
    cat <<ENV
# AUTO-GENERATED by deploy-cloud.sh config. Holds runtime overrides
# for the ${SERVICE_NAME} systemd unit (mtg-forge-rs server-web).
RUST_LOG=info
ENV
    if [[ -n "${TLS_CERT_PATH:-}" ]]; then
        echo "MTG_TLS_CERT=$TLS_CERT_PATH"
    fi
    if [[ -n "${TLS_KEY_PATH:-}" ]]; then
        echo "MTG_TLS_KEY=$TLS_KEY_PATH"
    fi
    if [[ -n "${TRUSTED_BUG_REPORT_PASSWORD:-}" ]]; then
        echo "TRUSTED_BUG_REPORT_PASSWORD=${TRUSTED_BUG_REPORT_PASSWORD}"
    fi
}

# ---------------------------------------------------------------------------
# config subcommand
# ---------------------------------------------------------------------------

cmd_config() {
    echo "→ Bootstrapping remote ($SYSTEMD_MODE mode)..."

    # Warn (but proceed) when no trusted-bug-report password is configured.
    if [[ -z "${TRUSTED_BUG_REPORT_PASSWORD:-}" ]]; then
        echo ""
        echo "⚠  WARNING: TRUSTED_BUG_REPORT_PASSWORD is not set."
        echo "   Bug reports will be stored as UNTRUSTED (no automated GitHub"
        echo "   issue filing or Claude autofix). To enable trusted reports:"
        echo "     1. Add TRUSTED_BUG_REPORT_PASSWORD=<secret> to your"
        echo "        .deepscry-deploy.env config file."
        echo "     2. Re-run: scripts/deploy-cloud.sh config"
        echo "   Proceeding with deploy (untrusted bug reports only)."
        echo ""
    fi

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
    # Enable lingering so the user's systemd manager (and therefore the
    # service) survives logout and reboot. WITHOUT this, the --user
    # manager is torn down when the user's last login session ends, so
    # the server dies the moment the deploy/admin SSH session closes
    # and only restarts on the next login (INCIDENT 2026-05-28: ~48min
    # outage; see mtg-584). enable-linger is idempotent, so we run it
    # unconditionally and then ASSERT Linger=yes — failing loudly if the
    # VM did not honour it (we must not leave a non-durable service).
    if ! command -v loginctl >/dev/null 2>&1; then
        echo "    ERROR: loginctl not found; cannot enable durable lingering for systemd-user mode" >&2
        exit 1
    fi
    echo "    enabling user lingering (idempotent)"
    sudo -n loginctl enable-linger "$USER" 2>/dev/null \
        || sudo loginctl enable-linger "$USER" \
        || { echo "    ERROR: could not enable-linger for $USER; service will NOT survive logout/reboot" >&2; exit 1; }
    if ! loginctl show-user "$USER" 2>/dev/null | grep -q '^Linger=yes$'; then
        echo "    ERROR: linger assertion failed — loginctl show-user $USER does not report Linger=yes" >&2
        exit 1
    fi
    echo "    verified: Linger=yes for $USER (service is durable across logout/reboot)"
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
    # Always rebuild WASM/data on deploy (unless --skip-wasm): a stale web/pkg
    # or web/data from an older SHA would ship mismatched code/assets, and the
    # content-addressed hashed filenames make a stale pkg especially dangerous
    # (silent old-glue / new-wasm mispair). --skip-wasm is the explicit opt-out
    # for when you have deliberately prebuilt current artefacts.
    local need_wasm=0
    [[ "$SKIP_WASM" != "1" ]] && need_wasm=1
    if (( need_wasm )); then
        echo "→ building WASM artefacts (make wasm-export wasm-network)"
        make wasm-export wasm-network
    else
        echo "→ WASM artefacts present (or --skip-wasm); not rebuilding"
    fi
    for f in web/pkg/mtg_engine_bg.wasm web/data/sets/index.json; do
        [[ -f "$f" ]] || { echo "error: missing required artefact: $f (run 'cargo run --bin mtg -- export-wasm' to (re)generate)" >&2; exit 1; }
    done
    # tokens.bin + decks.bin are now content-addressed (tokens+decks cache-skew
    # fix): their hashed names live in index.json, so resolve+verify via the
    # manifest instead of the retired fixed `web/data/decks.bin` path.
    for key in decks tokens; do
        rel="$(python3 -c "import json,sys; print(json.load(open('web/data/sets/index.json'))['$key'])" 2>/dev/null || true)"
        [[ -n "$rel" && -f "web/data/$rel" ]] || {
            echo "error: index.json '$key' bin missing or unresolved (got '${rel:-<none>}'); run 'cargo run --bin mtg -- export-wasm'" >&2
            exit 1
        }
    done

    # --- 2. Local native release binary ---
    # Use the slim `release-deploy` profile: strip + lto=fat + panic=abort,
    # produces a ~25 MB binary vs ~430 MB from `release` (which keeps debug
    # symbols for local profiling). Profiles cannot enable features, so we
    # always pass `--features network` explicitly on the build invocation.
    local native_bin="target/release-deploy/mtg"
    if [[ "$SKIP_BUILD" != "1" ]]; then
        # Always rebuild on deploy: a deploy must ship the CURRENT source's
        # code. A pre-existing target/release-deploy/mtg may be a STALE binary
        # from an older SHA — this once shipped a binary lacking the
        # `hash-web-assets` subcommand and tripped the pre-deploy gate. Use
        # --skip-build only when you have deliberately prebuilt this binary.
        echo "→ building release-deploy mtg binary (--features network)"
        cargo build --profile release-deploy --bin mtg --features network
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
    # directly and got SSL errors); see mtg-478.
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

    # --- 4b. PRE-RSYNC LOCAL SMOKE-TEST GATE (mtg-571) ---
    # Before touching the VM, prove the content-addressed pipeline works
    # end-to-end against a LOCAL `mtg server-web`: stage a hashed copy of
    # web/, serve it on a temp localhost port, and assert index.json is
    # no-cache, the hashed bin/wasm/js are immutable, asset resolution
    # (logical→hashed) works, and a fixed pkg name stays no-cache. This is
    # the same hermetic test wired into `make validate` (no deepscry.net), so
    # a broken pipeline aborts the deploy here instead of shipping a 404-y or
    # mis-cached tree to the VM. Skippable only via --skip-build/--skip-wasm
    # (which would mean assets weren't rebuilt anyway).
    if command -v node >/dev/null 2>&1; then
        echo "→ PRE-DEPLOY GATE: hermetic web-asset smoke test (mtg server-web)"
        ( cd "$REPO_ROOT/web" && MTG_BIN="$REPO_ROOT/$native_bin" node test_web_server_smoke.js ) || {
            echo "✗ PRE-DEPLOY GATE FAILED: web-asset smoke test did not pass; ABORTING deploy (nothing rsynced to the VM)." >&2
            exit 1
        }
        echo "✓ web-asset smoke passed"
    else
        echo "warning: node not found; SKIPPING pre-deploy web-asset smoke test (install Node to enable the gate)" >&2
    fi

    # --- 4c. PRE-RSYNC WASM-BOOT SMOKE (tokens+decks cache-skew fix) ---
    # The web-asset smoke above only checks HTTP status + cache headers; it
    # never boots the WASM and DESERIALIZES tokens.bin + decks.bin + the set
    # bins against the freshly-built glue. That gap let a code-vs-data enum-tag
    # skew ship undetected ("tag for enum is not valid, found 16"). This boot
    # smoke drives the actual WASM build headlessly: loads index.json → resolves
    # + deserializes the content-addressed decks/tokens/set bins → launches a
    # game. Any deserialize error fails LOUDLY and aborts the deploy.
    #
    # Chromium-gated like the e2e steps: if Playwright/Chromium is unavailable
    # in this environment it SKIPS with a loud warning (the same env-gated
    # pattern used by make validate's browser steps) rather than hard-failing.
    if command -v python3 >/dev/null 2>&1 && python3 -c 'import playwright' >/dev/null 2>&1; then
        echo "→ PRE-DEPLOY GATE: headless WASM-boot smoke (deserialize tokens+decks+sets, launch a game)"
        local boot_deck="decks/old_school2/the_deck_classic.dck"
        [[ -f "$boot_deck" ]] || boot_deck="$(ls decks/*.dck decks/**/*.dck 2>/dev/null | head -n1)"
        if python3 scripts/mtg_wasm_game.py --p1 random --p2 random --seed 42 --max-turns 3 \
                --out-dir "$(mktemp -d)/wasm_boot_smoke" "$boot_deck"; then
            echo "✓ WASM-boot smoke passed (tokens+decks+sets deserialized, game launched)"
        else
            echo "✗ PRE-DEPLOY GATE FAILED: WASM-boot smoke could not deserialize bins / launch a game; ABORTING deploy." >&2
            exit 1
        fi
    else
        echo "warning: python3+playwright not available; SKIPPING WASM-boot smoke (install: pip install playwright && playwright install chromium)" >&2
    fi

    # --- 5. Rsync web/ (content-addressed, mtg-571) ---
    # Stage web/ into a temp dir, then CONTENT-ADDRESS the wasm-bindgen pkg
    # pair there via `mtg hash-web-assets`: mtg_engine.js +
    # mtg_engine_bg.wasm are renamed to `<name>.<hash>.<ext>` and the HTML
    # import specifier + init({module_or_path}) arg are rewritten to match.
    # This SUPERSEDES the old `?v=<sha>` query-string cache-bust: a query
    # string still shared one filename (so a CDN ignoring `no-cache` could
    # still pair old glue with new wasm); a content-addressed FILENAME makes
    # that pairing impossible (mtg-475 / mtg-2indh). The .bin data files are
    # already content-addressed by the exporter (their hashed names live in
    # sets/index.json), so they need no staging rewrite here.
    local BUILD_SHA
    BUILD_SHA="$(git rev-parse --short HEAD 2>/dev/null || echo unknown)"
    if ! git diff --quiet 2>/dev/null || ! git diff --cached --quiet 2>/dev/null; then
        BUILD_SHA="${BUILD_SHA}+dirty"
    fi
    echo "→ build SHA: $BUILD_SHA"

    local web_stage
    web_stage="$(mktemp -d)"
    # Bake the path into the trap NOW (double-quotes): web_stage is `local`, so
    # a deferred '$web_stage' would be unbound at EXIT under `set -u` and make
    # the script exit 1 AFTER a successful deploy (lying exit code).
    trap "rm -rf '$web_stage'" EXIT
    # Copy the whole web/ to staging (cheap; web/ ≈ a few hundred MB
    # with images so we exclude those first). dist/ is trunk's output dir
    # and must not be shipped raw.
    rsync -a \
        --exclude='images/' --exclude='images' \
        --exclude='node_modules/' --exclude='screenshots/' \
        --exclude='dist/' --exclude='dist' \
        --exclude='server.log' --exclude='*.log' \
        --exclude='package-lock.json' --exclude='test_*.js' \
        --exclude='network_*_test_results.json' \
        web/ "$web_stage/"
    # Content-address the pkg pair on the staging copy (NOT the source tree,
    # so the committed HTML + `make validate` e2e tests stay on the fixed
    # path). Renames pkg/*.{js,wasm} -> hashed names and rewrites the HTML.
    # This is now a RUST subcommand (`mtg hash-web-assets`) — the shell
    # `hash_web_assets.sh` was retired (mtg-571) so the WHOLE content-addressed
    # pipeline (per-set bins + pkg pair) lives in one Rust path with one hash
    # implementation (blake3, asset_hash::asset_hash_hex). $native_bin was just
    # built above and carries the subcommand.
    echo "→ content-addressing pkg bundle (mtg hash-web-assets)"
    "$native_bin" hash-web-assets "$web_stage"
    # GC mark-sweep of orphaned hashed data bins: the exporter writes each
    # bin once per content hash, but if a previous export left a stale
    # <set>.<oldhash>.bin in web/data/sets that the CURRENT index.json no
    # longer references, drop it from the staging copy so `rsync --delete`
    # prunes it from the VM (and it never gets re-uploaded). index.json is
    # the authoritative manifest of live bin names.
    local sets_dir="$web_stage/data/sets"
    if [[ -f "$sets_dir/index.json" ]]; then
        echo "→ GC: sweeping orphaned hashed bins not referenced by index.json"
        python3 - "$sets_dir" <<'PYGC'
import json, os, sys
sets_dir = sys.argv[1]
with open(os.path.join(sets_dir, "index.json")) as fh:
    idx = json.load(fh)
live = {s["file"] for s in idx.get("sets", [])}
live.update(idx.get("cards", {}).values())
removed = 0
for name in os.listdir(sets_dir):
    if name.endswith(".bin") and name not in live:
        os.remove(os.path.join(sets_dir, name))
        removed += 1
print(f"    removed {removed} orphaned bin(s); {len(live)} live")
PYGC
    fi
    # rsync --delete propagates BOTH the renamed pkg files and the GC sweep
    # to the VM: any old hashed pkg/bin name the new HTML/index.json no
    # longer references is removed from the remote. This is the deploy-side
    # GC story — `trunk build` (once fully adopted) rebuilds dist/ clean and
    # rsync --delete prunes; today the staging copy + manifest sweep + delete
    # achieve the same pruning.
    echo "→ rsyncing web/ (content-addressed) with --delete"
    # NON-DESTRUCTIVE to web/images/ (mtg image-deploy policy): card-image
    # JPEGs are NOT part of the normal deploy (they are excluded from the
    # web_stage population above and from version control). They are pushed to
    # the VM out-of-band via a one-time manual rsync. Without the exclude here,
    # this --delete rsync from an images-less staging dir would WIPE the remote
    # web/images/ tree on every deploy. The exclude makes --delete skip that
    # directory entirely: every OTHER web/ dir is still destructively synced
    # (hashed pkg/bin GC works), but web/images/ on the VM is left untouched.
    rsync -avh --delete \
        --exclude='images/' --exclude='images' \
        "$web_stage/" "$REMOTE_SSH:$REMOTE_DIR/web/"

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
        # `systemctl --user` over a non-interactive SSH session has no
        # DBus/user-manager handle unless XDG_RUNTIME_DIR is set — otherwise
        # it fails "Failed to connect to bus: No medium found" (the user
        # lingers, the runtime dir exists, the env var just isn't exported).
        ssh "$REMOTE_SSH" "export XDG_RUNTIME_DIR=/run/user/\$(id -u); chmod +x ~/$REMOTE_DIR/bin/mtg && systemctl --user restart $SERVICE_NAME.service"
        # Give it a moment to come up
        sleep 2
        ssh "$REMOTE_SSH" "export XDG_RUNTIME_DIR=/run/user/\$(id -u); systemctl --user status $SERVICE_NAME.service --no-pager -n 10" || true
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

    # --- 9. Post-deploy HTTP probe ---
    # Verifies the freshly-restarted server is actually serving the new
    # bundle. Any FAIL aborts non-zero so CI / human operators notice.
    run_post_deploy_probe "$url_scheme" "$PUBLIC_HOST" "$REMOTE_PORT" "$BUILD_SHA"

    echo ""
    echo "═════════════════════════════════════════════════════════════════════"
    echo "  ✓ deploy complete"
    echo "═════════════════════════════════════════════════════════════════════"
    echo "  Landing page : ${url_scheme}://${PUBLIC_HOST}:${REMOTE_PORT}/"
    echo "  Lobby WS URL : derived in-browser from window.location → /lobby on same origin"
    echo "  Health JSON  : ${url_scheme}://${PUBLIC_HOST}:${REMOTE_PORT}/health"
    echo "  Build SHA    : $BUILD_SHA"
    echo "  Logs         : scripts/deploy-cloud.sh logs"
    echo "  Status       : scripts/deploy-cloud.sh status"
    echo "═════════════════════════════════════════════════════════════════════"
}

# ---------------------------------------------------------------------------
# Post-deploy HTTP probe
# ---------------------------------------------------------------------------
#
# Verifies the freshly-deployed server actually serves what we expect:
#   - landing page returns 200
#   - /pkg/mtg_engine.js is present and reasonable size
#   - /pkg/mtg_engine_bg.wasm is present and reasonable size
#   - /data/sets/index.json parses as JSON
#   - /health returns 200 with our build sha
#   - /lobby responds to a WS upgrade attempt (101 expected; 400 on
#     a malformed Upgrade is also acceptable — both mean "listening")
#
# Any failure exits the script non-zero with an actionable message.

run_post_deploy_probe() {
    local scheme="$1" host="$2" port="$3" expected_sha="$4"
    local base="${scheme}://${host}:${port}"

    echo ""
    echo "→ probing live deploy: $base"

    # TLS trust: we probe the ORIGIN directly (scheme://host:port), not
    # through the Cloudflare proxy. With the CF proxy off, the origin
    # presents the Cloudflare Origin Cert, whose CA is NOT in the system
    # bundle, so a strict curl fails (60) "unable to get local issuer
    # certificate" and reports HTTP 000 for EVERY endpoint — a false
    # failure even when the service is healthy (mtg-581). Use -k to skip
    # TLS-trust verification for the direct-origin probe; the content
    # assertions below (byte sizes, JSON validity, /health sha) are what
    # actually prove the deploy is good, not the cert chain.
    local curl_opts=(-sSk --max-time 15 --retry 3 --retry-delay 2 --retry-connrefused)

    local probe_failed=0
    local fail_reasons=()

    # 1. Landing page.
    local code
    code="$(curl -o /dev/null -w '%{http_code}' "${curl_opts[@]}" "$base/")" || code="000"
    if [[ "$code" != "200" ]]; then
        probe_failed=1; fail_reasons+=("landing page: HTTP $code")
    else
        echo "  ✓ /                          200"
    fi

    # 2+3. Content-addressed pkg pair (mtg-571 + mtg-620). The pkg filenames
    # AND the game-page HTML names are now hashed by `mtg hash-web-assets`.
    # Only `index.html` keeps a stable URL — every other reachable asset is
    # discovered by chasing references from there, exactly like a browser.
    # We:
    #   - fetch / (index.html), discover the hashed tui_game.<h>.html name,
    #   - fetch that, and from its content discover the hashed pkg pair JS +
    #     wasm, plus the hashed data/sets/index.<h>.json,
    #   - probe each hashed URL.
    local landing_html game_html hashed_tui hashed_js hashed_wasm hashed_data_idx
    landing_html="$(curl "${curl_opts[@]}" "$base/")" || landing_html=""
    hashed_tui="$(printf '%s' "$landing_html" | grep -oE "tui_game\.[0-9a-f]+\.html" | head -1)"
    if [[ -z "$hashed_tui" ]]; then
        probe_failed=1; fail_reasons+=("index.html: no hashed tui_game.<h>.html reference (mtg hash-web-assets did not run?)")
    fi

    # 2b. Lobby-redo NAV hardening (mtg-682). The lobby-redo deploy break was a
    # 404 on the launcher hub + the game-page cross-nav once HTML was hashed.
    # Probe the FULL nav chain a browser walks, resolving cycle-edge links
    # through the served runtime manifest exactly like asset_manifest.js does:
    #   - index.html must reference a HASHED launcher.<h>.html and it must 200;
    #   - the launcher's HASHED deck_editor.<h>.html forward link must 200;
    #   - the lobby_launcher.<h>.js redirect builder's LOGICAL game-page names
    #     must resolve (through the manifest) to a hashed 200.
    local hashed_launcher manifest_json nav_code
    hashed_launcher="$(printf '%s' "$landing_html" | grep -oE "launcher\.[0-9a-f]+\.html" | head -1)"
    if [[ -z "$hashed_launcher" ]]; then
        probe_failed=1; fail_reasons+=("index.html: no HASHED launcher.<h>.html reference (auto-discovery / lobby-redo break)")
    else
        nav_code="$(curl -o /dev/null -w '%{http_code}' "${curl_opts[@]}" "$base/$hashed_launcher")" || nav_code="000"
        if [[ "$nav_code" != "200" ]]; then
            probe_failed=1; fail_reasons+=("/$hashed_launcher: HTTP $nav_code (launcher hub must resolve hashed)")
        else
            echo "  ✓ /$hashed_launcher  200 (launcher hub)"
            # The launcher's forward link to the hashed deck editor must 200.
            local launcher_html hashed_deck
            launcher_html="$(curl "${curl_opts[@]}" "$base/$hashed_launcher")" || launcher_html=""
            hashed_deck="$(printf '%s' "$launcher_html" | grep -oE "deck_editor\.[0-9a-f]+\.html" | head -1)"
            if [[ -n "$hashed_deck" ]]; then
                nav_code="$(curl -o /dev/null -w '%{http_code}' "${curl_opts[@]}" "$base/$hashed_deck")" || nav_code="000"
                if [[ "$nav_code" != "200" ]]; then
                    probe_failed=1; fail_reasons+=("/$hashed_deck: HTTP $nav_code (launcher→deck_editor must resolve)")
                else
                    echo "  ✓ /$hashed_deck  200 (launcher→deck editor)"
                fi
            fi
        fi
    fi
    # The runtime manifest must map the game pages to their hashed names, and
    # those names must 200 (the cycle-edge resolution the lobby redirect uses).
    manifest_json="$(curl "${curl_opts[@]}" "$base/asset-manifest.json")" || manifest_json=""
    local nav_page hashed_nav
    for nav_page in tui_game native_game; do
        # Pull "<nav_page>.html": "<nav_page>.<h>.html" out of the manifest JSON.
        hashed_nav="$(printf '%s' "$manifest_json" | grep -oE "${nav_page}\.[0-9a-f]+\.html" | head -1)"
        if [[ -z "$hashed_nav" ]]; then
            probe_failed=1; fail_reasons+=("asset-manifest.json: no ${nav_page}.html → hashed mapping (cycle-edge resolver missing)")
        else
            nav_code="$(curl -o /dev/null -w '%{http_code}' "${curl_opts[@]}" "$base/$hashed_nav")" || nav_code="000"
            if [[ "$nav_code" != "200" ]]; then
                probe_failed=1; fail_reasons+=("/$hashed_nav: HTTP $nav_code (manifest-resolved game page must 200)")
            else
                echo "  ✓ /$hashed_nav  200 (manifest: ${nav_page}.html)"
            fi
        fi
    done

    game_html="$(curl "${curl_opts[@]}" "$base/$hashed_tui")" || game_html=""
    hashed_js="$(printf '%s' "$game_html" | grep -oE "pkg/mtg_engine\.[0-9a-f]+\.js" | head -1)"
    hashed_wasm="$(printf '%s' "$game_html" | grep -oE "pkg/mtg_engine_bg\.[0-9a-f]+\.wasm" | head -1)"
    hashed_data_idx="$(printf '%s' "$game_html" | grep -oE "data/sets/index\.[0-9a-f]+\.json" | head -1)"
    if [[ -z "$hashed_js" ]]; then
        probe_failed=1; fail_reasons+=("$hashed_tui: no hashed pkg JS import found")
    else
        local glue_size
        glue_size="$(curl -o /dev/null -w '%{size_download}' "${curl_opts[@]}" "$base/$hashed_js")" || glue_size=0
        if (( glue_size < 50000 )); then
            probe_failed=1; fail_reasons+=("/$hashed_js: $glue_size bytes (expected > 50000)")
        else
            echo "  ✓ /$hashed_js   200 ($glue_size bytes)"
        fi
    fi
    if [[ -z "$hashed_wasm" ]]; then
        probe_failed=1; fail_reasons+=("$hashed_tui: no hashed pkg wasm (init module_or_path) found")
    else
        local wasm_size
        wasm_size="$(curl -o /dev/null -w '%{size_download}' "${curl_opts[@]}" "$base/$hashed_wasm")" || wasm_size=0
        if (( wasm_size < 1000000 )); then
            probe_failed=1; fail_reasons+=("/$hashed_wasm: $wasm_size bytes (expected > 1000000)")
        else
            echo "  ✓ /$hashed_wasm   200 ($wasm_size bytes)"
        fi
    fi

    # 4. hashed data/sets/index.<h>.json — must parse as JSON, AND its first
    # listed content-addressed bin must be fetchable under its hashed name
    # (verifies the manifest->bin content-addressing landed end-to-end on the
    # VM). After mtg-620 the index.json itself is content-addressed and the
    # fixed `/data/sets/index.json` URL 404s — its hashed name was discovered
    # from the rewritten game HTML above.
    local idx_json
    if [[ -z "$hashed_data_idx" ]]; then
        probe_failed=1; fail_reasons+=("$hashed_tui: no hashed data/sets/index.<h>.json reference")
        idx_json=""
    else
        idx_json="$(curl "${curl_opts[@]}" "$base/$hashed_data_idx")" || idx_json=""
    fi
    local first_bin
    first_bin="$(printf '%s' "$idx_json" | python3 -c 'import json,sys
try:
    d=json.load(sys.stdin); print(d["sets"][0]["file"])
except Exception:
    pass' 2>/dev/null || echo "")"
    if [[ -z "$first_bin" ]]; then
        probe_failed=1; fail_reasons+=("/$hashed_data_idx: not valid JSON or no sets[]")
    else
        echo "  ✓ /$hashed_data_idx      200 (valid JSON)"
        local bin_size
        bin_size="$(curl -o /dev/null -w '%{size_download}' "${curl_opts[@]}" "$base/data/sets/$first_bin")" || bin_size=0
        if (( bin_size < 1000 )); then
            probe_failed=1; fail_reasons+=("/data/sets/$first_bin: $bin_size bytes (expected > 1000)")
        else
            echo "  ✓ /data/sets/$first_bin (hashed)  200 ($bin_size bytes)"
        fi
    fi

    # 5. /health — JSON sha must match.
    local health_json
    health_json="$(curl "${curl_opts[@]}" "$base/health")" || health_json=""
    local served_sha
    served_sha="$(echo "$health_json" | python3 -c 'import json,sys; d=json.load(sys.stdin); print(d.get("sha","?"))' 2>/dev/null || echo "?")"
    if [[ "$served_sha" == "?" || -z "$served_sha" ]]; then
        probe_failed=1; fail_reasons+=("/health: no sha in response: $health_json")
    elif [[ "$served_sha" != "$expected_sha" ]]; then
        # Not fatal — the running build might predate the binary on disk
        # if systemd restart was slow — but flag it loudly.
        echo "  ⚠ /health sha mismatch: served=$served_sha local=$expected_sha"
    else
        echo "  ✓ /health                    200 (sha=$served_sha)"
    fi

    # 6. /lobby — WebSocket probe. We can't easily do a full handshake
    # in bash; instead send a malformed Upgrade request and accept any
    # response other than 5xx / connection-refused. A real WS server
    # answers 426 / 400 to a non-WS GET on this path.
    code="$(curl -o /dev/null -w '%{http_code}' "${curl_opts[@]}" -H "Connection: Upgrade" -H "Upgrade: websocket" "$base/lobby")" || code="000"
    case "$code" in
        101|400|426|405|404) echo "  ✓ /lobby                     responding (HTTP $code)" ;;
        000) probe_failed=1; fail_reasons+=("/lobby: connection refused") ;;
        5*)  probe_failed=1; fail_reasons+=("/lobby: server error HTTP $code") ;;
        *)   echo "  ⚠ /lobby                     HTTP $code (unusual but proceeding)" ;;
    esac

    if (( probe_failed )); then
        echo ""
        echo "✗ POST-DEPLOY PROBE FAILED:"
        for r in "${fail_reasons[@]}"; do
            echo "    - $r"
        done
        echo ""
        echo "  Service may still be coming up. Inspect with:"
        echo "    scripts/deploy-cloud.sh logs"
        echo "    scripts/deploy-cloud.sh status"
        exit 1
    fi
    echo "  ✓ all probes passed"
}

# ---------------------------------------------------------------------------
# status / logs subcommands
# ---------------------------------------------------------------------------

cmd_status() {
    if [[ "$SYSTEMD_MODE" == "user" ]]; then
        ssh "$REMOTE_SSH" "export XDG_RUNTIME_DIR=/run/user/\$(id -u); systemctl --user status $SERVICE_NAME.service --no-pager -n 20"
    else
        ssh "$REMOTE_SSH" "sudo systemctl status $SERVICE_NAME.service --no-pager -n 20"
    fi
}

cmd_logs() {
    if [[ "$SYSTEMD_MODE" == "user" ]]; then
        ssh -t "$REMOTE_SSH" "export XDG_RUNTIME_DIR=/run/user/\$(id -u); journalctl --user -u $SERVICE_NAME.service -f"
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
