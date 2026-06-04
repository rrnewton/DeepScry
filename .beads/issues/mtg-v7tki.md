---
title: 'Experiment: ALL-DEBUG validate wall-clock vs release baseline'
status: open
priority: 3
issue_type: task
created_at: 2026-06-04T11:38:11.437857362+00:00
updated_at: 2026-06-04T11:38:11.437857362+00:00
---

# Description

mtg-717 follow-on (user-approved). HYPOTHESIS: building mtg in DEBUG instead of release may lower TOTAL validate wall-clock — the release build is the long pole AND the source of the unit.nextest->build.mtg-release coupling; debug compiles much faster but runs slower, so the net is UNKNOWN and must be measured. METHODOLOGY (on a branch, when the machine is IDLE): build mtg debug; point the determinism/shell/agentplay/network-e2e steps at the debug binary; run full `make validate`; measure wall-clock + per-step durations vs the release baseline (cite validate_logs SHA-stamped logs). Capture under experiments/ per the harness convention (README hypothesis+methodology+result, metadata.json with commit SHA/host). Decision output: keep release, or switch validate to debug-binary, or hybrid. Run ONLY after build-once (mtg-717) merges + network-redo is green + machine idle.
