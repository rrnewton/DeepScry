---
title: test_mode_equivalence byte-identical-mock-seed races under concurrent validates (cgroup scope insufficient; shared-resource isolation gap)
status: open
priority: 2
issue_type: task
labels:
- test-hardening
created_at: 2026-06-04T18:21:11.025808290+00:00
updated_at: 2026-06-04T18:21:11.025808290+00:00
---

# Description

SYMPTOM: agentplay/test_mode_equivalence.py::test_drivers_byte_identical_mock_seed
intermittently FAILS during a full `make validate`, even when that validate is
wrapped in a transient systemd `--user --scope` cgroup (the mtg-ibj22 mechanism).

OBSERVED FAILURE (2026-06-04, slot04 web-launcher-deckeditor-ui @ d7f5de1c):
The two drivers (stop-and-go vs persistent) diverged for `--mock --seed=42`:
  stop-and-go choices:  <Choice> AI-Heuristic1 chose 0 - play Forest
                        <Choice> AI-Heuristic2 chose 0 - play Forest
  persistent (prefix):  <Choice> Random1 chose 0 - play Mountain
                        <Choice> Random1 chose 'p' (pass priority)
i.e. the two runs played ENTIRELY DIFFERENT GAMES (different controllers
Heuristic-vs-Random, different decks Forest-vs-Mountain) — a CONFIG-LEVEL
divergence, NOT a same-game mid-action desync. Same seed, different game config =
the test's two driver invocations resolved different defaults/inputs, which points
at a SHARED-RESOURCE race, not an engine determinism bug.

WHY THIS IS NOT A REAL DESYNC / NOT THE TRIGGERING DIFF:
- The triggering branch is PURE FRONTEND (web/*.html, web/*.js only); it cannot
  touch the native/WASM engine determinism path.
- Run on the SAME content with a clean (dirty-tree) validate: FULL GREEN incl. this
  step. Integration is green on this test.
- Re-ran `./scripts/test_mode_equivalence.sh` STANDALONE twice back-to-back: both
  exit 0. The failure only reproduces under the concurrent full-validate fan-out.

ROOT-CAUSE HYPOTHESIS (shared resource, not process-tree orphaning):
mtg-ibj22's systemd-scope fix isolates the PROCESS TREE (atomic kill of all
descendants incl. setsid/double-forks) but does NOT namespace SHARED RESOURCES.
Two validates (across the 4 concurrent slots, or back-to-back) appear to contend on
one or more of:
  - a shared generated cache file (e.g. card-lookup.bin / catalog bin) — cf. the
    known "lingering generated card-lookup.bin contaminates hermetic runs" memory;
  - shared temp paths / fixed filenames the mock orchestrator writes;
  - fixed TCP ports (the port-collision class already noted in mtg-ibj22).
A second validate regenerating/overwriting that shared artifact mid-run would make
the two driver subprocesses read different inputs → different game config → this
exact config-level divergence.

FIX DIRECTION:
- Give test_mode_equivalence (and the agentplay orchestrator generally) per-run
  PRIVATE temp dirs + per-run unique ports + a private/immutable card-data cache
  (no shared writable artifact), so two validates cannot cross-contaminate.
- This is the SHARED-RESOURCE half of the netns/full-isolation work tracked in
  mtg-ibj22 (cgroup scope = process isolation; this = filesystem/port isolation).
  Consider folding into mtg-ibj22 or doing it as its companion.
- Recurrence of the project "isolate validates before concurrency" class: cgroup
  scope alone is necessary but NOT sufficient.

REPRO: run `make validate` concurrently across ≥2 slots (or in a tight loop) and
watch agentplay.mode-equiv; standalone `./scripts/test_mode_equivalence.sh` stays
green, isolating the cause to cross-validate resource sharing.

RELATED: mtg-ibj22 (cgroup-scope process isolation), and the card-lookup.bin
hermetic-contamination memory.
