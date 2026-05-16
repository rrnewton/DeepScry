# Parent Workspace Agent Guide (mtg-forge-rs)

This is the **dev harness** around the `mtg-forge-rs` project. It is NOT a
code project. It is the meta-workspace that contains the primary
checkout, private agent worktrees, experiment captures, ai_docs, scratch
output, and harness tooling. Keep the parent directory clean: every file
must be tracked, committed, or `.gitignored` before handoff.

The harness itself is versioned inside the project at
`mtg-forge-rs/multiagent_workspace/`. This file (`CLAUDE.md`) is
symlinked from the parent dir to `mtg-forge-rs/multiagent_workspace/CLAUDE.md`
by `multiagent_workspace/install.sh`. Edit the in-repo copy, not the
symlink target — changes there flow back to every machine via `git pull`.

## Vocabulary

These terms recur throughout this guide; agents must use them
consistently.

- **`parent`** — the dev harness root (typically `~/work/mtg/` or
  `~/working_copies/mtg/`; the two paths resolve to the same tree via
  the `~/work → ~/working_copies` symlink). When the user says
  "parent" they mean this dev harness, NOT any git "parent" commit and
  NOT the mtg-forge-rs project repo.
- **`primary checkout`** — `parent/mtg-forge-rs/`, the canonical
  mtg-forge-rs clone. Acts as the **donor** for reflink-cloned
  `target/` trees in every new worktree. Treat it as a build-cache
  reservoir, not a development workspace.
- **`worktree`** — an agent-private mtg-forge-rs checkout under
  `parent/worktrees/<branch>/`. All mutating work happens here, never
  in the primary checkout.
- **`mtg-forge-rs`** — the GitHub project. Remote `origin` points at
  the OSS upstream. The three-tier branch structure is `feature
  branches` → `integration` → `main`; see `mtg-forge-rs/CLAUDE.md` for
  the merge ceremony.

## Repo Boundaries & Layout

The parent directory is the orchestrator's CWD and coordination
workspace. Spawned agents normally start in a private worktree under
`worktrees/`.

Top-level layout:

```
parent/                       (= ~/work/mtg/)
├── CLAUDE.md                 → mtg-forge-rs/multiagent_workspace/CLAUDE.md (symlink)
├── .claude/                  → mtg-forge-rs/multiagent_workspace/.claude/ (symlink)
├── scripts/
│   └── new_worktree.sh       → mtg-forge-rs/multiagent_workspace/scripts/new_worktree.sh (symlink)
├── worktrees/
│   ├── ACTIVE.md             (registry of live agent worktrees)
│   ├── ARCHIVED.md           (historical log)
│   └── <branch-name>/        (each live worktree)
├── mtg-forge-rs/             (primary checkout)
├── ai_docs/                  (transient AI scratch notes; optional)
├── experiments/              (captured experiment data; optional)
├── scratch/                  (loose binaries, profiling output; optional)
└── .git/                     (local-only parent repo; usually no remote)
```

Project boundaries:

- `mtg-forge-rs/` is the actual project. Durable feature work, tests,
  architecture docs, and **beads (`mb`) issues** live there. See
  `mtg-forge-rs/CLAUDE.md` for project-internal rules — the coding
  conventions, DRY principles, no-clone/no-collect performance rules,
  the **No Hacky String Operations On Structured Data** rule, and the
  three-tier branch discipline.
- Parent-repo commits (if the parent has a remote at all) are for
  workspace policy, experiment artifacts, reports, and harness-level
  tooling. Do not commit transient source changes here that belong in
  a project checkout.

Rule of thumb: if it is durable source or project documentation for a
fresh clone, put it in `mtg-forge-rs/` (the project). If it is
investigation state, experiment data, or harness coordination, keep it
in the parent workspace.

## Workspace Discipline

Every agent that writes code gets a unique worktree. Do not let multiple
mutating agents share the same checkout.

Worktree naming:

- Directory: `parent/worktrees/<branch>/`
- Branch:    `<branch>` (any descriptive feature-branch name)
- The directory name is the branch name with `/` replaced by `-` (so
  `feature/foo-bar` → `worktrees/feature-foo-bar/`).
- The primary checkout (`mtg-forge-rs/`) is for integration work only:
  the donor of cached `target/` artifacts, the staging ground for
  merging accepted work, and the launchpad for new worktrees. Agents
  do not directly modify it.

Branch rules:

- Agent work happens on local feature branches branched from
  `origin/integration` (the default base) or `origin/main` (when
  explicit).
- The primary checkout MUST stay on `integration` or `main` and MUST
  build green with `cargo build --release --features network`. The
  `new_worktree.sh` script enforces this on every invocation.
- Bug-fix branches require an MTG rules review before merging into
  `integration`; see `mtg-forge-rs/.claude/skills/mtg-rules-review.md`.

