#!/usr/bin/env bash
# deploy-cloud.sh - Minimal idempotent deploy of the mtg-forge-rs web UI to
# a cloud VM reachable via passwordless SSH.
#
# WHAT IT DOES
#   1. Locally ensures the WASM data export and wasm-pack bundle exist
#      (runs `make wasm-export wasm-network` only if `web/data/` or
#      `web/pkg/` are missing).
#   2. Rsyncs the `web/` directory to the remote, EXCLUDING `images/`
#      (a separate one-time rsync handles those — 4 GB), node_modules,
#      logs, screenshots, and test artefacts.
#   3. Rsyncs the `cardsfolder/` (≈130 MB) to the remote as a reference
#      copy. NOTE: `make wasm-serve` does NOT read cards/ at runtime
#      because the card DB is baked into web/data/cards.bin by
#      wasm-export. The folder is shipped because the task brief asks
#      for it and so a future server-side `make wasm-export` would work.
#   4. Restarts the static web server on the VM inside a detached tmux
#      session named `mtg-server`, serving on $REMOTE_PORT (default
#      8080) via `python3 -m http.server`.
#
# WHAT IT DOES NOT DO
#   - No toolchain install on the VM (no cargo, no wasm-pack).
#   - No remote compilation. All WASM/data artefacts are built locally.
#   - No copy of `web/images/` (an in-flight rsync is/was handling that;
#     re-running this script will leave the remote images/ untouched).
#   - No firewall changes. Port $REMOTE_PORT must already be reachable
#     (the VM currently listens on 22/80/443 only; if external access on
#     8080 is required, open it manually).
#   - No "release binary" copy. `make wasm-serve` is just
#     `python3 -m http.server`; there is no Rust server binary.
#
# REMOTE ASSUMPTIONS
#   - Passwordless SSH to ${REMOTE} works.
#   - `python3` and `tmux` are installed on the VM (Ubuntu 24.04 default
#     has python3; tmux is installed by this script if missing).
#   - Remote home contains writeable `~/mtg-forge-rs/`.
#
# IDEMPOTENCY
#   - Uses rsync, not scp. Re-running only transfers deltas.
#   - The tmux session is killed and restarted on every run so the
#     server picks up new artefacts. Static files are served directly
#     from disk, so the restart is for cleanliness only.
#
# OPERATIONS
#   Start (re-deploy):  ./scripts/deploy-cloud.sh
#   Stop the server:    ssh newton@deepscry.net 'tmux kill-session -t mtg-server'
#   View server log:    ssh newton@deepscry.net 'tmux capture-pane -p -t mtg-server'
#   Attach interactive: ssh -t newton@deepscry.net 'tmux attach -t mtg-server'

set -euo pipefail

REMOTE="${REMOTE:-newton@deepscry.net}"
REMOTE_DIR="${REMOTE_DIR:-mtg-forge-rs}"
REMOTE_PORT="${REMOTE_PORT:-8080}"
TMUX_SESSION="${TMUX_SESSION:-mtg-server}"

# Resolve repo root from this script's location (works from any CWD).
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

echo "=== mtg-forge-rs cloud deploy ==="
echo "Remote:    $REMOTE:~/$REMOTE_DIR"
echo "Port:      $REMOTE_PORT"
echo "Session:   $TMUX_SESSION"
echo "Repo root: $REPO_ROOT"
echo

# --- 1. Build WASM artefacts locally if missing -----------------------------
#
# `make wasm-export` needs cardsfolder. The in-repo `cardsfolder` is a
# symlink into the `forge-java` git submodule, which may not be
# initialised in an agent worktree. As a fallback we let the user (or
# the script) point CARDSFOLDER at the primary checkout's copy.
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
    echo "--- web/data and web/pkg present; skipping local build (set REBUILD=1 to force) ---"
fi

# Sanity check: must have at least the WASM bundle and the card DB.
for f in web/pkg/mtg_forge_rs_bg.wasm web/data/cards.bin web/data/decks.bin; do
    [[ -f "$f" ]] || { echo "ERROR: missing required artefact: $f" >&2; exit 1; }
done

# --- 2. Ensure remote layout and tmux exist ---------------------------------
ssh "$REMOTE" "
    set -e
    mkdir -p ~/$REMOTE_DIR/web ~/$REMOTE_DIR/cardsfolder
    if ! command -v tmux >/dev/null 2>&1; then
        echo 'Installing tmux on remote...'
        sudo -n apt-get install -y tmux || {
            echo 'ERROR: tmux not installed and passwordless sudo unavailable' >&2
            exit 1
        }
    fi
"

# --- 3. Rsync web/ (excluding images and transient state) -------------------
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

# --- 4. Rsync cardsfolder/ (≈130 MB, follow symlink) ------------------------
# `cardsfolder` is a symlink to forge-java/forge-gui/res/cardsfolder/.
# Use --copy-links so the remote gets a plain directory. If the symlink
# is dangling (forge-java submodule not initialised in this worktree),
# fall back to $CARDSFOLDER if set, else skip.
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
    echo "--- cardsfolder/ unavailable; skipping (web/data/cards.bin already contains the baked card DB) ---"
fi

# --- 5. (Re)start the web server in a detached tmux session -----------------
echo "--- (re)starting web server on $REMOTE port $REMOTE_PORT ---"
ssh "$REMOTE" "
    set -e
    tmux kill-session -t $TMUX_SESSION 2>/dev/null || true
    cd ~/$REMOTE_DIR/web
    tmux new-session -d -s $TMUX_SESSION \
        \"python3 -m http.server $REMOTE_PORT 2>&1 | tee ~/$REMOTE_DIR/server.log\"
    sleep 1
    if ! tmux has-session -t $TMUX_SESSION 2>/dev/null; then
        echo 'ERROR: tmux session failed to start' >&2
        exit 1
    fi
    echo 'Server tmux session is up. Listening sockets on port $REMOTE_PORT:'
    ss -tlnp 2>/dev/null | grep ':$REMOTE_PORT' || echo '  (none — check ~/$REMOTE_DIR/server.log)'
"

echo
echo "=== Deploy complete ==="
echo "URL:   http://deepscry.net:$REMOTE_PORT/"
echo "Stop:  ssh $REMOTE 'tmux kill-session -t $TMUX_SESSION'"
echo "Log:   ssh $REMOTE 'tail -f ~/$REMOTE_DIR/server.log'"
