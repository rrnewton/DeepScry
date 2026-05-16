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

echo "→ [1/5] git fetch origin (in source)"
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
    echo "→ [2/5] cargo build --release --features network (in source)"
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
    echo "→ [2/5] SKIPPED (--no-build) — donor target/ may be stale or broken"
fi

# ---------------------------------------------------------------------------
# Step 3: garbage-collect source's target/
# ---------------------------------------------------------------------------

if [ $DO_SWEEP -eq 1 ]; then
    echo ""
    echo "→ [3/5] cargo sweep --time 14 (drop artifacts >14 days old)"
    if command -v cargo-sweep >/dev/null 2>&1; then
        ( cd "$SOURCE" && cargo sweep --time 14 ) || \
            echo "       warning: cargo sweep --time 14 returned non-zero, continuing"
        echo ""
        echo "→ [3b/5] cargo sweep --installed (drop artifacts from uninstalled toolchains)"
        ( cd "$SOURCE" && cargo sweep --installed ) || \
            echo "       warning: cargo sweep --installed returned non-zero, continuing"
    else
        echo "       warning: cargo-sweep not installed — skipping cleanup."
        echo "                Install with: cargo install cargo-sweep"
    fi
else
    echo ""
    echo "→ [3/5] SKIPPED (--no-sweep) — donor target/ not GC'd"
fi

# ---------------------------------------------------------------------------
# Step 4: create the worktree
# ---------------------------------------------------------------------------

echo ""
echo "→ [4/5] git worktree add $NEW_WORKTREE -b $SAFE_BRANCH $BASE"
git -C "$SOURCE" worktree add "$NEW_WORKTREE" -b "$SAFE_BRANCH" "$BASE"

# ---------------------------------------------------------------------------
# Step 5: CoW-clone target/
# ---------------------------------------------------------------------------

echo ""
echo "→ [5/5] cp -a --reflink=auto $SOURCE/target $NEW_WORKTREE/target"
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
