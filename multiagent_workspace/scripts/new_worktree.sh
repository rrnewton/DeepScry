#!/usr/bin/env bash
# new_worktree.sh — official entry point for creating a new mtg-forge-rs
#                   worktree. Consolidates the original
#                   scripts/new_worktree.sh (parent) and the older
#                   scripts/clone_worktree.sh (in-repo) into a single
#                   script with both behaviours.
#
# This is the ONLY supported way to create a worktree for child agents.
# Direct `git worktree add` is forbidden — it skips the optimisations
# below and leaves the new worktree with no build cache, costing ~90s of
# cold compile time.
#
# What this script does:
#
#   1. Refresh the SOURCE checkout (default: the primary at
#      <parent>/mtg-forge-rs/) — fetch origin, ensure it has a green
#      release build with --features network. The donor checkout is the
#      source-of-truth target/ donor.
#
#   2. Garbage-collect the source's target/ to keep the donor lean:
#        - cargo sweep --time 14   (drop artifacts older than 14 days)
#        - cargo sweep --installed (drop artifacts from uninstalled toolchains)
#
#   3. git worktree add a new worktree at <parent>/worktrees/<branch>/
#      with the requested branch, based off the requested base
#      (default origin/integration).
#
#   4. cp -a --reflink=auto <source>/target → new-worktree/target. On
#      reflink-capable filesystems (btrfs/xfs/zfs/apfs) this is a
#      copy-on-write clone in milliseconds, costing zero new disk space
#      until cargo overwrites individual artifacts.
#
#   5. Initialise submodules in the new worktree so `make validate` does
#      not bail with "Submodule changes detected". Two submodules:
#        - forge-java (~589 MB, 58k files): reflink-cloned from the
#          source's working tree, then its `.git` pointer is rewritten
#          to absolute path of the SHARED .git/modules/forge-java under
#          the primary repo. Footgun acknowledged: the shared modules
#          dir means HEAD/index for forge-java is SHARED across every
#          worktree. This is acceptable because forge-java is a frozen
#          reference of the Java upstream pinned to one SHA — all
#          worktrees normally pin the same SHA. If you intentionally
#          bump the forge-java pin in a worktree, ALL other worktrees
#          will see the new HEAD. If that becomes a problem, fall back
#          to `git submodule update --init forge-java` (a fresh per-
#          worktree clone, ~10s slower, ~543 MB more disk).
#        - .claude_template (~224 KB): plain
#          `git submodule update --init` (fast, per-worktree gitdir).
#
# Usage:
#   ./scripts/new_worktree.sh <branch-name>
#   ./scripts/new_worktree.sh <branch-name> --base origin/integration
#   ./scripts/new_worktree.sh <branch-name> --base <ref>
#   ./scripts/new_worktree.sh <branch-name> --source <other-worktree>
#   ./scripts/new_worktree.sh <branch-name> --no-build      # skip donor green-build
#   ./scripts/new_worktree.sh <branch-name> --no-sweep      # skip cargo sweep
#
# Examples:
#   ./scripts/new_worktree.sh fix-mana-burn
#   ./scripts/new_worktree.sh feature/network-v3 --base origin/main
#   ./scripts/new_worktree.sh quick-experiment --source ./worktrees/other-branch --no-build
#
# Must be run from the PARENT directory (the directory that contains
# mtg-forge-rs/ and worktrees/).

set -euo pipefail

# ---------------------------------------------------------------------------
# Argument parsing
# ---------------------------------------------------------------------------

BRANCH=""
BASE="origin/integration"
SOURCE_OVERRIDE=""
DO_BUILD=1
DO_SWEEP=1

usage() {
    sed -n '2,/^$/p' "$0" | sed 's/^# \{0,1\}//' >&2
}

while [ $# -gt 0 ]; do
    case "$1" in
        -h|--help) usage; exit 0 ;;
        --base) BASE="$2"; shift 2 ;;
        --base=*) BASE="${1#--base=}"; shift ;;
        --source) SOURCE_OVERRIDE="$2"; shift 2 ;;
        --source=*) SOURCE_OVERRIDE="${1#--source=}"; shift ;;
        --no-build) DO_BUILD=0; shift ;;
        --no-sweep) DO_SWEEP=0; shift ;;
        -*) echo "error: unknown flag: $1" >&2; usage; exit 2 ;;
        *)
            if [ -z "$BRANCH" ]; then
                BRANCH="$1"; shift
            else
                echo "error: unexpected positional arg: $1" >&2; usage; exit 2
            fi
            ;;
    esac
done

if [ -z "$BRANCH" ]; then
    echo "error: branch name is required" >&2
    usage
    exit 2
fi

