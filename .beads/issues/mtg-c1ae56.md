---
title: Build WASM and run full E2E Playwright tests
status: open
priority: 1
issue_type: task
created_at: 2026-04-04T02:16:36.520686490+00:00
updated_at: 2026-04-04T02:16:36.520686490+00:00
---

# Description

Files: web/test_bug_report.js, web/test_fancy_tui.js, web/test_human_input.js, Makefile

Action: With Chromium and WASM target both installed:
1. Build the WASM package (check Makefile for the right target — likely make wasm-test-fancy or similar)
2. Run the existing E2E tests:
   - node web/test_fancy_tui.js
   - node web/test_bug_report.js  
   - node web/test_human_input.js (if it exists)
3. If tests fail, diagnose and fix — these tests were written but never run in a real browser
4. Capture test results and screenshots

Why: These tests were written in the prior session but never actually executed. We need to validate they work.

Verify:
- All E2E test scripts exit 0
- Test output shows actual browser interactions (not just syntax checks)
- Screenshots or test_results.json produced
