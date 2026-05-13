# Old Branch History

This document captures institutional knowledge about feature branches that
either still hold unmerged work or have been recently archived/deleted.
Use it as the first stop when wondering "whatever happened to branch X?"
before spelunking through `git reflog` or remote refs.

Conventions:
- "Last commit" hashes are short SHAs from the remote tip at time of writing.
- Dates use the committer timestamp.
- "Tagged X.vN" means the branch was archived as a Git tag (`git tag X.vN`)
  and the branch ref was deleted, per the `ARCHIVE` policy in CLAUDE.md.

Last refreshed: 2026-05-13.

---

## UNMERGED

Branches that still exist on `origin` and contain commits not in
`origin/integration`. These are candidates to either revive, salvage
selected commits from, or archive + delete.

### `undo-optional`
- **Last commit:** `0edd0084` — `feat(undo): Add compile-time feature flag to disable undo logging` (2025-11-29)
- **Status:** 1 commit ahead of integration.
- **What it does:** Adds a Cargo feature flag that compiles out the undo
  log machinery entirely. The undo log is one of the larger per-action
  allocation sources, so being able to A/B compile with it disabled is
  valuable for benchmarks: it isolates undo's cost from other engine
  costs and gives a clean upper bound on simulation throughput.
- **Why unmerged:** The default product needs undo (TUI rewind, replay
  resume, MCTS rollouts), so the feature flag is a benchmark-only knob.
  We have not yet decided whether to keep it as a permanent build option
  or fold it into a more general "engine variants" build matrix.
- **Suggested disposition:** Keep alive for now; re-base periodically so
  we can flip it on for `cargo bench` runs.

### `fontsize-research`
- **Last commit:** `be53f646` — `prototype(web): scalable TUI font for fancy.html via innerWidth shim` (2026-05-09)
- **Status:** 1 commit ahead of integration. Open as **PR #8**.
- **What it does:** Research prototype for a scalable TUI font in
  `fancy.html`. Uses an `innerWidth` shim to drive font sizing so the
  ratatui-rendered TUI can scale to large screens / Retina without the
  characters becoming a postage-stamp-sized grid.
- **Why unmerged:** Still under PR review; we wanted to see if there
  was a cleaner approach via CSS container queries before committing
  to a JS shim.
- **Suggested disposition:** Land or close PR #8 — do not let it
  bit-rot indefinitely.

### `stop-replay`
- **Last commit:** `e07adea7` — `Merge branch 'main' into stop-replay` (2025-10-27)
- **Status:** Tagged `stop-replay.v1`. Branch still exists on origin.
- **What it does:** Introduced a `ReplayController` that could
  deterministically resume a game from a snapshot, intended for
  midturn-stop / midturn-resume scenarios.
- **Why unmerged:** The replay/resume capability ended up integrated
  through a different mechanism (the action-log + deterministic
  re-simulation path used by the network layer and the rewind/undo
  flows). The `ReplayController` abstraction itself was not adopted.
- **Suggested disposition:** Tag is the canonical record; the branch
  itself can be deleted whenever convenient. Keep `stop-replay.v1` tag
  forever as the audit trail.

### `allocator`
- **Last commit:** `b9a37a9a` — `WIP: Parameterize GameState with allocator API (Phase 2 in progress)` (2025-11-06)
- **Status:** 10 commits ahead of integration. Sibling branches
  `allocator-phase2-profiling` and `allocator_two` (tagged
  `allocator_two.v1`) explored follow-on directions.
- **What it does:** Per-thread bump allocators for engine storage with
  bounded lifetimes (turn / phase). The hypothesis: most engine
  allocations live for at most one turn or phase, so a per-thread
  `Bump` (from `bumpalo`) reset at the appropriate boundary should
  drastically cut allocator pressure relative to the global allocator,
  and beats `tcmalloc` / `jemalloc` for this allocation pattern.
- **Approach (visible in commit log):**
  1. Add an `Allocator API` shim around `bumpalo::Bump` so call sites
     can name "the current arena" without depending on `bumpalo`
     directly (`c5380c53`).
  2. Verify Rust's nightly `Allocator`-aware `Box<T, A>` works with the
     wrapper (`b4bc14f9`).
  3. Decision: keep `SmallVec` strategy unchanged; SmallVec inline
     storage and the bump arena are complementary, not redundant
     (`5a720638`).
  4. Begin parameterizing engine collections — `CardZone`, `PlayerZones`
     — over the allocator (`9c199c4c`).
  5. Phase 2 WIP: parameterize the full `GameState` over the allocator
     (`b9a37a9a`). This is where the branch was paused.
