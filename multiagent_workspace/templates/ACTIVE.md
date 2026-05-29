# Active worktrees

This file lists every live DeepScry worktree in
`<parent>/worktrees/` along with its branch and one-line purpose. It is
the durable source of truth for "what agent work is in flight on this
machine."

**Lifecycle rule:** every worktree under `worktrees/` MUST have a row
here. Add the row at dispatch time (BEFORE the agent's first commit).
Move it to `ARCHIVED.md` at closeout time (BEFORE
`git worktree remove`). See `../CLAUDE.md` → "Registry enforcement" for
the full discipline.

**Audit:** run the diff in `../CLAUDE.md` → "Audit self-check" before
spawning new agents to detect drift between this file and disk.

## Format

| Path                       | Branch                  | Started     | Purpose                              |
| -------------------------- | ----------------------- | ----------- | ------------------------------------ |
| `worktrees/<dir>`          | `<branch>`              | YYYY-MM-DD  | One-line description                 |

## Live entries

<!-- Add rows here. Keep the table sorted by Started ascending. -->

| Path | Branch | Started | Purpose |
| ---- | ------ | ------- | ------- |
