
MTG Forge-rs: Devolpement Guidelines
=======================================================

This document contains the development guidelines and instructions for the project. This guide OVERRIDES any default behaviors and MUST be followed exactly.

**CRITICAL**: DRY: do not duplicate code. Always test changes and present evidence.
NEVER run `git clean -fxd` in this repo or any submodule, that would destroy valuable container configuration.

See PROJECT_VISION.md for the *original* project vision (historical — it carries an out-of-date warning; OPTIMIZATION.md and README.md are the current authority for perf + design).

For build instructions, feature flags, and binary entry points, see **README.md**.

If you become stuck with an issue you cannot debug, you can file an issue for it and leave it to work on other topics. Of course, the tests should be always passing before each commit and achieve reasonably good code coverage as described below.

References
========================================

Refer to the MTG (Magic the Gathering) rules in the `./rules` directory.

 - 01_full_official_MagicCompRules_20250919
 - 02_mtg_rules_condensed_medium_length_gemini.md

You should mainly use the second, condensed summary for understanding the basic operation of the MTG game. When necessary, refer to the long rules list in the official MTG rules (the first document above).

Coding conventions
========================================

You HATE duplicated code. You follow DRY religiously and seek clean abstractions where functions are short, and complexity is factored into helpers, traits, and centralized infrastructure that is shared as much as possible. You hate duplication so much that you would rather centralize repetitive code EVEN if it means the interface to the shared functionality becomes fairly complex (e.g. the shared logic uses callbacks with complex types for the pieces that vary between use cases).

You also dislike long files. Whenever a file grows longer than 2000 lines you propose ideas for breaking it into separate modules.

PREFER STRONG TYPES. Do not use "u32" or "String" where you can have a more specific type or at least a type alias. "String" makes it very unclear which values are legal. We want explicit Enums to lock down the possibilities for our state, and we want separate types for numerical IDs and distinct, non-overlapping uses of basic integers.

Delete trailing spaces. Don't leave empty lines that consist only of whitespace. (Double newline is fine.)

Add README.md files for every major subdirectory/subsystem.  For example `src/core`, `src/game`, etc.

Follow the high-performance Rust coding conventions in **OPTIMIZATION.md** (unboxing, minimizing allocation, etc). In particular, adhere to the below programming patterns / avoid anti-patterns, which generally fall under the principle of "zero copy":

- Avoid clone: instead take a temporary reference to the object and manage lifetimes appropriately.
- Avoid collect: instead take an iterator with references to the original collection without copying.

Read OPTIMIZATION.md for more details.

SAFETY! This is a safe-rust project. We will not introduce the `unsafe` keyword unless we have a VERY good reason and with significant advanced planning.

### Network Architecture: Desync is ALWAYS Fatal

For network multiplayer code, the **deterministic sequential simulation** model in `docs/NETWORK_ARCHITECTURE.md` is inviolable. Any desynchronization between server and client is an **immediate fatal error** - never paper over desync with recovery hacks. Extra validation data in messages is for early detection only, NOT for recovering from inconsistent state.

**Controllers must be information-independent**: ALL controller types (heuristic, random, zero, etc.) MUST produce identical decisions whether running on the server (full state) or on a client (shadow state). Controllers must NEVER use hidden information (opponent hand contents, library order, RNG state). If a controller produces different gamelogs in local vs network mode, it has an information-leakage bug. See `docs/NETWORK_ARCHITECTURE.md` for details.

### NO HACKY STRING OPERATIONS ON STRUCTURED DATA

We do NOT treat structured data formats (card scripts, SVars, ability definitions) as unstructured strings. **NEVER** use substring matching like `body.contains("AB$ Mana")` or `line.contains("some keyword")` to parse structured DSL formats.

**Instead:**
1. Use proper tokenized parsing (split by delimiters like `|` and `$` FIRST)
2. Use existing parsing infrastructure: `AbilityParams::parse()` in `ability_parser.rs`
3. Query the structured result: `params.get("AB") == Some("Mana")`
4. Add new parsing utilities to centralized modules if needed

**Why this matters:**
- `contains("add")` would match "Madden", "adding", etc. (false positives)
- `contains("Damage")` would match "DealDamage", "PreventDamage", "AllDamage" (ambiguous)
- Substring checks are O(n) per call vs O(1) map lookup after tokenized parse
- Non-tokenized parsing is fragile and creates subtle bugs

**Example - BAD:**
```rust
if body.contains("AB$ Mana") {  // DON'T DO THIS
    if let Some(produced) = body.split("Produced$").nth(1) { ... }
}
```

**Example - GOOD:**
```rust
let params = AbilityParams::parse(ability)?;
if params.api_type == ApiType::Mana {
    let produced = params.get("Produced");
}
```