Worktree registry:

- Maintain `worktrees/ACTIVE.md` with every live worktree and branch
  plus a one-line purpose.
- Maintain `worktrees/ARCHIVED.md` as the historical log. When a
  worktree is archived, move the entry from ACTIVE to ARCHIVED and add
  the archive date.
- The orchestrator periodically compares `worktrees/` against
  `ACTIVE.md` and checks for stale worktrees, missing entries, and
  stranded agent branches.

Worktree lifecycle:

1. **Clean start.** Use `./scripts/new_worktree.sh <branch>` from the
   parent directory. The script (a) fetches `origin` in the primary,
   (b) verifies the primary builds green, (c) cleans the donor
   `target/` via `cargo sweep`, (d) creates the worktree under
   `worktrees/<branch>/`, (e) reflink-clones `target/` into it. Then
   register the new worktree in `worktrees/ACTIVE.md` BEFORE work
   begins. No exception for "small" tasks.
2. **Clean finish.** A task is not done until the worktree is clean:
   zero modified files, zero untracked files (unless covered by
   `.gitignore`). Files that should be tracked must be added and
   committed; truly transient files must be deleted or added to
   `.gitignore`. The reviewer or orchestrator must refuse to close a
   task whose worktree is not clean.
3. **Closure.** When a task closes, DELETE the worktree — do not leave
   it sitting around (each worktree holds gigabytes of `target/` data
   even with reflinking, and stale worktrees confuse later agents
   about which branch is canonical). Move its record (final commit
   SHA, branch, archive date) from `worktrees/ACTIVE.md` to
   `worktrees/ARCHIVED.md`. The local feature branch stays unless it
   has merged into a tracked branch or the user explicitly approves
   deletion. See Archive process below for the mechanical steps.
4. **Audit cadence.** The orchestrator must periodically reconcile —
   minimally before each new agent spawn — that `worktrees/` on disk
   matches `ACTIVE.md` exactly: no rogue paths, no stranded entries,
   no orphan feature branches.

Registry enforcement:

The Worktree registry rules above state WHAT must hold; this
subsection states HOW each lifecycle transition is verified in
practice.

1. **Pre-commit registration check (Clean-start enforcement).** Before
   an agent's FIRST commit in its worktree, it must verify its branch
   is registered. From inside the agent worktree:

   ```bash
   PARENT=~/work/mtg
   BRANCH=$(git rev-parse --abbrev-ref HEAD)
   grep -F "$BRANCH" "$PARENT/worktrees/ACTIVE.md" \
     || { echo "BRANCH $BRANCH NOT IN ACTIVE.md — register before committing"; exit 1; }
   ```

   If the grep fails, STOP. Add the row to `ACTIVE.md` (committed in
   the parent repo if it has one), and only then proceed with source
   work. The orchestrator is responsible for pre-registering at
   dispatch time, but the agent must double-check because
   dispatch-time registration slips.

2. **Pre-archive registration update (Clean-finish enforcement).**
   Before running `git worktree remove`, the closing agent (or the
   orchestrator on its behalf) must move the row from `ACTIVE.md` to
   `ARCHIVED.md` with a final-state summary (final SHA, push state,
   archive date). This ordering keeps the registry durable even if
   `git worktree remove` is interrupted.

3. **Dispatch-time registration step (orchestrator enforcement).**
   Every agent-spawn brief MUST include an explicit numbered step
   "Register branch + worktree in `worktrees/ACTIVE.md` BEFORE first
   source commit." Likewise, every closeout brief MUST include
   "Move row to `worktrees/ARCHIVED.md` BEFORE `git worktree remove`."