- **Supporting docs:** Allocation site analysis was committed on the
  branch (`b2e1ff12`); tracking issue `mtg-151` was opened to drive the
  rollout (`ef8d3f00`).
- **Why unmerged:** Phase 2 is genuinely WIP — the allocator parameter
  threads through nearly every engine type once it reaches `GameState`,
  which is a large invasive change with non-trivial generic-bound
  surface. Needs a focused work block to either finish or split into
  smaller mergeable pieces.
- **Suggested disposition:** Either resume the parameterization (with
  `mtg-151` as the tracking issue) or salvage the infrastructure
  commits (`c5380c53`, `b4bc14f9`, `b2e1ff12`) into integration as
  groundwork and re-open Phase 2 against a fresh base.

---

## ARCHIVED / DELETED

Branches deleted in recent cleanup sessions. Most have already been
merged or fully cherry-picked into integration; some were dead
research that produced docs instead of code. Tags (where listed)
preserve the tip commit so the branch can always be resurrected.

### `choose-from-library-refactor`
- ~3 months old, 7 commits, all landed on integration.
- Refactored the "choose card(s) from library" controller path so
  search/tutor effects share a single code path. Subsumed by the
  later card-definition refactor and the centralized reveal logic.

### `card-definition-refactor`
- ~3 months old. A subset of `choose-from-library-refactor`'s commit
  range. Deleted as redundant.

### `avatar`, `avatar2`, `avatar3`, `avatar4`
- ~4 months old. AI-persona work (different heuristic profiles for
  different "avatar" opponents). All useful commits cherry-picked
  into integration. Tags `avatar3.v1`, `avatar4.v1`, `avatar4.v2`,
  `avatar4.v3` preserve the historical tips.

### `prompt_table01`
- ~5 months old. AI features built on a prompt-table abstraction.
  All landed on integration; branch deleted.

### `bugfix-01`
- ~5 months old. Logging cleanup + `SearchLibrary` correctness fixes.
  All landed on integration.

### `wasm`, `wasm2`
- ~5 months old. Initial WASM front-end built on RatZilla plus the
  first `WasmHumanController`. All useful code landed on integration.
  RatZilla itself has since been dropped (see `decouple-step4`
  commit `1c0941ed`); we now drive the canvas without ratzilla.

### `network2`, `wasm-network2`
- ~4 months old. The `IVar` → `MVar` architecture transition for the
  client/server message bus. All landed on integration. Tags
  `network2.v1`, `network2.v2`, `wasm-network2.v1` preserve tips.

### `network-reveal`
- ~9 weeks old. Centralized the "reveal cards to player(s)" logic so
  scry / surveil / fateseal / library-search / clash all share a
  single reveal pipeline. Landed as `db69a10a` on integration.

### `feature/bug-report-system`
- Already merged. `git patch-id` confirms identical content to a
  commit already on integration; deletion was a no-op cleanup.

### `chumsky`
- ~7 months old. Experiment using the `chumsky` parser-combinator
  crate to replace the hand-written card-script parser. The
  conclusion: the manual parser won — better error messages, better
  performance, easier to maintain given our tokenized DSL shape.
  Findings were preserved in
  `ai_docs/CHUMSKY_EXPERIMENT_RESULTS.md`. Tag `chumsky.v1`
  preserves the experiment tip.

### `native-web-gui`
- Merged into integration. Branch deleted.

### `fix-chaos-orb`
- Merged into integration. Branch deleted.

### `layout-engine`
- Merged into integration. Branch deleted.

---

## Maintenance

When archiving a new branch:
1. If the branch had real value (research notes, alternative
   implementation, paused WIP), tag it as `<branch>.v1` (or `vN+1`
   if `v1` is taken) **before** deleting the ref.
2. Add a one-paragraph entry to the appropriate section of this
   file with: last commit hash + date, what it did, why it ended,
   and what (if anything) replaced it.
3. Reference the tag in the entry so future readers can `git
   checkout <tag>` to inspect.

When promoting a branch from `UNMERGED` to `ARCHIVED / DELETED`,
move the entry, do not duplicate it.
