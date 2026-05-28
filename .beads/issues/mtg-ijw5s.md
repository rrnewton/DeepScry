---
title: make validate must require a clean tree (no auto-wip commits)
status: open
priority: 3
issue_type: task
created_at: 2026-05-28T17:33:42.655377221+00:00
updated_at: 2026-05-28T17:33:42.655377221+00:00
---

# Description

make validate / scripts/validate.sh auto-creates a temporary "wip" commit when the working tree is dirty (to validate uncommitted changes), then unwinds it. User (2026-05-28) flagged these wip commits as a bad smell.

GOAL: never auto-produce temp wip commits for untracked/uncommitted changes. The convention is: AGENTS COMMIT LOCALLY BEFORE RUNNING VALIDATE, so validate always runs on a clean committed tree (= exactly what will be merged).

CHANGES:
1. scripts/validate.sh: instead of auto-wip-committing a dirty tree, REQUIRE a clean tree by default — if `git status` is dirty, fail loudly with a clear message ("commit your changes before running make validate; validate runs on the committed tree"). Optionally keep an explicit opt-in flag (e.g. --allow-dirty) for ad-hoc local use, but the default must not silently create wip commits.
2. The validate_logs SHA should then always be the real commit SHA (no more `..._dirty.log`).
3. Document the "commit before validate" convention in CLAUDE.md (Pre-Commit section) and in the agent-facing skills (compatibility_tracking / targeted_compatibility / mtg-rules-review) so every dispatched agent commits first.

WHY IT MATTERS beyond aesthetics: the wip-commit dance confounded coordinator stall-detection (a worktree HEAD showing `wip` looked like a stalled agent) and produced `..._dirty` validate-log SHAs that don't match the merged commit. A clean-tree requirement makes validation reproducible and the merge artifact unambiguous.

Priority: medium (process hygiene; not blocking). Touches scripts/validate.sh + CLAUDE.md + skill docs.