# ---------------------------------------------------------------------------
# Locate the parent dir + source checkout
# ---------------------------------------------------------------------------

# This script lives at <kit>/scripts/new_worktree.sh, but is normally
# invoked through a symlink at <parent>/scripts/new_worktree.sh. We
# derive PARENT_DIR from the symlink path (NOT the realpath) so we
# operate on the directory the user actually runs from.
INVOKED_SCRIPT="$0"
SCRIPT_DIR="$(cd "$(dirname "$INVOKED_SCRIPT")" && pwd)"
PARENT_DIR="$(dirname "$SCRIPT_DIR")"
PRIMARY="$PARENT_DIR/mtg-forge-rs"

if [ -n "$SOURCE_OVERRIDE" ]; then
    if [ ! -d "$SOURCE_OVERRIDE" ]; then
        echo "error: --source dir does not exist: $SOURCE_OVERRIDE" >&2
        exit 1
    fi
    SOURCE="$(cd "$SOURCE_OVERRIDE" && pwd)"
else
    SOURCE="$PRIMARY"
fi

if [ ! -d "$SOURCE/.git" ] && [ ! -f "$SOURCE/.git" ]; then
    echo "error: source checkout not found at $SOURCE" >&2
    echo "       (expected a git repo or worktree pointer there)" >&2
    exit 1
fi

# Strip refs/heads/ if user supplied a fully-qualified branch name
SAFE_BRANCH="${BRANCH#refs/heads/}"
# Replace slashes with dashes for the worktree directory name only.
SUFFIX="$(echo "$SAFE_BRANCH" | tr '/' '-')"
WORKTREES_DIR="$PARENT_DIR/worktrees"
NEW_WORKTREE="$WORKTREES_DIR/$SUFFIX"

mkdir -p "$WORKTREES_DIR"

# ---------------------------------------------------------------------------
# Pre-flight checks
# ---------------------------------------------------------------------------

if [ -e "$NEW_WORKTREE" ]; then
    echo "error: $NEW_WORKTREE already exists." >&2
    echo "       Pick a different branch name, or remove the existing worktree:" >&2
    echo "         git -C $SOURCE worktree remove $NEW_WORKTREE" >&2
    exit 1
fi

# If the branch already exists in the source's repo, refuse — the user
# almost certainly meant to attach a fresh branch. Existing-branch
# attach can be done manually with `git worktree add <path> <branch>`
# once they're sure.
if git -C "$SOURCE" rev-parse --verify --quiet "refs/heads/$SAFE_BRANCH" >/dev/null; then
    echo "error: branch '$SAFE_BRANCH' already exists in $SOURCE." >&2
    echo "       Either pick a different branch name, or attach it manually:" >&2
    echo "         git -C $SOURCE worktree add $NEW_WORKTREE $SAFE_BRANCH" >&2
    exit 1
fi

echo "═════════════════════════════════════════════════════════════════════"
echo "  new_worktree.sh"
echo "═════════════════════════════════════════════════════════════════════"
echo "  Source checkout : $SOURCE"
echo "  New worktree    : $NEW_WORKTREE"
echo "  New branch      : $SAFE_BRANCH"
echo "  Base            : $BASE"
echo "  Donor build     : $([ $DO_BUILD -eq 1 ] && echo enabled || echo SKIPPED)"
echo "  Donor sweep     : $([ $DO_SWEEP -eq 1 ] && echo enabled || echo SKIPPED)"
echo "═════════════════════════════════════════════════════════════════════"
echo ""

# ---------------------------------------------------------------------------
# Step 1: refresh source
# ---------------------------------------------------------------------------

echo "→ [1/6] git fetch origin (in source)"
git -C "$SOURCE" fetch origin

# Verify the requested base actually resolves now that we've fetched.
if ! git -C "$SOURCE" rev-parse --verify --quiet "$BASE" >/dev/null; then
    echo "error: base '$BASE' did not resolve in $SOURCE after fetch." >&2
    exit 1
fi

# ---------------------------------------------------------------------------
# Step 2: clean release build in source
# ---------------------------------------------------------------------------

if [ $DO_BUILD -eq 1 ]; then
    echo ""
    echo "→ [2/6] cargo build --release --features network (in source)"
    echo "       (this populates the donor target/ that the new worktree will reflink)"
    BUILD_START=$(date +%s)
    if ! ( cd "$SOURCE" && cargo build --release --features network ); then
        echo "error: source release build failed." >&2
        echo "       The donor target/ would be incomplete — refusing to create worktree." >&2
        echo "       (re-run with --no-build to bypass if you really know what you're doing.)" >&2
        exit 1
    fi
    BUILD_END=$(date +%s)
    echo "       source release build OK in $((BUILD_END - BUILD_START))s"
