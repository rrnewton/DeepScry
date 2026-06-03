#!/usr/bin/env bash
# archive_worktree.sh — official TEARDOWN for a deepscry worktree.
#
# This is the counterpart to new_worktree.sh. It exists because the
# ACTIVE.md → ARCHIVED.md registry move is the step everyone forgets,
# so this script REFUSES to remove a worktree until that move is done —
# and prints a ready-to-paste ARCHIVED.md row to make it one step.
#
# Usage:
#   ./scripts/archive_worktree.sh <slot>          # e.g. slot01
#
# Run from the PARENT directory (the one containing deepscry/ and
# worktrees/). Steps:
#
#   1. Verify the worktree is CLEAN (refuse if modified/untracked).
#   2. GATE on the registry: if ACTIVE.md still lists the slot, STOP and
#      print the exact ARCHIVED.md row (path/branch/date/SHA filled in)
#      to move — you add push-state + one-line purpose, delete the
#      ACTIVE row, then re-run.
#   3. Once ACTIVE.md no longer lists the slot, `git worktree remove` it
#      (--force, because the worktree contains submodules). The branch
#      ref is LEFT IN PLACE (delete it only if merged / user-approved).
#
# See the parent CLAUDE.md → "Worktree lifecycle" / "Archive process".

set -euo pipefail

SLOT="${1:-}"
if [ -z "$SLOT" ]; then
    echo "usage: $0 <slot>   (e.g. slot01)" >&2
    exit 2
fi
SLOT="${SLOT#worktrees/}"   # tolerate a worktrees/ prefix

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PARENT_DIR="$(dirname "$SCRIPT_DIR")"
PRIMARY="$PARENT_DIR/deepscry"
WT="$PARENT_DIR/worktrees/$SLOT"
ACTIVE="$PARENT_DIR/worktrees/ACTIVE.md"
ARCHIVED="$PARENT_DIR/worktrees/ARCHIVED.md"

if [ ! -d "$WT" ]; then
    echo "error: no worktree at $WT" >&2
    exit 1
fi

BRANCH="$(git -C "$WT" rev-parse --abbrev-ref HEAD)"
SHA="$(git -C "$WT" rev-parse --short HEAD)"
TODAY="$(date +%F)"

# --- 1. Clean check (ignore the shared forge-java submodule noise; we only
#        care that no real source work is uncommitted/untracked here). ---
if [ -n "$(git -C "$WT" status --porcelain --ignore-submodules=all)" ]; then
    echo "error: worktree '$SLOT' ($BRANCH) is NOT clean — commit, stash, or" >&2
    echo "       remove untracked files before archiving:" >&2
    git -C "$WT" status --short --ignore-submodules=all >&2
    exit 1
fi

# --- 2. Registry gate ---
if grep -qF "worktrees/$SLOT\`" "$ACTIVE" 2>/dev/null || grep -qE "worktrees/$SLOT( |\`)" "$ACTIVE" 2>/dev/null; then
    cat >&2 <<EOF
──────────────────────────────────────────────────────────────────────
STOP: worktrees/$SLOT is still listed in ACTIVE.md.

Per the worktree lifecycle, move its row to ARCHIVED.md FIRST (newest at
the TOP of the table), THEN re-run this script. Ready-to-paste row —
fill in <push state> and <one-line purpose>:

| \`worktrees/$SLOT\` (slot) | \`$BRANCH\` | $TODAY | \`$SHA\` | <push state> | <one-line purpose> |

Then delete the matching row from ACTIVE.md and re-run:
    $0 $SLOT
──────────────────────────────────────────────────────────────────────
EOF
    exit 1
fi

# --- 3. Remove the worktree (force: it contains submodules) ---
echo "→ ACTIVE.md no longer lists $SLOT; removing the worktree."
git -C "$PRIMARY" worktree remove --force "$WT"
echo "✓ removed worktrees/$SLOT  (branch '$BRANCH' @ $SHA left intact)."
echo "  Delete the branch only if it has merged into a tracked branch or"
echo "  the user explicitly approves:  git -C deepscry branch -D $BRANCH"
