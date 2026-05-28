---
title: 'CI: switch test-determinism/test-shell to a larger runner (4-8 vCPU) once enabled'
status: open
priority: 3
issue_type: task
created_at: 2026-05-28T19:01:37.736153416+00:00
updated_at: 2026-05-28T19:01:37.736153416+00:00
---

# Description

Follow-up to mtg-578 (CI Test job split into parallel jobs). The new test-determinism and test-shell jobs in .github/workflows/ci.yml run on STANDARD ubuntu-latest (2-vCPU). Their bottleneck is core oversubscription from per-test full-game subprocesses (determinism_e2e: 66 tests x 2 full games each; shell_script_tests: 28 full-game .sh scripts). A LARGER runner (4-8 vCPU) would roughly halve/quarter these two jobs' wall time and is the highest-leverage next step once the runner is available.

ACTION: change 'runs-on: ubuntu-latest' to the larger runner label for the test-determinism and test-shell jobs (e.g. an org runner-group label, or GitHub larger-runner labels like ubuntu-latest-4-cores). Requires the user's GitHub account settings/billing to enable larger runners (NOT assumable from a feature branch). A clear comment marking this is already in ci.yml at the top of the jobs: block.

Once enabled, also consider bumping the determinism job test-threads to match core count. test-shell should stay --test-threads=1 (subprocess-heavy scripts oversubscribe regardless of core count).