else
    echo ""
    echo "→ [2/6] SKIPPED (--no-build) — donor target/ may be stale or broken"
fi

# ---------------------------------------------------------------------------
# Step 3: garbage-collect source's target/
# ---------------------------------------------------------------------------

if [ $DO_SWEEP -eq 1 ]; then
    echo ""
    echo "→ [3/6] cargo sweep --time 14 (drop artifacts >14 days old)"
    if command -v cargo-sweep >/dev/null 2>&1; then
        ( cd "$SOURCE" && cargo sweep --time 14 ) || \
            echo "       warning: cargo sweep --time 14 returned non-zero, continuing"
        echo ""
        echo "→ [3b/6] cargo sweep --installed (drop artifacts from uninstalled toolchains)"
        ( cd "$SOURCE" && cargo sweep --installed ) || \
            echo "       warning: cargo sweep --installed returned non-zero, continuing"
    else
        echo "       warning: cargo-sweep not installed — skipping cleanup."
        echo "                Install with: cargo install cargo-sweep"
    fi
else
    echo ""
    echo "→ [3/6] SKIPPED (--no-sweep) — donor target/ not GC'd"
fi

# ---------------------------------------------------------------------------
# Step 4: create the worktree
# ---------------------------------------------------------------------------

echo ""
echo "→ [4/6] git worktree add $NEW_WORKTREE -b $SAFE_BRANCH $BASE"
git -C "$SOURCE" worktree add "$NEW_WORKTREE" -b "$SAFE_BRANCH" "$BASE"

# ---------------------------------------------------------------------------
# Step 5: CoW-clone target/
# ---------------------------------------------------------------------------

echo ""
echo "→ [5/6] cp -a --reflink=auto $SOURCE/target $NEW_WORKTREE/target"
if [ -d "$SOURCE/target" ]; then
    FSTYPE=$(stat -fc '%T' "$SOURCE/target" 2>/dev/null || echo "unknown")
    case "$FSTYPE" in
        btrfs|xfs|zfs|apfs)
            REFLINK_STATUS="enabled (filesystem: $FSTYPE)" ;;
        *)
            REFLINK_STATUS="best-effort (filesystem: $FSTYPE — may fall back to full copy)" ;;
    esac
    COPY_START=$(date +%s)
    cp -a --reflink=auto "$SOURCE/target" "$NEW_WORKTREE/target"
    COPY_END=$(date +%s)
    TARGET_SIZE=$(du -sh "$NEW_WORKTREE/target" 2>/dev/null | awk '{print $1}')
    COPY_ELAPSED=$((COPY_END - COPY_START))
    echo "       reflink: $REFLINK_STATUS"
    echo "       cloned $TARGET_SIZE of target/ in ${COPY_ELAPSED}s"
else
    echo "       warning: $SOURCE/target does not exist; new worktree starts cold."
    REFLINK_STATUS="n/a"
    TARGET_SIZE="0"
    COPY_ELAPSED=0
fi

# ---------------------------------------------------------------------------
# Step 6: initialise submodules
# ---------------------------------------------------------------------------
#
# Without this, `make validate` aborts immediately with "Submodule
# changes detected" because validate.sh treats uninitialised submodules
# as a dirty working copy.

echo ""
echo "→ [6/6] initialise submodules in new worktree"

# 6a. .claude_template — small, just init normally.
if [ -f "$NEW_WORKTREE/.gitmodules" ]; then
    if grep -q '\.devcontainer\|\.claude_template' "$NEW_WORKTREE/.gitmodules" 2>/dev/null; then
        echo "       initialising .claude_template (plain submodule update)"
        ( cd "$NEW_WORKTREE" && git submodule update --init .claude_template ) || \
            echo "       warning: .claude_template init returned non-zero, continuing"
    fi
fi

