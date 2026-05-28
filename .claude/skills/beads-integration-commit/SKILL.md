---
name: beads-integration-commit
description: Use BEFORE committing any new/changed beads issue to the integration branch in the primary checkout. Renumbers hash-based issue IDs (mtg-a1b2c3) to readable sequential numbers (mtg-171) at the serialization point, restores parallel-safe hash filing, and stages .beads — so the renumber step is never forgotten. Apply when you have filed/edited issues directly on integration, or right after a wave of feature branches (each carrying hash IDs) merges in.
---

# Beads Integration Commit (renumber serialization point)

Per `mtg-forge-rs/CLAUDE.md` → "Issue IDs: hash on worktrees, numeric on
integration": every NEW issue is born **hash-based** (`mb-hash-ids: true`)
so parallel worktrees never collide. The **integration branch / primary
checkout is the serialization point** where hash IDs get renumbered to
readable sequential numbers. This step is easy to forget before a `.beads`
commit — invoke this skill so it is mechanical and verified.

## When to use

- You filed or edited beads issues **directly on `integration`** (in the
  primary checkout) and are about to commit `.beads`.
- A **wave of feature branches just merged**, each carrying hash IDs, and
  you are in a quiescent window before dispatching the next wave.

## When NOT to use

- On a worktree / feature branch: just `bd create` (hash) and commit the
  hash-named file. **Never** renumber on a worktree — let integration do it.

## The hard timing constraint (read before running)

Renumbering **renames** hash-named files (`mtg-vk4b7.md` → `mtg-NNN.md`).
If any **in-flight worktree branch** has committed-or-uncommitted edits to
a `.beads/issues` file that the renumber renames, the eventual merge hits a
**modify/delete rebase conflict** — the documented footgun. So:

> **Only renumber when no in-flight feature branch is touching `.beads`.**
> Do it right after a wave merges, before dispatching the next wave.

The helper script enforces this with a safety gate (it inspects every live
worktree for `.beads/issues` edits and refuses unless `--force`).

## Steps

1. From the **primary checkout**, on `integration`, with the issues you
   want committed already present in `.beads/issues/`:
   ```sh
   scripts/beads_integration_commit.sh
   ```
   It runs `mb mb-migrate --dry-run --to numeric` (shows the plan), then
   `mb mb-migrate --to numeric` (renames hash → numeric, rewrites
   cross-refs), restores `mb-hash-ids: true` (so future filing stays
   parallel-safe), and `git add .beads`.
2. If the script **BLOCKS** citing an in-flight worktree, that is correct
   behavior, not an error: a live branch still edits a hash-named issue.
   Wait for that branch to land, then re-run. Use `--force` only if you
   accept resolving the modify/delete conflicts by hand at merge time.
3. Review and commit:
   ```sh
   git diff --cached --stat        # confirm: renamed issue files + config
   git commit -m "beads: <what changed> (+ renumber hash IDs to numeric)"
   ```
   The `.beads` change normally rides with the feature/registry commit
   that prompted it.

## Note on citing renumbered IDs

A hash ID cited in a commit message or code TODO before renumbering still
resolves via the renamed file's git history. When a reference must survive
a renumber, prefer citing the issue by **title** rather than hash ID.
