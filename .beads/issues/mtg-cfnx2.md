---
title: 'validate-network-e2e-step: Playwright ''Failed to install browsers'' hard-fails validate on cold browser cache'
status: open
priority: 2
issue_type: bug
created_at: 2026-06-03T06:41:40.674101644+00:00
updated_at: 2026-06-03T06:41:40.674101644+00:00
---

# Description

INFRA FLAKE (affects ALL branches, not code-specific). Observed repeatedly during the 2026-06-02 all-night run.

SYMPTOM: make validate's validate-network-e2e-step (Makefile:349, tests under tests/network_*/Playwright) prints:
  Installing dependencies...
  Switching to root user to install dependencies...
  Failed to install browsers
  Error: Installation process exited with code: 1
When the Playwright browser cache is COLD, this hard-fails the step → 'make: *** [Makefile:143: validate-impl] Error 2' → whole validate RED (cascades into 'Target page, context or browser has been closed' + 'P2 connection terminated unexpectedly' in the network GUI e2e). When browsers ARE cached, the same warning prints but is NON-FATAL and validate passes. So it's intermittent, keyed on cache state + the root-user install path failing in this sandbox.

IMPACT: spurious RED validates on otherwise-green branches; wastes full ~10-15min validate cycles on a re-run lottery. Hit fix-mtg-89-stress twice during the all-night run (the branch was content-complete; the engine/test legs all passed in nextest).

REPRO: run make validate with no Playwright browser cache present (or after a cache eviction) under concurrent load.

FIX IDEAS:
- Pin/prebuild the Playwright browser bundle once at environment setup (devcontainer/CI image) so validate never needs a runtime root install; OR
- Make the npm/Playwright browser install in the network-e2e harness robust: don't require 'switching to root', use a user-writable PLAYWRIGHT_BROWSERS_PATH, and HARD-FAIL with a clear 'browsers missing — run <cmd>' message rather than a cascading page-closed error; OR
- Gate validate-network-e2e-step on a browser-availability precheck that fails fast with an actionable message.

NOT a determinism/engine bug. Filed by backlog-logfix per team-lead request (2026-06-02).