See `ai_docs/ability_parsing_comparison.md` for detailed analysis and `ai_docs/CARD_SCRIPT_SPEC.md` for the card script DSL specification.

Documentation and Analysis
========================================

When creating analysis documents, specifications, or other AI-generated documentation, place them in the `ai_docs/` directory. This keeps the top-level clean and makes it clear which documents are AI-generated analysis (and may become outdated) versus core project documentation.

Debugging Scripts and Temporary Files
========================================

**ALWAYS** place one-off debugging scripts and temporary test files in the `debug/` directory.

This includes:
- One-off test scripts (JavaScript, Python, Shell, etc.)
- Temporary screenshot/log analysis tools
- Quick reproduction scripts for specific bugs
- Experimental test harnesses that aren't part of the test suite

The `debug/` directory is gitignored, so you can freely create files there without polluting the repository.

**Core test scripts** that are part of `make validate` belong in their proper locations:
- `web/test_*.js` - Browser/WASM E2E tests (called by `make validate`)
- `tests/` - Rust unit and integration tests
- `examples/` - Rust example programs used for validation

If you create a temporary script in the root directory or elsewhere by mistake, move it to `debug/` immediately.

Workflow: Task tracking
========================================

We use "beads" to track our issues locally under version control. Review `bd quickstart` to learn how to use it. 

Every time we do a git commit, update our beads issues to reflect:
- What was just completed (check off items in lists, close completed task(s))
- What's next (update the in tracking issues that track the granular issues)
- Mention in the commit any new issues created to document bugs found or future work.

The beads database is our primary tracking mechanism, so if we lose conversation history we can start again from there.  You should periodically do documentation work, usually before committing, to make sure information in the issues is up-to-date.

### Beads CONVENTIONS for this project

Do NOT read or modify files inside the `./.beads/` private database, except when fixing merge conflicts in markdown files that you can read.

Prefer the MCP client to the CLI tool if available. ALWAYS `bd update` existing issues, never introduce duplicates with spurious `bd create`.

The issue prefix may be customized (`foobar-1`, `foobar-2`), but here we will refer `bd-1` as example issue names

#### Tracking issues and Priorities

Warning: Be careful to EDIT tracking issues (`bd update`) and not just
file a new duplicate issue with `bd create`.

- Issues labeled "human" are created by me and will always have 0 priority.
- Issue mtg-1, at priority 0, is the OVERALL tracking issue. It primarily references other tracking issues
  and reiterate some of these conventions. We want to keep it pretty short.

- The next tracking issues, e.g. mtg-2 and on have priority 1 and are topic-specific trackers:
  - Optimization tracking
  - MTG feature completeness: supporting keywords/abilities/complex mana and effects.
  - Gameplay feautures: like an actual TUI to play as a human.
  - Cross-cutting codebase issues: APIs (player, controller, etc), testing coverage and methodology.

 - All tracking issues refer to granular issues by name in their text, e.g. "mtg-42"
 - All other granular issues will have priority 3 to 4 unless they are seen as a critical bug, which will bump them to priority 2.

#### Mark transient information

We often record transient information, like benchmark results, that quickly gets out of date. We want to label such information so we can tell how old it is. In addition to YYYY-MM-DD, our convention is to use:
  `git rev-list --count HEAD`
which prints out the number of commits in the repo (or equivalently the ./scripts/gitdepth.sh script), and then format the timestamp as `YYYY-MM-DD_#DEPTH(387498cecf)` e.g. `2025-10-22_#161(387498cecf)`. That's our full timestamp for any transient information that derives from a specific commit.
Sometimes this requires us to split our commits into (1) functionality and then (2) documentation-update.

#### Reference issues in code TODO

We don't want TODO items to be in floating code alone. For anything but the most trivial TODOs, we adopt the convention of referencing issues that tracks the TODO:

```
// TODO(mtg-13): brief summary here
```

Then, the commit that fixes the issue both removes the comment and closes the issue in beads.

#### Use description field only, not notes

When creating or updating issues with `bd`, always put ALL content in the description field. Do NOT use the --notes field, as it creates duplication and confusion between what's in description vs notes. Keep all issue information consolidated in the description field only.

#### Issue IDs: hash on worktrees, numeric on integration

Minibeads supports both **hash-based** IDs (`mtg-a1b2c3`, content-derived) and
**numeric** IDs (`mtg-171`, sequential). Hash IDs let many agents file issues in
parallel worktrees without colliding; numeric IDs are nicer to read and cite but
require a serialization point to assign without conflict. We use BOTH, with the
**integration branch / primary checkout as the serialization point**:

