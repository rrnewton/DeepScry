---
name: new-worktree
description: Standard workflow for creating a new git worktree in the deepscry project. ALWAYS use scripts/new_worktree.sh — never raw `git worktree add` — to get a refreshed primary checkout, a green release build, and a copy-on-write reflinked target/ that makes the new worktree's first incremental build minutes faster. Use whenever spinning up a new worktree for an agent, a feature branch, or any parallel investigation.
---

# new-worktree — official worktree creation workflow

This skill codifies the **only** supported way to create a new worktree
in this project. Every orchestrator and every child agent must follow
it.

## TL;DR

```sh
cd ~/work/mtg                                                          # parent dev-harness root
./scripts/new_worktree.sh slot01 --branch <branch>                    # fresh branch off origin/integration (default)
./scripts/new_worktree.sh slot01 --branch <existing-branch>           # ATTACH an existing branch into the slot
./scripts/new_worktree.sh slot01 --branch <branch> --base origin/main # explicit base (new branch only)
./scripts/new_worktree.sh slot01 --branch <branch> --source ./worktrees/slot02
./scripts/new_worktree.sh slot01 --branch <branch> --no-build         # skip donor green-build (rare)
```

**Slot protocol:** the first positional arg is the **opaque slot
directory** (`slot01`, `slot02`, …) created under `worktrees/`. Slot
names are permanent identifiers for the physical directory; the branch
inside can change freely. `--branch` names the branch — if it does not
exist it is created off the base; if it **already exists** it is
**attached** as-is (so a slot can re-home an existing branch, e.g. after
a workspace move). If `--branch` is omitted it defaults to the slot
name. The worktree appears at `./worktrees/<slot>`.

> The script lives in `multiagent_workspace/scripts/new_worktree.sh`
> inside the deepscry project, and is symlinked into the parent at
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
   `deepscry/`, `worktrees/`, and the harness symlinks.

3. **NEVER do code work in `./deepscry/`.** That is the **primary
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
4. **`git worktree add <slot-path> -b <branch> <base>`** — creates the
   worktree and the branch in one shot. If the branch **already
   exists**, it is **attached** to the slot instead (`git worktree add
   <slot-path> <branch>`); the script only aborts if that branch is
   already checked out in another worktree (git forbids that).
5. **`cp -a --reflink=auto source/target → new-worktree/target`** —
   on the btrfs/xfs/zfs/apfs filesystems we run on, this is a
   copy-on-write clone that finishes in seconds and consumes zero new
   disk space until cargo overwrites individual artifacts.

### `--source` flag

By default the source is the primary checkout
(`<parent>/deepscry/`). Pass `--source <path>` to clone `target/`
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
Only the **workspace member crates** (`mtg-engine`,
`mtg-benchmarks`) recompile, because cargo's fingerprints contain
absolute source paths and the worktree's source path is necessarily
different. That's a roughly 4-5x speedup on the first build, and
successive incremental builds are normal-fast.

## Primary checkout convention

`./deepscry/` is the **primary checkout**. It is the donor for
every reflink clone. To preserve that role:

- Keep it on `integration` (or `main`) — never on a feature branch.
- Keep it green: `cargo build --release --features network` must
  succeed. The script enforces this on every invocation.
- Don't edit code in it. All editing happens in feature-branch
  worktrees (`./worktrees/<branch>/`).
- Sibling worktrees are short-lived: created → used → removed via
  `git -C deepscry worktree remove <path>` once the branch is
  merged.

## Cleaning up worktrees

When done with a feature branch, use the **teardown script** — the
counterpart to `new_worktree.sh`. It ENFORCES the registry move that is
otherwise easy to forget: it refuses to remove the worktree while
`ACTIVE.md` still lists the slot, and prints the ready-to-paste
`ARCHIVED.md` row (path/branch/date/SHA pre-filled):

```sh
cd ~/work/mtg
./scripts/archive_worktree.sh slot<NN>
# • If ACTIVE.md still lists the slot → the script STOPS and prints the
#   ARCHIVED.md row to paste (you add push-state + one-line purpose).
#   Move it to the TOP of ARCHIVED.md, delete the ACTIVE.md row, re-run.
# • Once ACTIVE.md no longer lists the slot → it runs
#   `git worktree remove --force` (force = the worktree has submodules).
# • The branch ref is LEFT INTACT. Delete it only if merged / approved:
git -C deepscry branch -D <branch>
```

The script also refuses if the worktree has uncommitted/untracked work,
so you never silently discard it.

Don't leave orphan worktrees lying around — they each hold gigabytes
of `target/` data even with reflinking, and they confuse later agents
about which branch is canonical.

## Troubleshooting

- **branch already checked out elsewhere** — git allows a branch in only
  one worktree at a time. The script aborts and prints the worktree
  list; remove the other worktree (or pick a different branch) first.
  (An existing branch that is NOT checked out anywhere is simply
  attached — that is the normal re-home flow, not an error.)
- **"<path> already exists" error** — that slot is already in use.
  Remove it with `git -C deepscry worktree remove worktrees/slot<NN>`
  first, or pick a different slot number.
- **Source release build fails** — fix that *first*. The whole point
  of the script is that the donor must be green. Don't bypass with
  `git worktree add` directly; that just spreads the broken state.
  Use `--no-build` only when you know the donor is intentionally
  ahead of green (e.g., investigating a known regression).
- **Reflink falls back to full copy** — happens on non-CoW filesystems
  (ext4, etc.). The script warns. The clone still works, it just
  takes seconds rather than milliseconds and uses real disk space.
