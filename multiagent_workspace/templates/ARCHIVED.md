# Archived worktrees

Historical log of mtg-forge-rs worktrees that have been closed out and
removed from disk. Each row records the final state of the branch at
archive time so future agents can reason about stranded refs without
re-cloning.

**Lifecycle rule:** move rows here from `ACTIVE.md` at closeout time,
BEFORE running `git -C mtg-forge-rs worktree remove`. Append the final
SHA and archive date. See `../CLAUDE.md` → "Archive process".

## Format

| Path                       | Branch                  | Archived    | Final SHA   | Push state         | Purpose                       |
| -------------------------- | ----------------------- | ----------- | ----------- | ------------------ | ----------------------------- |
| `worktrees/<dir>`          | `<branch>`              | YYYY-MM-DD  | `<short>`   | merged/pushed/local | One-line description         |

## Archived entries

<!-- Newest at top. -->

| Path | Branch | Archived | Final SHA | Push state | Purpose |
| ---- | ------ | -------- | --------- | ---------- | ------- |