# 6b. forge-java — heavy, reflink the working tree from source and
#     point its .git file at the SHARED .git/modules/forge-java under
#     the primary repo. See header for the footgun discussion.
if [ -d "$SOURCE/forge-java" ] && [ -e "$SOURCE/forge-java/.git" ]; then
    echo "       reflink-cloning forge-java from $SOURCE/forge-java"
    FJ_COPY_START=$(date +%s)
    # Remove the empty placeholder dir that `git worktree add` created
    # for the submodule path (cp -a refuses to merge into a non-empty
    # destination cleanly if the placeholder has any contents).
    rm -rf "$NEW_WORKTREE/forge-java"
    cp -a --reflink=auto "$SOURCE/forge-java" "$NEW_WORKTREE/forge-java"
    FJ_COPY_END=$(date +%s)

    # Resolve the SHARED git-common-dir of the source (this is the
    # primary repo's real `.git` dir even when SOURCE is itself a
    # worktree; git-common-dir always returns the main repo's gitdir).
    SHARED_GIT_DIR="$(cd "$SOURCE" && git rev-parse --git-common-dir)"
    # Make it absolute. `git rev-parse --git-common-dir` returns a path
    # relative to PWD when possible; resolve from $SOURCE.
    if [[ "$SHARED_GIT_DIR" != /* ]]; then
        SHARED_GIT_DIR="$(cd "$SOURCE" && cd "$SHARED_GIT_DIR" && pwd)"
    fi
    SHARED_MODULES_DIR="$SHARED_GIT_DIR/modules/forge-java"
    if [ -d "$SHARED_MODULES_DIR" ]; then
        echo "       rewriting forge-java/.git → $SHARED_MODULES_DIR"
        echo "gitdir: $SHARED_MODULES_DIR" > "$NEW_WORKTREE/forge-java/.git"
        # Verify it works.
        if ! ( cd "$NEW_WORKTREE/forge-java" && git status >/dev/null 2>&1 ); then
            echo "       warning: forge-java git pointer rewrite did not produce a working repo;"
            echo "                falling back to fresh git submodule update --init forge-java"
            rm -rf "$NEW_WORKTREE/forge-java"
            ( cd "$NEW_WORKTREE" && git submodule update --init forge-java ) || \
                echo "       warning: forge-java fresh init also failed; continuing"
        else
            FJ_SIZE=$(du -sh "$NEW_WORKTREE/forge-java" 2>/dev/null | awk '{print $1}')
            echo "       forge-java ready (${FJ_SIZE}, reflink in $((FJ_COPY_END - FJ_COPY_START))s, shared modules)"
        fi
    else
        echo "       warning: shared modules dir not found at $SHARED_MODULES_DIR"
        echo "                falling back to fresh git submodule update --init forge-java"
        rm -rf "$NEW_WORKTREE/forge-java"
        ( cd "$NEW_WORKTREE" && git submodule update --init forge-java ) || \
            echo "       warning: forge-java fresh init failed; continuing"
    fi
fi

# Confirm submodule status is clean (no +/-/U prefixes), EXCLUDING submodules
# configured `update = none` in .gitmodules. An inactive (update=none) submodule
# (e.g. the optional `assets` design-asset repo) is intentionally left
# un-checked-out and legitimately shows a '-' prefix — that is NOT dirty. A real
# uninitialised REQUIRED submodule (e.g. forge-java) still flags. This mirrors
# scripts/validate.sh's submodule_dirty_lines so validate won't bail later.
submodule_dirty_lines() {
    local inactive
    inactive=$(git config -f .gitmodules --get-regexp '\.update$' 2>/dev/null \
        | awk '$2 == "none" { name = $1; sub(/^submodule\./, "", name); sub(/\.update$/, "", name); print name }' \
        | while read -r n; do git config -f .gitmodules --get "submodule.$n.path" 2>/dev/null; done)
    git submodule status 2>/dev/null | awk -v inactive="$inactive" '
        BEGIN { n = split(inactive, a, /[ \t\n]+/); for (i = 1; i <= n; i++) if (a[i] != "") skip[a[i]] = 1 }
        /^[+\-U]/ { if (!($2 in skip)) print }
    '
}
if [ -z "$( cd "$NEW_WORKTREE" && submodule_dirty_lines )" ]; then
    echo "       submodule status clean — make validate will not bail on submodules"
else
    echo "       WARNING: submodule status still shows changes:"
    ( cd "$NEW_WORKTREE" && submodule_dirty_lines || true )
fi

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------

echo ""
echo "═════════════════════════════════════════════════════════════════════"
echo "  ✓ Worktree ready"
echo "═════════════════════════════════════════════════════════════════════"
echo "  Path        : $NEW_WORKTREE"
echo "  Branch      : $SAFE_BRANCH (based on $BASE)"
echo "  target/ size: $TARGET_SIZE"
echo "  reflink     : $REFLINK_STATUS"
echo ""
echo "  Activate with:"
echo "    cd $NEW_WORKTREE"
echo ""
echo "  REMINDER: register this worktree in $PARENT_DIR/worktrees/ACTIVE.md"
echo "  BEFORE the first commit (see parent CLAUDE.md → Registry enforcement)."
echo ""
echo "  First incremental build will recompile workspace members only"
echo "  (~1-2 min) because cargo fingerprints embed absolute source paths."
echo "  Dependency compilation is cached. Successive builds are normal-fast."
echo "═════════════════════════════════════════════════════════════════════"
