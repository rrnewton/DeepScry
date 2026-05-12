---
title: 'Get WASM E2E tests running: Chromium + WASM toolchain'
status: closed
priority: 1
issue_type: task
created_at: 2026-04-04T02:16:36.515457382+00:00
updated_at: 2026-05-12T13:58:08.100104374+00:00
closed_at: 2026-05-12T13:58:08.100104304+00:00
---

# Description

Milestone: Get the existing Playwright E2E tests (web/test_bug_report.js, web/test_fancy_tui.js, web/test_human_input.js) running in a real browser.

Two blockers:
1. No Chromium binary installed (Playwright needs it)
2. rustup can't download nightly wasm32-unknown-unknown target (proxy issues)

Verify: npx playwright test or node web/test_bug_report.js runs and passes in a real Chromium browser.
