# Parent Workspace Agent Guide (deepscry)

This is the **dev harness** around the `deepscry` project. It is NOT a
code project. It is the meta-workspace that contains the primary
checkout, private agent worktrees, experiment captures, ai_docs, scratch
output, and harness tooling. Keep the parent directory clean: every file
must be tracked, committed, or `.gitignored` before handoff.

The harness itself is versioned inside the project at
`deepscry/multiagent_workspace/`. This file (`CLAUDE.md`) is
symlinked from the parent dir to `deepscry/multiagent_workspace/CLAUDE.md`
by `multiagent_workspace/install.sh`. Edit the in-repo copy, not the
symlink target — changes there flow back to every machine via `git pull`.

## Communicating in plain language (no jargon drift)

When reporting to the user — and when agents report up to the
orchestrator — explain in plain language. Internal shorthand, issue
IDs, and acronyms drift toward a private dialect that locks the user
out of their own project. Counter it actively:

- **Expand on first use.** The first time a term, issue ID, or acronym
  appears in a user-facing message, gloss it in plain words: "the
  `action_count` exclusion (a temporary workaround that stopped
  comparing the two sides' action counters)". Bare `mtg-610`, `4a`,
  `un-excluded`, `dual-ac-stamp`, `L1–L4` with no gloss is a defect.
- **Lead with the plain point, IDs after.** Say what happened and why
  it matters in words a smart non-specialist could follow; attach the
  precise issue IDs / commit SHAs afterward for traceability, not
  instead of the explanation.
- **Prefer description to code-name:** "the reveal-ordering rework
  (4a)" beats "4a"; "the build-once CI restructure (mtg-717)" beats a
  bare "mtg-717".
- **Orchestrator translates.** When relaying an agent's report to the
  user, translate its shorthand first — never pass a wall of internal
  codenames straight through.

This is load-bearing for trust: the user cannot steer what they cannot
read. A report the user has to ask you to decode is a failed report.

## Vocabulary

These terms recur throughout this guide; agents must use them
consistently.

- **`parent`** — the dev harness root (typically `~/work/mtg/` or
  `~/working_copies/mtg/`; the two paths resolve to the same tree via
  the `~/work → ~/working_copies` symlink). When the user says
  "parent" they mean this dev harness, NOT any git "parent" commit and
  NOT the deepscry project repo.
- **`primary checkout`** — `parent/deepscry/`, the canonical
  deepscry clone. Acts as the **donor** for reflink-cloned
  `target/` trees in every new worktree. Treat it as a build-cache
  reservoir, not a development workspace.
- **`worktree`** — an agent-private deepscry checkout under
  `parent/worktrees/<branch>/`. All mutating work happens here, never
  in the primary checkout.
- **`deepscry`** — the GitHub project. Remote `origin` points at
  the OSS upstream. The three-tier branch structure is `feature
  branches` → `integration` → `main`; see `<RepoRoot>/CLAUDE.md` for
  the merge ceremony.

## Repo Boundaries & Layout

The parent directory is the orchestrator's CWD and coordination
workspace. Spawned agents normally start in a private worktree under
`worktrees/`.

Top-level layout:

```
parent/                       (= ~/work/mtg/)
├── CLAUDE.md                 → deepscry/multiagent_workspace/CLAUDE.md (symlink)
├── .claude/                  → deepscry/multiagent_workspace/.claude/ (symlink)
├── scripts/
│   └── new_worktree.sh       → deepscry/multiagent_workspace/scripts/new_worktree.sh (symlink)
├── worktrees/
│   ├── ACTIVE.md             (registry of live agent worktrees)
│   ├── ARCHIVED.md           (historical log)
│   └── <branch-name>/        (each live worktree)
├── deepscry/             (primary checkout)
├── ai_docs/                  (transient AI scratch notes; optional)
├── experiments/              (captured experiment data; optional)
├── scratch/                  (loose binaries, profiling output; optional)
└── .git/                     (local-only parent repo; usually no remote)
```

Project boundaries:

- `deepscry/` is the actual project. Durable feature work, tests,
  architecture docs, and **beads (`mb`) issues** live there. See
  `<RepoRoot>/CLAUDE.md` for project-internal rules — the coding
  conventions, DRY principles, no-clone/no-collect performance rules,
  the **No Hacky String Operations On Structured Data** rule, and the
  three-tier branch discipline.
- Parent-repo commits (if the parent has a remote at all) are for
  workspace policy, experiment artifacts, reports, and harness-level
  tooling. Do not commit transient source changes here that belong in
  a project checkout.

Rule of thumb: if it is durable source or project documentation for a
fresh clone, put it in `deepscry/` (the project). If it is
investigation state, experiment data, or harness coordination, keep it
in the parent workspace.

## Workspace Discipline

Every agent that writes code gets a unique worktree. Do not let multiple
mutating agents share the same checkout.

Worktree naming:

- Directory: `parent/worktrees/slot<NN>/` — use **opaque slot names**
  (`slot01`, `slot02`, ...) NOT branch-derived names. Slot names are
  permanent identifiers for the physical directory; the branch checked
  out inside can change freely (and is tracked in `ACTIVE.md`).
- Branch:    `<branch>` (any descriptive feature-branch name)
- **Why slots, not branch names:** branch-derived directory names cannot
  be safely `mv`-renamed (the worktree's `core.worktree` and the shared
  `forge-java` gitdir rewrite both break on rename — see the
  `feedback_no_mv_rename_worktrees` memory). Opaque slot directories
  stay put; only the branch inside them changes. This also makes moving
  the whole workspace to a new partition safe: you can copy or reflink
  the slot directories without git repair.
- Pick the next available slot number; record it in `ACTIVE.md` before
  the agent's first commit.
- The primary checkout (`deepscry/`) is for integration work only:
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
  `integration`; see `<RepoRoot>/.claude/skills/mtg-rules-review/SKILL.md`.

Push policy:

- **Pushing is allowed without per-push user confirmation** in two
  cases:
  1. From the primary checkout (`parent/deepscry/`), push
     `integration` (and `main`, when promoted) after a green
     `cargo build --release --features network` and `make validate`.
  2. From an agent worktree (`parent/worktrees/<branch>/`), push its
     own feature branch to `origin/<branch>` once the work is
     **completed** — i.e. the worktree is clean, validation passes,
     and the branch is ready to be merged (or reviewed). Do not push
     mid-task WIP without explicit user approval.
- `main` is protected; never push directly to `main`. Promotion to
  `main` goes through the integration ceremony described in
  `<RepoRoot>/CLAUDE.md` → "Branches and pushing".
- `--force` / `--force-with-lease` pushes always require explicit user
  approval, regardless of branch.

Validation proof (MANDATORY for completion):

Every agent must run a **full** `make validate` to completion in its
worktree before reporting "done" or pushing. The orchestrator MUST
verify this. Past failures (most recently mtg-472 / fix-mtg-472)
have all started with an agent skipping `make validate` and
fabricating a "Test Results Summary" of partial cargo invocations.
Without `-D warnings` and `--all-targets`, ad-hoc cargo commands miss
lints CI will catch.

Concrete rules:

1. **Required artifact**: a successful `make validate` writes
   `validate_logs/validate_<SHA>.log` and updates the
   `validate_logs/validate_latest.log` symlink. **No artifact, no
   "validate passed" claim.** Agents must cite this path in their
   "Test Results Summary" or explicitly explain why it could not run
   (and the orchestrator should treat that as a blocker, not a "ship
   it anyway" license).
2. **No watered-down clippy**: `cargo clippy --features network --lib
   --bins` (no `-D warnings`, no `--all-targets`) is NOT a substitute.
   CI runs `cargo clippy -p mtg-engine --all-targets --all-features
   --features network -- -D warnings`; the agent's local check must
   match (or just run `make clippy`).
3. **Submodule init**: `new_worktree.sh` now initialises both
   submodules automatically (`.claude_template` via plain
   `git submodule update --init`, and `forge-java` via reflink-clone +
   shared-modules-dir gitdir rewrite). A fresh worktree from
   `new_worktree.sh` starts with `git submodule status` clean. If you
   create a worktree by other means, run
   `git submodule update --init --recursive` yourself — otherwise
   `scripts/validate.py` bails with "Submodule changes detected".
4. **Orchestrator verification**: before ff-merging a feature branch,
   check `validate_logs/validate_<last-commit-sha>.log` exists on the
   branch (or in the agent's worktree, copied to the parent in the
   final report). If absent, do not merge — re-dispatch with the
   missing-artifact note, OR run `make validate` on the branch
   yourself from the primary checkout.
5. **One last "this is the artifact" line** in every agent brief:
   "Your final report MUST cite the path to
   `validate_logs/validate_<sha>.log` proving `make validate` passed.
   If you cannot produce this file, do NOT push your branch and
   surface the blocker instead."

Linear history (MANDATORY):

- **Always rebase the feature branch onto the latest `integration`
  before merging, then fast-forward merge.** Never use `git merge
  --no-ff`. Merge commits clutter `git log --oneline` and make
  bisecting harder.
- Mechanical sequence for landing any agent feature branch:
  ```sh
  # In the agent worktree
  git fetch origin
  git rebase origin/integration
  # Resolve any conflicts, re-run validate, push the rebased branch
  git push --force-with-lease origin <branch>   # explicit user OK ahead of time for this case
  # In the primary checkout
  git fetch origin
  git merge --ff-only origin/<branch>
  git push origin integration
  ```
- The `--ff-only` flag will REFUSE to create a merge commit. If the
  ff-only merge fails, it means the feature branch wasn't rebased
  onto the latest `integration` — rebase, don't fall back to
  `--no-ff`.
- The single exception is the rare case where you genuinely want to
  preserve a "this is one logical feature" boundary in the history
  (e.g. promoting `integration` → `main`). For that case, use
  `--no-ff` *with explicit user approval per merge*, not by default.
- Force-with-lease on the feature branch after rebase is fine
  (covered by the standing "push your own feature branch on
  completion" allowance above). Force-push to `integration` or
  `main` is never allowed without explicit user approval.

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

1. **Clean start.** Use
   `./scripts/new_worktree.sh slot<NN> --branch <branch>` from the
   parent directory (the first positional is the OPAQUE slot directory;
   `--branch` names the branch — created off the base, or ATTACHED if it
   already exists). The script (a) fetches `origin` in the primary,
   (b) verifies the primary builds green, (c) cleans the donor
   `target/` via `cargo sweep`, (d) creates the worktree under
   `worktrees/slot<NN>/`, (e) reflink-clones `target/` into it. Then
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
     <( git -C deepscry worktree list --porcelain \
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
(`deepscry/`), AND every agent worktree (`worktrees/<branch>/`).
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

**Use `./scripts/archive_worktree.sh <slot>` — it mechanizes and ENFORCES
steps 1–3 below** (refuses while the worktree is dirty, refuses while
`ACTIVE.md` still lists the slot, prints the ready-to-paste `ARCHIVED.md`
row, then runs `git worktree remove --force`). The manual steps remain
the contract it enforces:

1. Verify the worktree has no uncommitted changes and `git status` is
   clean, including no untracked files. If either check fails, surface
   it to the user or orchestrator. Never silently discard work.
2. Move the entry from `worktrees/ACTIVE.md` to `worktrees/ARCHIVED.md`,
   keeping the description and adding the archive date and final SHA.
3. Remove the git worktree:
   `git -C deepscry worktree remove --force worktrees/<slot>`.
   The `--force` flag is REQUIRED because the worktree contains
   submodules; without it git refuses with "contains a .git directory".
4. Delete the local branch only if it has merged into a tracked branch
   or the user explicitly approves deletion.
5. Confirm no data was lost. Reachable commits must remain available
   from refs, or be explicitly covered by the rollback/recovery plan.

### Worktree cleanup — DO NOT deinit submodules

**CRITICAL FOOTGUN:** never run `git submodule deinit -f --all` (or
`git submodule deinit -f <name>`) inside a worktree before removing
it. The deinit command nukes `.git/modules/<name>/...`, but that path
is **shared across every worktree and the primary checkout**
(`new_worktree.sh` deliberately reflinks `forge-java`'s working tree
and points its `.git` file at the shared modules dir to save ~543 MB
per worktree). Deinit in one worktree breaks `forge-java` in the
primary checkout and every other live worktree simultaneously —
recovery requires `git submodule update --init --force forge-java` in
each affected checkout.

Correct teardown sequence for a worktree:

```sh
# From the primary checkout (or any other worktree):
git -C deepscry worktree remove --force worktrees/<branch>
# That's it. No deinit. git worktree remove handles per-worktree
# submodule gitdirs under .git/worktrees/<branch>/modules/ (for
# .claude_template) automatically, and leaves the SHARED
# .git/modules/forge-java untouched (which is what we want).
```

If you ever genuinely need to recover from a corrupted shared
forge-java modules dir:

```sh
cd deepscry
rm -rf forge-java
git submodule update --init --force forge-java
# Then re-point every worktree's forge-java/.git file:
for wt in ../worktrees/*; do
    [ -d "$wt/forge-java" ] || continue
    echo "gitdir: $(pwd)/.git/modules/forge-java" > "$wt/forge-java/.git"
done
```

## CWD Protocol

- Orchestrator CWD: parent workspace root (`~/work/mtg/`).
- Mutating agent CWD: `worktrees/<branch>/`.
- Read-only agent CWD: normally the primary checkout `deepscry/`
  (treat as read-only) or a clearly marked read-only worktree.
- Task instructions may direct outputs outside the agent CWD, such as
  `ai_docs/` or `experiments/`. Follow those destinations exactly.
- Durable documentation, when agreed with the user, goes in the
  project's canonical docs directory: `<RepoRoot>/docs/` or
  `<RepoRoot>/ai_docs/`.

## Task Tracking

Two systems coexist and do not auto-sync:

- **`mb` (minibeads)** — the PRIMARY, version-controlled task store
  inside `<RepoRoot>/.beads/`. Issues are durable project state.
  Run `mb` from inside the project / a worktree. Read `bd quickstart`
  (the upstream `bd` CLI is the underlying tool) for the workflow.
  Conventions (mirrored from `<RepoRoot>/CLAUDE.md`):
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
  of deepscry under test, command line, seeds used, host info).
- CSVs must have headers and consistent columns.
- Reports must cite source files and commands for every number.
- Do not hand-write data tables. Generate them from captured data with
  scripts.

Use the project's transient-information stamp convention for any
result that derives from a specific commit (see
`<RepoRoot>/CLAUDE.md` → "Mark transient information"):

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
`<RepoRoot>/CLAUDE.md`.

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
  2. **Diff-gate the branch before ff-merge.** Run `git -C deepscry
     diff --stat integration...<branch>` and scan the file list. REFUSE
     to merge (send back for fixing) if the branch adds **tracked image
     files** (`*.png/*.jpg/*.gif/*.webp/...`, outside `cardsfolder/`),
     other binaries, or any file > the 2 MB ceiling without explicit
     user approval. QA/screenshot output belongs in gitignored `debug/`
     or `scratch/`, never tracked — see the "NEVER commit images" rule
     in `<RepoRoot>/CLAUDE.md`. A green `make validate` does NOT excuse
     a polluting diff; the orchestrator owns this gate.
  3. Move the row from `ACTIVE.md` to `ARCHIVED.md`.
  4. `git -C deepscry worktree remove worktrees/<branch>`.
  5. Leave the branch ref in place unless explicitly told otherwise.

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
  project-internal `<RepoRoot>/CLAUDE.md` so the sub-agent knows
  the discipline rules without harness context.
- Sub-agents that file beads issues should use `mb` (the project's
  minibeads CLI), NOT the `tg` ephemeral task graph. `tg` is
  per-session orchestrator state only.
- When using the Agent tool, always set `mode: "bypassPermissions"`.

## Web Frontend Layout (landing page + lobby)

The deployed web frontend at `parent/deepscry/web/` is structured as:

- `web/index.html` — public **landing page + lobby**. Explains the
  project, collects a username, connects to the native Rust `mtg
  server` over WebSocket (`ListGames` / `CreateGame` / `JoinGame` from
  `mtg-engine/src/network/protocol.rs`), and offers launch buttons that
  redirect into a game-page UI with
  `?lobby=...&game=...&pass=...&name=...&ws=...` query params for the
  downstream game page to consume.
- `web/demo.html` — the original WASM AI-vs-AI engine demo (was
  previously `index.html`). Still linked from the landing page.
- `web/tui_game.html` — fancy terminal-style WASM game page (formerly
  `fancy.html`).
- `web/native_game.html` — card-style native web GUI page (formerly
  `game.html`).
- `web/server-config.js` — small JS shim exporting
  `window.MTG_WS_URL`. **Generated at deploy time** by
  `scripts/deploy-cloud.sh deploy` from the values in the local config
  file (`<parent>/.deepscry-deploy.env`).

## Deploy script

`scripts/deploy-cloud.sh` is the canonical deploy entry point. It has
TWO phases:

- `scripts/deploy-cloud.sh config` — bootstraps a VM. Run once per
  VM (or whenever infra changes). Idempotent. Installs the systemd
  unit (defaults to `--mode user` so no root is needed for the
  deploy phase), writes the env file, opens the firewall port, and
  cleans up legacy tmux sessions / systemd units from older deploys.
- `scripts/deploy-cloud.sh deploy` — runs on every code change.
  Rebuilds WASM artefacts and the release `mtg` binary locally,
  rsyncs `web/`, `cardsfolder/`, and the binary, then restarts the
  systemd service. Does NOT require root.

  The deploy build uses the **`release-deploy` cargo profile** (defined
  in the workspace `Cargo.toml`): strip + `lto = "fat"` + `panic =
  "abort"` produce a ~25 MB binary suitable for rsync to a VM. Local
  profiling work continues to use `cargo build --release`, which keeps
  full debug symbols (~430 MB) for flamegraphs / `perf` / `samply`.
  Cargo profiles cannot enable features, so the deploy script passes
  `--features network` explicitly on the build invocation.

**No hardcoded site values.** Username, hostname, ports, TLS paths,
and the systemd unit name all come from one of:

1. Local config file `<parent>/.deepscry-deploy.env` (gitignored —
   see `scripts/deepscry-deploy.env.example` for the template).
2. CLI flags (`--user`, `--host`, `--port`, `--service`, ...).
3. Environment variables (`REMOTE_USER`, `REMOTE_HOST`, ...).

CLI > env > config file > built-in defaults.

Typical first-time setup:

```sh
# In the parent workspace:
cp deepscry/scripts/deepscry-deploy.env.example .deepscry-deploy.env
$EDITOR .deepscry-deploy.env                     # fill in REMOTE_USER + REMOTE_HOST
deepscry/scripts/deploy-cloud.sh config      # bootstrap the VM (once)
deepscry/scripts/deploy-cloud.sh deploy      # ship the code (repeat as needed)
```