4. **Audit self-check (one-line reconciliation).** The orchestrator's
   periodic audit compares disk against `ACTIVE.md` with:

   ```bash
   diff \
     <( git -C mtg-forge-rs worktree list --porcelain \
          | awk '/^worktree/{print $2}' \
          | grep -F "/worktrees/" \
          | sed -E 's|.*/worktrees/||' \
          | sort -u ) \
     <( awk -F'`' '/^\| [a-z0-9-]/{print $2}' worktrees/ACTIVE.md \
          | sed 's|^worktrees/||' | sort )
   ```

   Any line in the diff is a discipline violation: left-only paths are
   stranded worktrees with no `ACTIVE.md` row; right-only paths are
   `ACTIVE.md` rows pointing at deleted worktrees. Run before every
   new agent spawn at minimum.

5. **Failure mode: pushed-to-origin without ACTIVE.md row.** The most
   dangerous failure mode is an agent that commits, pushes to origin,
   and exits without registering — leaving a stranded feature branch
   on the remote with no local breadcrumb explaining what it is or
   whether it is safe to rebase / archive / merge.

Clean-start gate:

The `git status` clean check is a PRECONDITION for starting work in
ANY checkout — the parent dev harness, the primary checkout
(`mtg-forge-rs/`), AND every agent worktree (`worktrees/<branch>/`).
Reproducibility depends on it: if an agent determines that commit X
yields result Y, that result must be reproducible by other agents and
by the user from the same SHA without depending on untracked files
sitting in someone's working tree. **Untracked files break determinism.**

1. **Run `git status` FIRST.** Before starting any task, run
   `git status` (or `git -C <checkout> status`) in every checkout the
   task will touch. The expected output is "nothing to commit, working
   tree clean."
2. **Resolve dirty state BEFORE starting work.** If `git status` shows
   anything modified or untracked (and not gitignored), resolve it
   first using one of these paths:
   - **Track and commit** small text files that are durable harness or
     project state.
   - **Add to `.gitignore`** truly transient patterns (build
     artifacts, trace captures, runtime logs, scratch output).
   - **DELETE stray scratch files** that are neither durable nor
     pattern-matched. Use the safety-net pattern (copy to
     `scratch/<task>-<date>/` before `rm -rf` if there is any chance
     the content matters).
   - **Surface to user** anything ambiguous or any submodule pointer
     drift. Do NOT silently commit, gitignore, or delete these.
3. **No starting work atop a dirty tree.** The reviewer or
   orchestrator must refuse to launch a new task in a checkout whose
   `git status` is not clean. This applies as much to the
   orchestrator's own parent CWD as to any spawned agent worktree.

Parent commit cadence:

If the parent has a remote (it usually does NOT — `install.sh`
initializes a local-only repo), the same discipline applies: commit
coherent units immediately, do not accumulate WIP. Untracked files in
the parent have a half-life of one task: either commit them or
`.gitignore` them. Without a remote, the commits still serve as a
local audit log of harness state — `worktrees/ACTIVE.md` updates,
experiment captures, harness-script edits.

Archive process:

1. Verify the worktree has no uncommitted changes and `git status` is
   clean, including no untracked files. If either check fails, surface
   it to the user or orchestrator. Never silently discard work.
2. Move the entry from `worktrees/ACTIVE.md` to `worktrees/ARCHIVED.md`,
   keeping the description and adding the archive date and final SHA.
3. Remove the git worktree:
   `git -C mtg-forge-rs worktree remove worktrees/<branch>`.
4. Delete the local branch only if it has merged into a tracked branch
   or the user explicitly approves deletion.
5. Confirm no data was lost. Reachable commits must remain available
   from refs, or be explicitly covered by the rollback/recovery plan.

## CWD Protocol

- Orchestrator CWD: parent workspace root (`~/work/mtg/`).
- Mutating agent CWD: `worktrees/<branch>/`.
- Read-only agent CWD: normally the primary checkout `mtg-forge-rs/`
  (treat as read-only) or a clearly marked read-only worktree.
- Task instructions may direct outputs outside the agent CWD, such as
  `ai_docs/` or `experiments/`. Follow those destinations exactly.
- Durable documentation, when agreed with the user, goes in the
  project's canonical docs directory: `mtg-forge-rs/docs/` or
  `mtg-forge-rs/ai_docs/`.

## Task Tracking

Two systems coexist and do not auto-sync:

- **`mb` (minibeads)** — the PRIMARY, version-controlled task store
  inside `mtg-forge-rs/.beads/`. Issues are durable project state.
  Run `mb` from inside the project / a worktree. Read `bd quickstart`
  (the upstream `bd` CLI is the underlying tool) for the workflow.
  Conventions (mirrored from `mtg-forge-rs/CLAUDE.md`):
  - Issues labelled "human" are user-created (priority 0).
  - Tracking issues sit at priority 1; `mtg-1` is the overall tracker.
  - Granular issues are priority 3-4; bumped to 2 for critical bugs.
  - TODOs in code reference issues: `// TODO(mtg-13): summary`.
  - All content goes in the `description` field — never use `--notes`.
  - Always `bd update` existing issues; never duplicate via `bd create`.
- **`tg` (task-graph)** — ephemeral, per-session orchestration state on
  the local machine. Used by the orchestrator to track in-flight
  agents and short-lived subtasks. NOT durable; significant outcomes
  must be summarized into a minibeads issue before the session ends.

When you commit, update beads issues to reflect what was completed and
what's next, so the next person/agent can pick up the work from the
beads issues alone, without access to `tg` graphs or chat history.

## Experiment Captures

When the harness runs experiments (perf comparisons, AI heuristic
sweeps, deck-pair tournaments), capture them under
`parent/experiments/<experiment_name>_YYYYMMDD/`.

