#!/usr/bin/env bash
# Renumber hash-based beads IDs -> numeric at the integration serialization
# point, then stage the .beads dir for commit.
#
# Why this script exists: per mtg-forge-rs/CLAUDE.md ("Issue IDs: hash on
# worktrees, numeric on integration"), every NEW issue is born hash-based
# (mb-hash-ids: true) so parallel worktrees never collide. The integration
# branch / primary checkout is the SERIALIZATION POINT where those hash IDs
# get renumbered to readable sequential numbers. Doing this by hand is
# error-prone and easy to forget before a .beads commit (see the missed
# renumber that motivated this script). Invoke this instead.
#
# Usage (from the PRIMARY checkout, on integration):
#   scripts/beads_integration_commit.sh            # renumber + stage .beads
#   scripts/beads_integration_commit.sh --force     # bypass the in-flight gate
#
# It deliberately does NOT create the commit — review `git diff --cached`
# then commit with your own message (the .beads change usually rides along
# with whatever feature/registry work prompted it).
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

force=0
[ "${1:-}" = "--force" ] && force=1

branch=$(git rev-parse --abbrev-ref HEAD)
if [ "$branch" != "integration" ] && [ "$branch" != "main" ]; then
  echo "ERROR: on '$branch'. Renumber only at the serialization point (integration/main)." >&2
  echo "       On a worktree/feature branch, just 'mb create' (hash) and let integration renumber." >&2
  exit 1
fi

# Refuse to run from a linked worktree -- the serialization point is the
# primary checkout. Linked worktrees have a 'gitdir' file in their git dir.
if [ -f "$(git rev-parse --git-dir)/gitdir" ]; then
  echo "ERROR: this is a linked worktree, not the primary checkout. cd to the primary checkout." >&2
  exit 1
fi

# Safety gate: renumbering RENAMES hash-named files (mtg-vk4b7.md ->
# mtg-NNN.md). If any live worktree branch has committed-or-uncommitted edits
# to a .beads/issues file that we are about to rename, the eventual merge
# hits a modify/delete conflict (the documented footgun). Block unless forced.
blocked=0
while read -r wt; do
  [ -n "$wt" ] || continue
  [ "$wt" = "$(pwd)" ] && continue
  # Uncommitted .beads edits in the worktree:
  if [ -n "$(git -C "$wt" status --porcelain -- .beads/issues 2>/dev/null)" ]; then
    echo "BLOCKED: live worktree has UNCOMMITTED .beads/issues edits: $wt" >&2
    blocked=1
  fi
  # Committed-but-unmerged .beads edits on the worktree's branch:
  if git -C "$wt" rev-parse --abbrev-ref HEAD >/dev/null 2>&1; then
    diverged=$(git -C "$wt" diff --name-only integration...HEAD -- .beads/issues 2>/dev/null || true)
    if [ -n "$diverged" ]; then
      echo "BLOCKED: live worktree has unmerged .beads/issues commits: $wt" >&2
      echo "$diverged" | sed 's/^/           /' >&2
      blocked=1
    fi
  fi
done < <(git worktree list --porcelain | awk '/^worktree/{print $2}')

if [ "$blocked" = 1 ] && [ "$force" = 0 ]; then
  echo >&2
  echo "Renumbering now would cause modify/delete rebase conflicts on those hash-named" >&2
  echo "files. Land (or set aside) the in-flight wave first, THEN renumber. Override with" >&2
  echo "--force only if you accept resolving those conflicts manually at merge time." >&2
  exit 1
fi

echo "=== mb mb-migrate --dry-run --to numeric ==="
mb mb-migrate --dry-run --to numeric

echo
echo "=== mb mb-migrate --to numeric ==="
mb mb-migrate --to numeric

# mb-migrate flips mb-hash-ids -> false; we WANT it true for future parallel filing.
sed -i 's/^mb-hash-ids: false/mb-hash-ids: true/' .beads/config-minibeads.yaml
echo "restored mb-hash-ids: true"

git add .beads
echo
echo "Staged .beads (renamed files + ref rewrites + config). Review with:"
echo "  git diff --cached --stat"
echo "Then commit (the .beads change rides with your feature/registry commit)."
