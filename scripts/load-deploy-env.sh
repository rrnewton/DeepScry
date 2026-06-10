# load-deploy-env.sh — shared, SOURCEABLE deploy-config loader.
#
# This is NOT an executable script: `source` it from another bash
# script (deploy-cloud.sh, tests/remote/*.sh, ...) to populate the
# deploy environment variables (REMOTE_USER, REMOTE_HOST, REMOTE_PORT,
# ...) from the local, gitignored `.deepscry-deploy.env` file.
#
# DRY: this is the SINGLE source of the config-file search logic. It
# searches the same three locations in the same order as every caller
# expects:
#   1. $DEPLOY_CONFIG_FILE_OVERRIDE  (if set non-empty — e.g. a --config flag)
#   2. <parent>/.deepscry-deploy.env (the dev harness root; preferred)
#   3. <repo>/.deepscry-deploy.env   (the mtg-forge-rs primary checkout)
#   4. ~/.config/deepscry/deploy.env
#
# Usage (from a sourcing script):
#   # Optionally narrow / override the search:
#   #   DEPLOY_CONFIG_FILE_OVERRIDE=/path/to/file   (force one file)
#   #   DEPLOY_CONFIG_REPO_ROOT=/path/to/repo       (repo root; default: dir above this script)
#   source "$SCRIPT_DIR/load-deploy-env.sh"
#   load_deploy_env || exit 1   # prints an explicit error + returns 1 if no config found
#
# After a successful `load_deploy_env`, the config file has been
# sourced into the current shell, so REMOTE_HOST / REMOTE_USER / etc.
# are available. The function FAILS (non-zero, with a template-fill
# message) when no config file is found OR when the found file does
# not set REMOTE_HOST.

# Resolve this loader's own location so we can compute the default
# repo root and parent dir even when sourced from elsewhere.
_LOAD_DEPLOY_ENV_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Print the canonical "no config" / "missing REMOTE_HOST" error,
# pointing the user at the tracked template. Single definition so the
# message stays identical across every caller.
deploy_env_error() {
    local repo_root="$1"
    cat >&2 <<ERR
ERROR: no deploy config found. Copy scripts/deepscry-deploy.env.example to
       ${repo_root%/}/../.deepscry-deploy.env (the dev harness parent dir, preferred)
       — or ${repo_root%/}/.deepscry-deploy.env, or ~/.config/deepscry/deploy.env —
       and fill in REMOTE_HOST (and REMOTE_USER).
ERR
}

# Search the standard locations, source the first config file found,
# and verify REMOTE_HOST is set. Returns 0 on success, 1 otherwise
# (printing deploy_env_error). The chosen file path is exported as
# DEPLOY_CONFIG_FILE for the caller to log.
load_deploy_env() {
    local repo_root="${DEPLOY_CONFIG_REPO_ROOT:-$(cd "$_LOAD_DEPLOY_ENV_DIR/.." && pwd)}"
    local parent_dir
    parent_dir="$(cd "$repo_root/.." 2>/dev/null && pwd || echo "$repo_root")"

    local search_paths=(
        "${DEPLOY_CONFIG_FILE_OVERRIDE:-}"
        "$parent_dir/.deepscry-deploy.env"
        "$repo_root/.deepscry-deploy.env"
        "$HOME/.config/deepscry/deploy.env"
    )

    DEPLOY_CONFIG_FILE=""
    local p
    for p in "${search_paths[@]}"; do
        [[ -z "$p" ]] && continue
        if [[ -f "$p" ]]; then
            DEPLOY_CONFIG_FILE="$p"
            break
        fi
    done

    if [[ -z "$DEPLOY_CONFIG_FILE" ]]; then
        deploy_env_error "$repo_root"
        return 1
    fi

    # shellcheck disable=SC1090
    source "$DEPLOY_CONFIG_FILE"

    if [[ -z "${REMOTE_HOST:-}" ]]; then
        echo "ERROR: $DEPLOY_CONFIG_FILE is missing REMOTE_HOST." >&2
        deploy_env_error "$repo_root"
        return 1
    fi

    return 0
}

# Source the gitignored OAuth + R2 secret files (`.oauth.env`, `.r2.env`)
# into the current shell, FAIL-SOFT. These hold the login (GitHub/Google)
# and Cloudflare-R2 deck-storage secrets; they are deliberately SEPARATE
# from `.deepscry-deploy.env` (which carries infra config) so the high-
# sensitivity credentials live in their own narrowly-scoped, easily-
# rotated files. Each is OPTIONAL: a missing or empty file leaves the
# corresponding feature disabled (mirrors the server's "disabled if the
# env vars are absent" behaviour) and MUST NOT break the deploy.
#
# Searched in the SAME parent-then-repo order as the main config so the
# operator can keep all three local files together in the dev-harness
# parent dir. After this returns, any vars the files set
# (GITHUB_OAUTH_CLIENT_ID, ..., AWS_ACCESS_KEY_ID, ...) are present in the
# environment for render_env_file() to forward. Sourced files are echoed
# (by path only, never their contents) into DEPLOY_OAUTH_FILE /
# DEPLOY_R2_FILE for the caller to log.
load_deploy_secrets() {
    local repo_root="${DEPLOY_CONFIG_REPO_ROOT:-$(cd "$_LOAD_DEPLOY_ENV_DIR/.." && pwd)}"
    local parent_dir
    parent_dir="$(cd "$repo_root/.." 2>/dev/null && pwd || echo "$repo_root")"

    # Search roots, in order: parent dir (preferred), then repo root. A test
    # harness or an unusual layout can override BOTH with a single explicit
    # directory via DEPLOY_SECRETS_SEARCH_DIR (used by the deploy-env-
    # forwarding test to point at a temp dir holding dummy secret files).
    local -a search_dirs
    if [[ -n "${DEPLOY_SECRETS_SEARCH_DIR:-}" ]]; then
        search_dirs=("$DEPLOY_SECRETS_SEARCH_DIR")
    else
        search_dirs=("$parent_dir" "$repo_root")
    fi

    DEPLOY_OAUTH_FILE=""
    DEPLOY_R2_FILE=""

    local basename file dir
    # Map each secret-file basename to the caller-visible "which file did
    # we source" variable name. Parent dir is preferred; repo root is the
    # fallback (matching the main config search).
    for basename in .oauth.env .r2.env; do
        for dir in "${search_dirs[@]}"; do
            file="$dir/$basename"
            [[ -f "$file" ]] || continue
            # shellcheck disable=SC1090
            source "$file"
            case "$basename" in
                .oauth.env) DEPLOY_OAUTH_FILE="$file" ;;
                .r2.env)    DEPLOY_R2_FILE="$file" ;;
            esac
            break
        done
    done

    return 0
}
