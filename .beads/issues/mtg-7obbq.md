---
title: 'Flaky engine determinism: test_engine_self_determinism_random_vs_random fails only under heavy concurrent validate (seed 42, one log drops interior <Choice> lines)'
status: open
priority: 2
issue_type: bug
created_at: 2026-06-04T06:20:00.377202682+00:00
updated_at: 2026-06-04T06:20:00.377202682+00:00
---

# Description

DISCOVERED 2026-06-04 during slot05's UNRELATED gh-filing validate (fix-bugreport-gh-filing @1dd7e06f; disjoint diff: network/server.rs + deploy scripts only — NOT the cause).

SYMPTOM: agentplay/test_mode_equivalence.py::test_engine_self_determinism_random_vs_random (+ ::test_drivers_byte_identical_mock_seed) FAILED inside a full `make -j4` validate. The test runs `mtg tui decks/simple_bolt.dck decks/simple_bolt.dck --p1=random --p2=random --seed=42 --verbosity=verbose` TWICE and asserts byte-identical auto-saved engine logs.

EVIDENCE (saved to gitignored scratch/determinism-flake-2026-06-04/ on the slot05 worktree host: validate_failure_excerpt.txt + CHARACTERIZATION.md): the two logs share the SAME turn-1 header and SAME final outcome ("Gabriel wins!"), but ONE log is MISSING interior lines the other has:
  + <Choice> Ryan chose 1 - play Volcanic Island
  + [GAMELOG Turn1 M1] Ryan plays Volcanic Island (59)
  + <Choice> Ryan chose 1 - cast Lightning Bolt
  + [GAMELOG Turn1 M1] Ryan casts Lightning Bolt (60) (putting on stack)
  ... (~1803 more diff lines truncated by pytest -q).

CAPTURE MECHANISM (_run_engine_directly, test_mode_equivalence.py:477): subprocess.run runs to COMPLETION (returncode==0 asserted) BEFORE reading the engine's auto-saved log file (path from "Log saved to <path>" on stderr). The engine PROCESS has fully exited before the read → an OS file-flush race on the log file is unlikely.

FREQUENCY / LOAD-DEPENDENCE: passes 3/3 in ISOLATION; 48 standalone engine runs (24 paired diffs) under MODERATE concurrent load did NOT reproduce; only the full `make -j4` validate (nextest + clippy + browser e2e saturating CPU) triggered it. => LOW-frequency, HEAVY-concurrency-only.

CHARACTERIZATION (INCONCLUSIVE without a fresh full divergent-log diff — could not reproduce in 48 runs): load-dependence argues AGAINST pure deterministic seed-derivation RNG divergence (load-insensitive). Two hypotheses:
  (a) REAL load/timing-sensitive engine nondeterminism (thread scheduling, HashMap iteration order, time/duration-based decision). If so → netarch determinism domain, and mtg-725 ('try_get(None) nondeterminism audit', marked DONE) MISSED a load-sensitive source.
  (b) A verbose-game-log WRITE/capture artifact under heavy load (log buffer flushed off the main path / in a Drop or background task racing process teardown), dropping interior lines from ONE capture.
The 'missing interior lines, identical header+winner' pattern leans (b) [partial capture] over (a) [a different game would usually change winner/state], but (a) is NOT excluded.

RECOMMENDED NEXT STEP (owner): repro-under-stress — run the two-invocation diff in a tight loop while `stress-ng -c $(nproc)` saturates CPU, then DIFF the two divergent logs in full. Clean 'missing contiguous block' → (b) log-capture (validate-infra/slot02 concurrent-load flake class). DIFFERENT choices/values at the same position → (a) REAL nondeterminism (critical; re-open mtg-725).

CROSS-REF: mtg-725 (try_get(None) audit). NON-BLOCKING for slot05's disjoint gh-filing fix (re-run was clean), but a self-determinism canary failing under load is sacred-ground per 'desync ALWAYS fatal' and must be owned, not buried.
