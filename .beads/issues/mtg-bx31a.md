---
title: 'validate: eager-exit on first failure (kill in-flight) — default + --keep-going opt-out'
status: open
priority: 3
issue_type: task
created_at: 2026-06-04T14:54:15.771007628+00:00
updated_at: 2026-06-04T14:54:15.771007628+00:00
---

# Description

mtg-717 follow-on (user-reported: 'one thing fails but make validate continues'). DONE on validate-followons.

DIAGNOSIS: Runner.run()'s self.stop stopped LAUNCHING new steps on failure but did NOT kill the steps already running in parallel (each blocked in proc.wait), so a fast failure waited for the slow in-flight build to finish before exiting.

FIX: on the FIRST real failure, EAGER-EXIT (default) kills every in-flight step via its process group (the per-step killpg reaper from mtg-ibj22), marks them ABORTED (not FAIL), and the scheduler winds down immediately. --keep-going opt-out runs everything to completion (collect all failures in one pass) for triage. Killed steps are labelled '⊘ ABORT' + listed in stats ('ABORTED (eager-exit, not run to completion)') + counted separately from failures — never silently. Tests (agentplay/test_validate_flags.py): fast-fail + slow step -> slow KILLED + run returns <20s (not 60s) + marked aborted; --keep-going -> nothing aborted. CI shards are per-group so fail-fast-within-shard is fine; default eager everywhere unless --keep-going.
