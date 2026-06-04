---
title: 'Experiment: ALL-DEBUG validate wall-clock vs release baseline'
status: open
priority: 3
issue_type: task
created_at: 2026-06-04T11:38:11.437857362+00:00
updated_at: 2026-06-04T17:27:13.682912225+00:00
---

# Description

mtg-717 follow-on (user-approved). HYPOTHESIS: building mtg in DEBUG instead of release may lower TOTAL validate wall-clock — the release build is the long pole AND the source of the unit.nextest->build.mtg-release coupling; debug compiles much faster but runs slower, so the net is UNKNOWN and must be measured. METHODOLOGY (on a branch, when the machine is IDLE): build mtg debug; point the determinism/shell/agentplay/network-e2e steps at the debug binary; run full `make validate`; measure wall-clock + per-step durations vs the release baseline (cite validate_logs SHA-stamped logs). Capture under experiments/ per the harness convention (README hypothesis+methodology+result, metadata.json with commit SHA/host). Decision output: keep release, or switch validate to debug-binary, or hybrid. Run ONLY after build-once (mtg-717) merges + network-redo is green + machine idle.

== STATUS 2026-06-04 (slot02; DEFERRED — needs a quiet machine) ==
NOT STARTED (deliberately deferred, agreed w/ team-lead). This is a MEASUREMENT
(all-debug validate wall-clock vs the release baseline), so it needs:
(1) a QUIET machine — with 3 concurrent cross-slot validates the numbers are pure
    noise; run it only when the box is idle;
(2) a debug-binary knob — the test steps reuse target/release/mtg via
    MTG_REUSE_PREBUILT; to measure all-debug, build target/debug/mtg + add an env
    override (e.g. MTG_BINARY) so the determinism/shell/agentplay/e2e steps point at
    it. (Small code change, but it IS a change — not a pure measurement.)
Methodology + capture-under-experiments/ convention already written in the
description above. Pick up on an idle box.
