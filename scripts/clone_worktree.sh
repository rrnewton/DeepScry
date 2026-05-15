#!/usr/bin/env bash
# clone_worktree.sh — create a new git worktree with a CoW-cloned target/
#                     directory for instant near-zero-cost build setup.
#
# When the working filesystem is btrfs / xfs / zfs (which support reflink),
# `cp --reflink=auto` produces a copy-on-write clone of the target/ tree in
# milliseconds without touching disk space. The new worktree's cargo build
# starts from a fully-populated cache and only recompiles whatever the new
# branch actually changes.
#
# This is strictly better than the fclones-hardlink approach (see
# dedup_targets.sh) because:
#   - hardlinks share an inode and break on the first cargo overwrite
#   - reflink CoW keeps mtimes intact, so cargo's freshness check is happy
#   - extents that legitimately diverge (because of different commits)
#     still consume their own blocks, but identical extents stay shared
#
# Usage:
#   scripts/clone_worktree.sh BRANCH                        # clone target/ from current worktree
#   scripts/clone_worktree.sh BRANCH SOURCE_WORKTREE_PATH   # explicit source
#   scripts/clone_worktree.sh -b BASE BRANCH                # base BRANCH off BASE (default: HEAD)
#
# Examples:
#   scripts/clone_worktree.sh fix-foo                            # new branch fix-foo at HEAD
#   scripts/clone_worktree.sh -b integration fix-foo             # branched from integration
#   scripts/clone_worktree.sh fix-foo /path/to/another-worktree  # use specified source

set -euo pipefail

BASE="HEAD"
ARGS=()

while [ $# -gt 0 ]; do
    case "$1" in
        -b|--base) BASE="$2"; shift 2 ;;
        -h|--help)
            sed -n '2,/^$/p' "$0" | sed 's/^# \{0,1\}//'
            exit 0
            ;;
        *) ARGS+=("$1"); shift ;;
    esac
done

if [ "${#ARGS[@]}" -lt 1 ] || [ "${#ARGS[@]}" -gt 2 ]; then
    echo "Usage: $0 [-b BASE] BRANCH [SOURCE_WORKTREE_PATH]" >&2
    exit 1
fi

NEW_BRANCH="${ARGS[0]}"
SOURCE_WORKTREE="${ARGS[1]:-}"

# Locate the current repo top-level (caller may invoke from a subdir).
REPO_ROOT=$(git rev-parse --show-toplevel)
PARENT_DIR=$(dirname "$REPO_ROOT")

# Default source: the worktree we're being invoked from.
if [ -z "$SOURCE_WORKTREE" ]; then
    SOURCE_WORKTREE="$REPO_ROOT"
fi

if [ ! -d "$SOURCE_WORKTREE/target" ]; then
    echo "warning: source worktree has no target/ dir at $SOURCE_WORKTREE/target" >&2
    echo "         new worktree will start with a fresh build cache." >&2
fi

# New worktree path: parent_dir / repo-basename + branch suffix
REPO_NAME=$(basename "$REPO_ROOT")
# Strip any trailing digits/dashes the user may have used (mtg-forge-rs2 → mtg-forge-rs)
BASE_REPO=$(echo "$REPO_NAME" | sed -E 's/[-]?[0-9]+$//')
SAFE_BRANCH=$(echo "$NEW_BRANCH" | tr '/' '-')
NEW_WORKTREE="$PARENT_DIR/${BASE_REPO}-${SAFE_BRANCH}"

if [ -e "$NEW_WORKTREE" ]; then
    echo "error: $NEW_WORKTREE already exists. Choose a different branch name or remove it first." >&2
    exit 1
fi

echo "Source worktree: $SOURCE_WORKTREE"
echo "New worktree:    $NEW_WORKTREE"
echo "New branch:      $NEW_BRANCH (based on $BASE)"
echo ""

# Step 1: create the worktree (creates branch if it doesn't exist)
echo "→ git worktree add -b $NEW_BRANCH $NEW_WORKTREE $BASE"
if git rev-parse --verify --quiet "refs/heads/$NEW_BRANCH" >/dev/null; then
    # Branch already exists, attach without -b
    git worktree add "$NEW_WORKTREE" "$NEW_BRANCH"
else
    git worktree add -b "$NEW_BRANCH" "$NEW_WORKTREE" "$BASE"
fi

# Step 2: CoW-clone target/ if source has one
if [ -d "$SOURCE_WORKTREE/target" ]; then
    # Detect filesystem; warn if reflink unavailable
    FSTYPE=$(stat -fc '%T' "$SOURCE_WORKTREE/target" 2>/dev/null || echo "unknown")
    case "$FSTYPE" in
        btrfs|xfs|zfs|apfs)
            REFLINK_MODE="auto" ;;
        *)
            echo "" >&2
            echo "warning: filesystem type '$FSTYPE' may not support reflink." >&2
            echo "         If reflink fails, cp will fall back to a full byte copy." >&2
            REFLINK_MODE="auto" ;;
    esac

    echo ""
    echo "→ cp -a --reflink=$REFLINK_MODE $SOURCE_WORKTREE/target $NEW_WORKTREE/target"
    START=$(date +%s.%N)
    # 'cp -a' preserves timestamps, perms, and recurses; --reflink=auto uses CoW
    # when supported and falls back to full copy otherwise.
    cp -a --reflink="$REFLINK_MODE" "$SOURCE_WORKTREE/target" "$NEW_WORKTREE/target"
    END=$(date +%s.%N)
    ELAPSED=$(awk "BEGIN{printf \"%.2f\", $END - $START}")
    SIZE=$(du -sh "$NEW_WORKTREE/target" 2>/dev/null | awk '{print $1}')
    echo "  cloned ${SIZE} of target/ in ${ELAPSED}s"
fi

echo ""
echo "Done. Activate the new worktree with:"
echo "    cd $NEW_WORKTREE"
echo ""
echo "First incremental cargo build should be near-instant if no source changed."
