# ai_docs/ — AI-generated documentation

AI-generated analysis, specs, designs, and reports live here (kept out of the
repo top level). Everything is bucketed into one of three subdirectories by
**lifecycle**, not topic. When you add or touch a doc, put it in the right
bucket; when a transient doc's work is done, move it to `archived/`.

## `reference/` — evergreen, kept up to date

Living documents that describe how things **currently** work and should track
reality: format specs, grammars, architecture docs, and maintained logs. If the
code changes such that one of these is wrong, **fix the doc** — don't archive it.
Examples: `CARD_SCRIPT_SPEC.md`, `PZL_GRAMMAR.md`, `CONTROLLER_DESIGN.md`,
`UI_ARCHITECTURE.md`, `snapshot_architecture.md`, `OLD_BRANCH_HISTORY.md`.
Some of these are cited by `CLAUDE.md` and source comments — keep those links
pointing at `ai_docs/reference/...`.

`reference/rules` is a symlink to the repo's top-level `../rules/` corpus
(official + condensed MTG Comprehensive Rules) so the canonical rules sit
alongside the other evergreen references.

## `transient/` — current work, archive when done

Docs describing in-flight work, experiments, or the current state of the world
that will become stale once the work lands. **Most of this should be a minibeads
issue instead** — only write a full doc here when an agent's work genuinely
warrants a long-form report beyond what fits in an issue. When the work
completes, move the doc to `archived/` with a date tag (or delete it if a
minibeads issue already captures the outcome).

## `archived/` — dated point-in-time snapshots

Frozen records of past investigations, analyses, experiment results, audits, QA
reports, and superseded plans. **Never updated.** Filenames carry a `YYYYMMDD`
(or `YYYY-MM-DD`) tag marking the snapshot date, e.g.
`AI_PORTING_ANALYSIS_20251026.md`. Useful as institutional memory; assume the
contents are out of date relative to current code.

## Adding a doc

- Spec / grammar / current-architecture / maintained log → `reference/`.
- A report on work you're doing right now → prefer a minibeads issue; if a
  full doc is warranted, `transient/`.
- A finished point-in-time analysis/report → `archived/NAME_YYYYMMDD.md`.