- **`.beads/config-minibeads.yaml` keeps `mb-hash-ids: true`** — so every NEW
  issue (anywhere, including worktrees) is born hash-based and parallel-safe.
- **On a worktree:** just `bd create` (hash ID) and commit the hash-named file.
  Do NOT run `mb mb-migrate` in a worktree — let integration renumber later.
  Your branch will reference hash IDs; that is expected and fine.
- **On the primary checkout, before committing a `.beads` change to
  `integration`:** run the renumber, then restore the hash setting, then stage
  the whole `.beads` dir:
  ```sh
  mb mb-migrate --dry-run --to numeric      # inspect first
  mb mb-migrate --to numeric                # renames hash files -> numeric, rewrites cross-refs
  # mb-migrate flips mb-hash-ids -> false; we WANT it true for future parallel filing:
  sed -i 's/^mb-hash-ids: false/mb-hash-ids: true/' .beads/config-minibeads.yaml
  git add .beads                            # whole dir: renamed files + ref rewrites + config
  ```
  This converts any hash IDs that arrived via merged feature branches into the
  next sequential numbers and keeps `git log`/issue citations readable.
- **Timing (orchestrator):** only renumber when **no in-flight feature branch is
  touching `.beads`** (do it right after a wave of card/feature branches merges,
  before dispatching the next wave). Renumbering while a branch has edits to a
  hash-named issue file causes modify/delete rebase conflicts (integration
  renamed `mtg-2b3951.md`→`mtg-393.md`; the branch still edits `mtg-2b3951.md`).
- A hash ID cited in a commit message or code TODO before renumbering still
  resolves via the renamed file's history; prefer citing issues by title when the
  reference must survive a renumber.


Workflow: Commits and Version Control
================================================================================

Commit to git in small, coherent units (see the Pre-Commit and Branches sections below).
Our submodules in this directory should stay pinned to the latest upstream branch:
 - forge-java: master branch
 - .claude_template: mtg-rs branch
Resolve any conflicts by just always taking the upstream latest for these branches.

Clean Start: Before beginning work on a task
--------------------------------------------

Make sure we start in a clean state. Check that we have no uncommitted changes in our working copy. Perform `git pull origin <BRANCH>` to make sure we are starting with the latest version on our branch. Check that `make validate` passes in our starting state.

