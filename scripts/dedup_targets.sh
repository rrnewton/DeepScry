#!/usr/bin/env bash
# dedup_targets.sh — reclaim disk by hardlinking byte-identical artifacts
#                    across all git worktree target/ directories.
#
# Background: Cargo builds in parallel worktrees produce mostly-identical
# dependency .rmeta/.rlib/.o files. When the workspace is built with
# `trim-paths = "all"` (see .cargo/config.toml) plus the optional CFLAGS
# prefix-map trick (see release_build.sh), 99%+ of dep artifacts are byte-
# identical across worktrees and can be deduplicated. The buildopt Phase 3
# experiment (phase3-fclones-dedup) measured ~30% / 1.8 GB savings on a
# 2-worktree setup.
#
# IMPORTANT GOTCHA: fclones default invocation respects .gitignore, and the
# unpacked tikv-jemalloc-sys source ships its own .gitignore matching *.o.
# We pass `-A` (--no-ignore) so dep build artifacts are actually considered.
#
# IMPORTANT FILESYSTEM NOTE: on btrfs/xfs/zfs, `cp --reflink=auto` (as used
# by multiagent_workspace/scripts/new_worktree.sh) is a strictly better
# choice than fclones hardlinks for new worktrees, because reflink survives
# `cargo build` overwrites whereas hardlinks get unlinked by the first
# incremental rebuild that touches a deduped file. Use this script for
# periodic maintenance only.
#
# Usage:
#   scripts/dedup_targets.sh                # dedup all sibling worktree target/ dirs
#   scripts/dedup_targets.sh --dry-run      # report savings without linking
#   scripts/dedup_targets.sh path1 path2 …  # dedup an explicit list of dirs

set -euo pipefail

DRY_RUN=0
EXPLICIT_DIRS=()

for arg in "$@"; do
    case "$arg" in
        --dry-run|-n) DRY_RUN=1 ;;
        -h|--help)
            sed -n '2,/^$/p' "$0" | sed 's/^# \{0,1\}//'
            exit 0
            ;;
        *) EXPLICIT_DIRS+=("$arg") ;;
    esac
done

if ! command -v fclones >/dev/null 2>&1; then
    echo "error: fclones not found in PATH. Install with:" >&2
    echo "    cargo install fclones --locked" >&2
    exit 1
fi

# Collect target/ directories. Use explicit list if given, else discover via
# `git worktree list` from the current repo.
TARGET_DIRS=()
if [ "${#EXPLICIT_DIRS[@]}" -gt 0 ]; then
    TARGET_DIRS=("${EXPLICIT_DIRS[@]}")
else
    while IFS= read -r line; do
        # `git worktree list` lines look like:
        #   /path/to/worktree  HASH [branch]
        wt=$(echo "$line" | awk '{print $1}')
        if [ -d "$wt/target" ]; then
            TARGET_DIRS+=("$wt/target")
        fi
    done < <(git worktree list)
fi

if [ "${#TARGET_DIRS[@]}" -lt 2 ]; then
    echo "Need at least 2 target/ directories to dedup (found ${#TARGET_DIRS[@]})." >&2
    echo "Discovered worktrees:" >&2
    git worktree list >&2 || true
    exit 1
fi

echo "Deduplicating across ${#TARGET_DIRS[@]} target/ directories:"
for d in "${TARGET_DIRS[@]}"; do
    sz=$(du -sh "$d" 2>/dev/null | awk '{print $1}')
    echo "  ${sz}  ${d}"
done
echo ""

# Combined-disk-usage before (du -shc dedups hardlinks across args)
BEFORE_TOTAL=$(du -shc "${TARGET_DIRS[@]}" 2>/dev/null | tail -1 | awk '{print $1}')
echo "Combined size before: ${BEFORE_TOTAL}"
echo ""

REPORT=$(mktemp -t fclones_dupes.XXXXXX)
trap 'rm -f "$REPORT"' EXIT

echo "Scanning for duplicates (fclones group -A …)…"
fclones group -A "${TARGET_DIRS[@]}" > "$REPORT" 2>&1 || {
    echo "fclones group failed; report:" >&2
    cat "$REPORT" >&2
    exit 1
}

# Print summary lines from the report header
grep -E "^# (Total|Redundant|Missing):" "$REPORT" || true
echo ""

if [ "$DRY_RUN" -eq 1 ]; then
    echo "(dry-run) Skipping fclones link. Re-run without --dry-run to dedup."
    exit 0
fi

echo "Hardlinking duplicates (fclones link)…"
fclones link < "$REPORT"

AFTER_TOTAL=$(du -shc "${TARGET_DIRS[@]}" 2>/dev/null | tail -1 | awk '{print $1}')
echo ""
echo "Combined size after:  ${AFTER_TOTAL}  (was ${BEFORE_TOTAL})"
echo ""
echo "Done. Re-run after future cargo builds — incremental rebuilds may"
echo "break some hardlinks by overwriting cached files."
