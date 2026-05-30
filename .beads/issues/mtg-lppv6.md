---
title: 'Flaky: agentplay determinism tests collide on per-second /tmp/mtg_game_<sec>.log filename'
status: open
priority: 3
issue_type: bug
created_at: 2026-05-30T05:45:47.099612415+00:00
updated_at: 2026-05-30T05:45:47.099612415+00:00
---

# Description

DISCOVERED while landing the proptest invariant suite (branch
fuzz-proptest-invariants). NOT caused by that change — pre-existing.

## Symptom

Under a full parallel `make validate` (make -j4), these two tests intermittently FAIL:
- agentplay/test_mode_equivalence.py::test_engine_self_determinism_random_vs_random
- agentplay/test_mode_equivalence.py::test_drivers_byte_identical_mock_seed

with 'Two engine invocations with the same seed produced different game logs —
the engine is nondeterministic'. The diff shows mismatched *player names*
(Random1/Random2 vs Ryan/Gabriel) and extra/missing spells — i.e. content from
an UNRELATED game leaking into the compared log.

## Root cause (confirmed)

The engine writes its auto-saved game log to a path generated in
mtg-engine/src/main.rs::save_game_log_to_tmp as
`/tmp/mtg_game_{YYYYMMDD_HHMMSS}.log` — **per-SECOND granularity**. The test
helper agentplay/test_mode_equivalence.py::_run_engine_directly runs the same
binary twice and parses 'Log saved to <path>' from stderr. When two invocations
(or any concurrent validate game) land in the same wall-clock second, they
generate the SAME filename and clobber/interleave each other's file. The test
then reads a corrupted/foreign log and reports spurious nondeterminism.

## Proof it is NOT an engine bug

Running the binary twice >1s apart (distinct filenames) yields BYTE-IDENTICAL
logs (diff = 0). Running the two tests in ISOLATION (no parallel load) passes
3/3 consistently. The flake only appears under parallel validate load.

## Fix options

1. Make save_game_log_to_tmp's filename unique per process: append PID and/or a
   counter / nanosecond / random suffix (e.g. mtg_game_{ts}_{pid}_{nanos}.log).
   This is the real fix — a per-second timestamp is not a unique path.
2. (test side) _run_engine_directly could pass an explicit per-run output path
   instead of parsing the shared /tmp path, but the engine-side filename
   collision is the underlying defect and should be fixed at the source.

## Affected files
- mtg-engine/src/main.rs (save_game_log_to_tmp, ~line 59-72)
- agentplay/test_mode_equivalence.py (_run_engine_directly, ~line 478)
