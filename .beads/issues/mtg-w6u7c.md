---
title: 'validate: npm/playwright OFFLINE-first + hard-fail, no silent skip (locked-down host unblock)'
status: open
priority: 2
issue_type: task
created_at: 2026-06-04T12:40:21.837628423+00:00
updated_at: 2026-06-04T12:40:21.837628423+00:00
---

# Description

mtg-717 follow-on #1 (user-priority — unblocks the user's locked-down Meta host where npm install is forbidden). DONE on branch validate-followons.

PRINCIPLE (CLAUDE.md never-skip / coverage-desync): NO automatic test skip ever — a silently-skipped e2e is indistinguishable from a passing one. Honest options ONLY: provision (tests RUN) or explicitly disable (visible + reported).

CHANGES:
- web/ensure_node_deps.js (NEW): OFFLINE-FIRST provisioning. (1) if playwright requireable + chromium binary present -> use vendored, no npm install (the offline path: pre-stage web/node_modules + chromium cache once -> full e2e runs, no network); (2) else npm install with output SURFACED (never swallowed); (3) still absent -> HARD-FAIL exit 1 with actionable 3-option message. Does NOT npx-playwright-install at runtime (mtg-716 hermeticity — chromium provisioned by make setup).
- validate_run.py: wasm.npm-install now runs ensure_node_deps.js (was bare 'npm install --silent 2>/dev/null'); network.playwright-check now verify-only (was 'npm install ... 2>/dev/null || true' which hid the real failure). Added --no-wasm-e2e (alias --no-browser): explicit opt-out disabling ALL browser-resource steps (wasm browser suite + native-vs-WASM equiv sweeps + networked browser e2e) + their npm provisioning. Disabled steps RECORDED + REPORTED in header AND summary ('DISABLED via <flag> ... NOT full coverage'); --no-network now reports too (was a silent drop).
- Makefile: make setup npm-not-found -> HARD-FAIL exit 1 with offline option (was silent 'skipping' notice).

VERIFIED local: offline path exit 0; hard-fail path (deps absent + NPM=false) exit 1 w/ message; --no-wasm-e2e disables+reports 16 steps; provisioning+verify steps pass through runner; py_compile + node --check clean. Full validation via branch CI.

Relationship to Java Forge: N/A (CI/validate infra).
