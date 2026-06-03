---
title: 'validate-network-e2e-step: Playwright ''Failed to install browsers'' hard-fails validate on cold browser cache'
status: in_progress
priority: 2
issue_type: bug
created_at: 2026-06-03T06:41:40.674101644+00:00
updated_at: 2026-06-03T06:58:04.245032947+00:00
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

--- FIX (2026-06-02, branch fix-mtg-cfnx2) ---
ROOT CAUSE pinpointed: the Makefile playwright-install line used
`npx playwright install chromium --with-deps`. The `--with-deps` flag makes
Playwright try to apt-install OS libraries as root ('Switching to root user to
install dependencies'), which HARD-FAILS in the non-root sandbox and aborts the
whole install (exit 1) — including the browser BINARY download. The trailing
`2>/dev/null || true` then swallowed the failure, so validate-network-e2e-step
proceeded with NO chromium present → 'Target page/context/browser closed' +
'P2 connection terminated unexpectedly' cascade. (When chromium happened to be
cached from a prior run, the same warning printed but was non-fatal → flaky.)

COMMAND-LEVEL RED→GREEN proof (this sandbox, non-root):
  npx playwright install chromium --with-deps  → exit 1 ('Failed to install browsers')
  npx playwright install chromium              → exit 0 (binary downloads; no root needed)

FIX (Makefile): drop `--with-deps` from all three playwright-install lines
(validate-network-e2e-step + the two manual wasm-e2e-network* targets). The
browser BINARY download needs no root; the OS libs ship with the dev/CI image
(proven by the cache-hit runs passing). For the validate path, also removed the
`|| true` on the browser install so a genuine download failure fails FAST with
Playwright's clear message instead of cascading into a confusing browser-closed
error. CI (ci.yml) does its own root `--with-deps` install directly in YAML
(unaffected by this Makefile change) and remains green as root.

FOLLOW-UP (optional, not done): pre-provision chromium in `make setup` (or the
devcontainer image) so `make validate` never downloads a browser at runtime
(fully hermetic). Tracked here for later.

Status: fix applied; make validate green run pending (cite validate log).

--- REVISED to the HERMETIC design (team-lead-approved target) ---
Rather than just dropping --with-deps and keeping a RUNTIME install in
validate (a network fetch mid-validate = anti-pattern), the fix now:
1. make setup: provisions chromium ONCE (cd web && npm install && npx
   playwright install chromium — binary only, no --with-deps/root). Best-effort
   with a clear skip notice if npm is absent.
2. validate-network-e2e-step: NO browser fetch. It verifies chromium is present
   via the Playwright API (node -e require('playwright').chromium.executablePath()
   + fs.existsSync — a STRUCTURED check, not a string grep) and FAILS FAST with
   an actionable 'Run: make setup' message if missing, instead of cascading into
   'Target page/context/browser has been closed'.
3. The two manual wasm-e2e-network* targets keep a self-healing idempotent
   install (no --with-deps); they are NOT part of validate so the hermeticity
   rule does not apply.
CI (ci.yml) still does its own root --with-deps install in YAML, unaffected.

This keeps make validate hermetic (CLAUDE.md: no runtime network fetch) and
self-validating (the changed validate-network-e2e-step is exercised by my own
make validate).
