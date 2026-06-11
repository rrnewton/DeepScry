---
title: 'GUARD: reject duplicate top-level YAML keys in .beads issue frontmatter (mtg-742 corruption class)'
status: open
priority: 2
issue_type: task
created_at: 2026-06-11T07:33:32.281500982+00:00
updated_at: 2026-06-11T07:33:32.281500982+00:00
---

# Description

GUARD against duplicate top-level YAML frontmatter keys in .beads/issues/*.md (the mtg-742 corruption class).

PROBLEM (recurred 4x in one night during release ceremonies):
Beads issue files start with a YAML frontmatter block delimited by '---'. Several feature branches stamp the SAME tracker issue (notably mtg-742, the R2 deck-storage tracker) via 'mb update', which rewrites the top-level 'updated_at:' line. When two such branches merge, git's 3-way TEXT merge cannot tell that both sides changed the same logical key, so it keeps BOTH 'updated_at:' lines. The resulting duplicate top-level key makes the YAML ambiguous and causes 'mb list' / 'mb show' to error out across the ENTIRE .beads directory (one poisoned file breaks every mb command). The release delegate hit this four times in a single night.

GUARD (this change, tooling-only — no engine/refactor-owned files touched):
1. scripts/check_beads_dup_keys.py — standalone pure-python checker. Scans .beads/issues/*.md (or explicit files/dirs), flags any duplicate TOP-LEVEL frontmatter key (only column-0 'key:' lines inside the leading '---' block; list items '- x' and nested keys are correctly ignored). Exit 1 + offending file+key on any dup. '--repair' collapses each dup to its LAST occurrence (for updated_at = newest timestamp), preserving all other keys + body.
2. scripts/validate.py — new fast 'lint.beads-dupkey' step (no build deps, 0s) running the checker over .beads/issues. CI already runs '--group lint' (.github/workflows/ci.yml line 135), so CI catches it with NO workflow change. The error message names the file+key and points at 'check_beads_dup_keys.py --repair'.
3. scripts/git-hooks/pre-commit — lightweight local guard: when any .beads/issues/*.md is STAGED, run the checker on just those files BEFORE the existing Rust fmt check (restructured so a beads-only commit is still guarded; the old hook early-exited when no .rs was staged). Bypass with --no-verify.
4. Makefile 'beads-check' target (REPAIR=1 to auto-fix) for manual use.
5. agentplay/test_beads_dup_keys.py — 10 pytest cases (run by validate's agentplay.pytest step + CI '--group agentplay'): detects the exact mtg-742 dup-updated_at shape, does NOT false-positive on a labels block-list, --repair keeps newest + is idempotent, main() exit codes, directory scan, and a guard that the committed .beads tree is clean.

LATENT-DUP SCAN: scanned all 905 .beads/issues/*.md on integration (476e4873) — ZERO existing duplicate-key files (the mtg-742 instances were already hand-repaired before this branch). So no in-tree repairs were needed; the guard is purely preventative going forward.

HOW TO FIX A FUTURE OCCURRENCE: 'make beads-check REPAIR=1' (or 'python3 scripts/check_beads_dup_keys.py --repair'), then 'git add .beads/issues'. Repair keeps the latest updated_at and drops the earlier line.

STATUS: DONE on branch claude/beads-dupkey-guard @06ae7792 (rebased onto integration 1604577c). FULL 'make validate' GREEN — 35 passed, 0 failed, 0 skipped — artifact validate_logs/validate_06ae77927a491d742671f4506349f4d13d583761.log. New step lint.beads-dupkey PASS; agentplay.pytest 108 passed/3 skipped (incl. 10 new test_beads_dup_keys.py cases). Latent-dup scan on the rebased tree (906 .beads/issues/*.md) = clean. Ready for delegate ff-merge.

STAMP: 2026-06-11 #depth(476e4873 base; landed @06ae7792). Tooling task, parallel-safe, disjoint from the engine refactor.
