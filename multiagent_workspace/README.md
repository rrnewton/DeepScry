# `multiagent_workspace/` — the in-repo dev harness kit

This directory ships the **parent-workspace dev harness** alongside the
`deepscry` project source. It is the install-time content for the
parent directory (the "dev harness root") that hosts the primary
checkout plus any number of agent worktrees.

Without this kit, the parent directory is just a plain folder
containing a clone of deepscry. With this kit installed, the parent
becomes a coordinated multi-agent workspace with:

- A canonical workspace-discipline `CLAUDE.md` (registered worktrees,
  clean-start gate, archive process, coordinator instructions).
- The `new_worktree.sh` script enforcing fetch → green-build →
  cargo-sweep → reflinked `target/` for every new worktree.
- A `worktrees/` directory pre-seeded with `ACTIVE.md` and
  `ARCHIVED.md` registry templates.
- A `.claude/` skill/command tree that Claude Code (or compatible
  agents) auto-loads when run from the parent dir.

The kit is **versioned with the project source** so every clone of
deepscry carries the harness with it. To install on a new machine,
run `install.sh` from the parent directory (see below).

## Layout of this kit

```
multiagent_workspace/
├── README.md                     (this file)
├── CLAUDE.md                     (workspace-discipline guide; symlinked to parent)
├── .gitignore                    (parent repo ignore rules; read via core.excludesFile)
├── install.sh                    (one-shot installer)
├── scripts/
│   └── new_worktree.sh           (worktree creation; symlinked to parent)
├── .claude/
│   ├── commands/
│   │   └── playtester.md         (long-form MTG playtester command)
│   └── skills/
│       └── new-worktree/
│           └── SKILL.md          (Claude Code skill: worktree workflow)
└── templates/
    ├── ACTIVE.md                 (worktree registry; COPIED, not symlinked)
    └── ARCHIVED.md               (historical log;     COPIED, not symlinked)
```

## Install procedure

From a fresh checkout of deepscry:

```sh
# 1. Set up the parent directory layout. If you cloned the project
#    directly into ~/work/mtg/deepscry/ , the parent is already
#    where the installer expects it.
cd ~/work/mtg/deepscry        # the project checkout

# 2. Run the installer. It works from inside the project checkout and
#    operates on its parent directory.
./multiagent_workspace/install.sh

# 3. From now on, work from the PARENT:
cd ..
ls   # CLAUDE.md, .claude, scripts/, worktrees/, deepscry/ ...
```

What `install.sh` does:

1. **Verifies layout.** Confirms `parent/deepscry/` exists and is
   a git checkout (the primary).
2. **Symlinks** the following into the parent:
   - `parent/CLAUDE.md` → `deepscry/multiagent_workspace/CLAUDE.md`
   - `parent/.claude` → `deepscry/multiagent_workspace/.claude`
   - `parent/scripts/new_worktree.sh` →
     `deepscry/multiagent_workspace/scripts/new_worktree.sh`
3. **Copies (does NOT symlink)** the mutable registry templates:
   - `templates/ACTIVE.md` → `parent/worktrees/ACTIVE.md` (only if absent)
   - `templates/ARCHIVED.md` → `parent/worktrees/ARCHIVED.md` (only if absent)
4. **Creates `parent/worktrees/`** if it doesn't already exist.
5. **Initialises a local-only git repo** in the parent (with
   deepscry registered as a submodule if not already so), and points the
   parent's `core.excludesFile` at `multiagent_workspace/.gitignore`.
   (git won't follow a symlinked top-level `.gitignore`, so the parent's
   ignore rules come from the in-repo file via excludesFile — single
   source, no copy.) Does
   NOT add a remote — the parent repo is purely local audit history
   by default.

Symlinks vs. copies, in summary:

| Path in parent           | Mode    | Why                                            |
| ------------------------ | ------- | ---------------------------------------------- |
| `CLAUDE.md`              | symlink | one canonical guide, versioned with the kit   |
| `.claude/`               | symlink | skills/commands evolve with the project       |
| `scripts/new_worktree.sh`| symlink | script edits flow on `git pull`               |
| `worktrees/ACTIVE.md`    | copy    | per-machine state, must not be tracked by kit |
| `worktrees/ARCHIVED.md`  | copy    | per-machine state, must not be tracked by kit |
| `.gitignore`             | copy    | editable per-machine                           |

## Updating the kit

When the harness evolves, edit files **in
`deepscry/multiagent_workspace/`** (the kit), not the parent
symlinks. Commit and push as part of normal project work. Other
machines pick up changes via the next `git pull` of deepscry.

If you need to add a new templated file (per-machine state), drop it
in `templates/` and extend `install.sh` to copy it on install.

## See also

- `CLAUDE.md` — the actual workspace discipline guide.
- `<RepoRoot>/CLAUDE.md` — project-internal conventions (coding
  rules, branch ceremony, beads workflow).
- `.claude/skills/new-worktree/SKILL.md` — the worktree-creation
  workflow Claude Code auto-loads.