- Include `README.md` with hypothesis, methodology, and result summary.
- Include `metadata.json` at experiment and capture level (commit SHA
  of mtg-forge-rs under test, command line, seeds used, host info).
- CSVs must have headers and consistent columns.
- Reports must cite source files and commands for every number.
- Do not hand-write data tables. Generate them from captured data with
  scripts.

Use the project's transient-information stamp convention for any
result that derives from a specific commit (see
`mtg-forge-rs/CLAUDE.md` → "Mark transient information"):

```
YYYY-MM-DD_#DEPTH(<short-sha>)
```

where `DEPTH = git rev-list --count HEAD` (or `./scripts/gitdepth.sh`).

## File Conventions

- **Experiments:** `experiments/<name>_YYYYMMDD/` (see above).
- **AI scratch notes / analysis docs:** `ai_docs/SUBJECT_YYYYMMDD.md`.
- **Loose binaries, profiler data:** `scratch/` or a gitignored
  subdir under an experiment.
- **Coverage:** commit text summaries (txt, lcov, json). Ignore
  generated HTML reports and `.profdata`/`.profraw`.

Do not leave undated scratch reports or untracked markdown inside
project checkouts. The orchestrator owns parent workspace cleanliness.

## Commit Hygiene

Parent-repo history (if a remote exists) must stay small. Before
`git add` or `git commit` in the parent repo, audit what is about to
be added:

```bash
git diff --cached --stat
git diff --cached --name-only --diff-filter=ACMRT | xargs -r ls -lh
```

Hard ceiling: no file larger than 2 MB may be committed to the parent
repo without explicit user approval. Large generated traces, logs,
build outputs, compressed captures, and binaries belong in gitignored
output paths or external storage.

**Never run `git clean`** in any checkout of this workspace.
`.devcontainer/` and other container-home configuration is untracked
on purpose and must not be deleted. To clean a working directory, use
`git reset --hard HEAD` only — it resets tracked files without
touching untracked files/directories. This matches the rule in
`mtg-forge-rs/CLAUDE.md`.

## Coordinator-Specific Instructions

This section is for the *orchestrating* agent — the one that spawns
child agents in worktrees. Per-agent execution rules live above; this
section covers the dispatch / coordination layer.

### General orchestration

- **One worktree per child agent.** Never co-locate two mutating
  agents in the same checkout. The Workspace Discipline rules above
  are non-negotiable.
- **Pre-flight every spawn.** Before dispatching a child:
  1. Run the worktree-vs-ACTIVE.md diff (Audit self-check above) and
     resolve any discrepancies.
  2. Verify the primary checkout is on `integration` (or `main`) and
     `git status` is clean. `new_worktree.sh` will refuse to proceed
     otherwise.
  3. Confirm the child's brief includes the explicit registration
     step ("Register branch + worktree in `worktrees/ACTIVE.md` BEFORE
     first source commit").
- **Closeout every child.** When a child reports done:
  1. Verify the worktree is clean (zero modified, zero untracked).
  2. Move the row from `ACTIVE.md` to `ARCHIVED.md`.
  3. `git -C mtg-forge-rs worktree remove worktrees/<branch>`.
  4. Leave the branch ref in place unless explicitly told otherwise.

### Orc orchestration (when applicable)

When the orchestrator is `orc`, the additional rules are:

- `orc` tracks live agents in its own state file; that state is
  REDUNDANT with `worktrees/ACTIVE.md`. The markdown registry is the
  durable source of truth — the `orc` state must be reconciled against
  it on every audit.
- `orc spawn` briefs MUST be self-contained: every brief references
  this CLAUDE.md by path so the child can re-read the discipline rules
  without depending on chat context.
- `orc` is permitted to run on the parent CWD; it never mutates the
  primary checkout directly, only via `new_worktree.sh` invocations.

### Claude Code sub-agents

When the orchestrator spawns Claude Code sub-agents (via the `Task`
tool / `Agent` tool):

- Sub-agents inherit the parent's CWD by default. Override with the
  `isolation: "worktree"` argument when the sub-agent will mutate
  source — that creates a fresh worktree automatically and is
  preferred over manual `new_worktree.sh` invocation for
  short-lived investigations.
- For long-lived feature work, manually invoke `new_worktree.sh`
  first, then point the sub-agent at the resulting worktree path.
  The auto-isolation worktree is reaped on agent completion, which
  is the wrong lifecycle for multi-session feature branches.
- Every sub-agent brief must explicitly cite this CLAUDE.md and the
  project-internal `mtg-forge-rs/CLAUDE.md` so the sub-agent knows
  the discipline rules without harness context.
- Sub-agents that file beads issues should use `mb` (the project's
  minibeads CLI), NOT the `tg` ephemeral task graph. `tg` is
  per-session orchestrator state only.