If github MCP is configured and github actions workflows exist for this project, check the github actions CI status for the most recent commit and make sure it not red (if it's still pending, ignore and proceed). If
there's a CI failure, then fixing THAT becomes our task. Finally, check that `make validate` passes locally in our starting state.

Pre-Commit: checks before committing to git
--------------------------------------------

**MANDATORY first step before EVERY commit:** run `cargo fmt --all` (or
`make fmt`) so the working tree is formatted, then `cargo fmt --all -- --check`
to confirm the diff is clean. CI runs `cargo fmt --all -- --check` against the
**nightly** toolchain (see `.github/workflows/ci.yml`'s `fmt` job) and a single
mis-formatted line will turn the build red. This has been a repeated source of
CI failures — do **NOT** skip it. The same check is wired into `make validate`
as `validate-fmt-step`, but you should also run it explicitly so you catch
formatting drift before launching the rest of validation.

If your toolchain has nightly available (`rustup toolchain list`), prefer
`cargo +nightly fmt --all -- --check` to exactly match what CI runs.

A tracked git pre-commit hook lives at `scripts/git-hooks/pre-commit` and runs
the same fmt check on staged `.rs` files. A tracked pre-push hook at
`scripts/git-hooks/pre-push` runs the full
`cargo clippy --all-targets --all-features --features network -- -D warnings`
that CI runs, so a push that would fail CI's clippy job is blocked locally
first. Install both once per clone with `make install-hooks` (also part of
`make setup`); the bypass for either is `--no-verify` (do not make a habit
of it).

Then run `make validate` and ensure that it passes or fix any problems before
committing.

Also include a `Test Results Summary` section in every commit message that summarizes how many tests passed of what kind.

If you validate some changes with a new manual or temporary test, that test should be added to either the unit tests, examples, or e2e tests and it should be called consistently from both `make validate` and Github CI.

NEVER skip tests in CI. If a test cannot run due to missing dependencies (submodules, tools, data files), fix the CI configuration to provide those dependencies. Tests must hard-fail (`exit 1`) on missing prerequisites, never gracefully skip (`exit 0`).

**CI and `make validate` MUST NOT depend on any deployed environment** (e.g. `deepscry.net` or any cloud VM). They run hermetically against the local checkout only. Tests that exercise a *live deployment* (e.g. `web/smoke_test_live.js`, `tests/remote/*.sh`) are a SEPARATE category: they are invoked manually or as a post-deploy step, never wired into `make validate`, the `validate-*-step` targets, the `tests/*.sh` auto-discovery glob, or the GitHub CI workflow. Keep remote/live-VM smoke tests under `tests/remote/` (not auto-discovered) and out of the explicit e2e file lists.

NEVER add binary files or large serialized artifact to version control without explicit permission. Always carefully review what you are adding with `git add`, and update `.gitignore` as needed.

If the commit is about optimization, refresh the benchmark results as well with `./scripts/run_benchmark.sh`

Post-commit: refreshing benchmark results
----------------------------------------

Run `./scripts/periodically_run_benchmarks.sh` (which calls `run_benchmark.sh` if 5+ commits since last recorded benchmark). If it modifies the working copy (specifically, `experiment_results/<CPU>/perf_history.csv`), make an extra git commit that describes the result.

**Official benchmark entrypoint**: `./scripts/run_benchmark.sh` - this runs benchmarks AND records results to CSV. Never call `cargo bench` directly for tracked performance measurements.

Branches and pushing
----------------------------------------

You may push after validation and can check CI status with github MCP. Don't force push unless you're asked to or ask permission.

**MANDATORY: MTG rules review for bug fixes.** Every bug fix — regardless of where it originated (fuzz testing, user report, tournament discovery, differential testing against Forge-Java) — MUST pass an MTG Comprehensive Rules compliance review before merging into `integration` (and therefore before any promotion to `main`). The review is documented in `.claude/skills/mtg-rules-review/SKILL.md` and produces an explicit `PASS`/`CONCERN`/`FAIL` verdict block in the PR description / commit message. A `FAIL` verdict blocks the merge; a `CONCERN` verdict requires a linked beads follow-up issue. This is in addition to (not a replacement for) `make validate` and the fmt check.

**IMPORTANT: The `main` branch is protected.** Do NOT merge directly to main. We use a three-tier branch structure:
- **main**: Stable branch - only receives merges from `integration` after CI passes
- **integration**: Staging branch - receives merges from feature branches with green CI, or direct commits when working on integration branch.
- **Feature branches**: Active development on specific features (e.g., `avatar4`, `network2`)

To get changes into main, use the `ci-integration-monitor` agent (see `.claude/agents/ci-integration-monitor.md`) which handles:
1. Checking CI status on feature branches
2. Merging green feature branches into `integration`
3. Running local validation on `integration`
4. Promoting `integration` to `main` after CI passes

ARCHIVE completed feature branches. Upon merging a feature branch X, archive it as tag `X.v1` or `X.(N+1)` if that tag is taken.

When merging, archiving, or deleting a feature branch, also update `ai_docs/OLD_BRANCH_HISTORY.md` with a brief note describing what the branch contained and its disposition (merged into integration/main, archived as tag, deleted, etc.). This preserves institutional memory about historical work even after the branch ref is gone.

**CRITICAL**: NEVER use `git clean` commands (`git clean -f`, `git clean -fd`, `git clean -fxd`, etc.) in this repository. The `.devcontainer/` directory contains valuable container home directory configuration that must not be deleted. To clean working directory, use ONLY `git reset --hard HEAD` which resets tracked files without removing untracked files/directories.

But make sure we do NOT have a dirty working copy in terms of `git status`. Any accidental untracked files must be (1) properly tracked, or (2) gitignored. We don't want untracked files hanging around.


Commit message documents relationship to original Java version
--------------------------------------------------------------

Finally, also before committing reanalyze the relationship between (1) what you built and (2) the existing Java implementation, and summarize it. It's ok for the Rust and Java versions to deviate, but there should be a reason for it and we should document it in these commit messages.

```
## Relationship to Java Forge

- this Rust reimplementation does X
- the upstream Java version does Y
```

Commit message justifies game play logic with real games
--------------------------------------------------------

Except for purely internal fixes that don't directly affect MTG gameplay, in every commit you will need to justify changes with real gameplay logs. Add a section to the commit message which provides evidence for the correct behavior of the fix in the form of a log snippet from a real `agentplay/*.sh`/`mtg tui` CLI game, ideally with a runnable reproducer CLI command.

- We will reason about the behavior of the game in terms of the log messages of game actions.
- Compare against the rules of MTG (and cite the rule numbers where applicable). Keep an eye out for for missing behaviors, contradictory information, or impossible events.
- In the case of AI, consider whether the player actions make a basic level of sens.

Runnable commands included in the message should refer to actual `.dck` files in the repository so that the user can indeed reproduce them and see the logs cited.

See the file `docs/HOWTO_AGENTPLAY+REPRODUCERS.md` for instructions on playing the game as an agent to observe engine behaviors without writing new code.

