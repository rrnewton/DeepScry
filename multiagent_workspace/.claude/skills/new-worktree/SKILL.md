---
name: new-worktree
description: Standard workflow for creating a new git worktree in the mtg-forge-rs project. ALWAYS use scripts/new_worktree.sh — never raw `git worktree add` — to get a refreshed primary checkout, a green release build, and a copy-on-write reflinked target/ that makes the new worktree's first incremental build minutes faster. Use whenever spinning up a new worktree for an agent, a feature branch, or any parallel investigation.
---

# new-worktree — official worktree creation workflow

This skill codifies the **only** supported way to create a new worktree
in this project. Every orchestrator and every child agent must follow
it.

## TL;DR

```sh
cd ~/work/mtg                                                  # parent dev-harness root
./scripts/new_worktree.sh <branch-name>                        # base off origin/integration (default)
./scripts/new_worktree.sh <branch-name> --base origin/main     # explicit base
./scripts/new_worktree.sh <branch-name> --source ./worktrees/other-branch
./scripts/new_worktree.sh <branch-name> --no-build             # skip donor green-build (rare)
```

The new worktree appears at `./worktrees/<suffix>` where `<suffix>` is
the branch name with `/` replaced by `-`.

> The script lives in `multiagent_workspace/scripts/new_worktree.sh`
> inside the mtg-forge-rs project, and is symlinked into the parent at
> `<parent>/scripts/new_worktree.sh` by
> `multiagent_workspace/install.sh`. Edit the in-repo copy; never the
> symlink target.

## Hard rules

1. **NEVER run raw `git worktree add` for this project.** It bypasses
   the source-checkout refresh, the green-build precondition, the
   cargo-sweep GC, and the reflink CoW clone. The result is a worktree
   with a cold build cache, costing ~90s of compile time per agent,
   every time.

2. **ALWAYS invoke `scripts/new_worktree.sh` from the parent
   directory** (`~/work/mtg/`), the directory that contains
   `mtg-forge-rs/`, `worktrees/`, and the harness symlinks.

3. **NEVER do code work in `./mtg-forge-rs/`.** That is the **primary
   checkout** — the donor for the reflink clone. Treat it as a
   build-cache reservoir. Keep it on `integration` (or `main`) and
   clean.

4. **ALWAYS register the new worktree in
   `<parent>/worktrees/ACTIVE.md`** BEFORE the agent's first commit.
   See `../CLAUDE.md` → "Registry enforcement".

## What the script does (and why)

The script performs five steps and refuses to proceed if any of them
would produce a degraded worktree:

1. **`git fetch origin` in the source checkout** — guarantees the new
   branch starts from the latest base ref (default
   `origin/integration`).
2. **`cargo build --release --features network` in the source** — this
   is the precondition that makes the donor `target/` worth cloning.
   If the source cannot build green, the script aborts: a broken
   donor would poison every child agent. (Skippable with `--no-build`
   only when you knowingly want a cold-start worktree.)
3. **`cargo sweep --time 14` and `cargo sweep --installed`** — keeps
   the donor `target/` lean by dropping artifacts older than 14 days
   and artifacts from uninstalled toolchains. Keeps every future
   reflink clone small.
4. **`git worktree add <new-path> -b <branch> <base>`** — creates the
   worktree and the branch in one shot. Refuses if the branch already
   exists (so you don't accidentally overwrite work-in-progress).
5. **`cp -a --reflink=auto source/target → new-worktree/target`** —
   on the btrfs/xfs/zfs/apfs filesystems we run on, this is a
   copy-on-write clone that finishes in seconds and consumes zero new
   disk space until cargo overwrites individual artifacts.

### `--source` flag

By default the source is the primary checkout
(`<parent>/mtg-forge-rs/`). Pass `--source <path>` to clone `target/`
from a different worktree — useful when an existing feature worktree
has already built artifacts that match the new branch's commit. The
script still enforces the green-build precondition on the chosen
source (unless `--no-build` is set).

## Why this matters: the CoW target/ optimisation

Cargo's `target/` directory is enormous (12+ GB after a full
`--features network` build). A cold worktree must recompile every
dependency from scratch — that's typically 5+ minutes of CPU.

`cp -a --reflink=auto` on a CoW filesystem (btrfs, xfs, zfs, apfs)
creates extent-shared copies in milliseconds. Cargo's freshness check
then sees up-to-date artifacts for every dependency and skips them.
Only the **workspace member crates** (`mtg-forge-rs`,
`mtg-benchmarks`) recompile, because cargo's fingerprints contain
absolute source paths and the worktree's source path is necessarily
different. That's a roughly 4-5x speedup on the first build, and
successive incremental builds are normal-fast.

## Primary checkout convention

`./mtg-forge-rs/` is the **primary checkout**. It is the donor for
every reflink clone. To preserve that role:

- Keep it on `integration` (or `main`) — never on a feature branch.
- Keep it green: `cargo build --release --features network` must
  succeed. The script enforces this on every invocation.
- Don't edit code in it. All editing happens in feature-branch
  worktrees (`./worktrees/<branch>/`).
- Sibling worktrees are short-lived: created → used → removed via
  `git -C mtg-forge-rs worktree remove <path>` once the branch is
  merged.

## Cleaning up worktrees

When done with a feature branch:

```sh
cd ~/work/mtg

# 1. Move the row from ACTIVE.md to ARCHIVED.md, commit if parent
#    has a remote.
$EDITOR worktrees/ACTIVE.md worktrees/ARCHIVED.md

# 2. Remove the worktree.
git -C mtg-forge-rs worktree remove worktrees/<branch>

# 3. Delete the local branch only if merged or explicitly approved.
git -C mtg-forge-rs branch -D <branch>
```

Don't leave orphan worktrees lying around — they each hold gigabytes
of `target/` data even with reflinking, and they confuse later agents
about which branch is canonical.

## Troubleshooting

- **"branch already exists" error** — pick a different branch name, or
  attach the existing branch manually with
  `git -C mtg-forge-rs worktree add <path> <branch>`.
- **"<path> already exists" error** — the previous worktree wasn't
  cleaned up. Remove it with
  `git -C mtg-forge-rs worktree remove <path>` first.
- **Source release build fails** — fix that *first*. The whole point
  of the script is that the donor must be green. Don't bypass with
  `git worktree add` directly; that just spreads the broken state.
  Use `--no-build` only when you know the donor is intentionally
  ahead of green (e.g., investigating a known regression).
- **Reflink falls back to full copy** — happens on non-CoW filesystems
  (ext4, etc.). The script warns. The clone still works, it just
  takes seconds rather than milliseconds and uses real disk space.
