---
title: Playwright Page.captureScreenshot protocol error under CPU contention — validate-network-e2e false-fail
status: open
priority: 3
issue_type: bug
created_at: 2026-06-03T21:56:32.190571514+00:00
updated_at: 2026-06-03T21:56:32.190571514+00:00
---

# Description

RECURRING FALSE-FAIL (validate-infra). The wasm/web-UI screenshot step in validate-network-e2e-step intermittently fails with:

  [blocking] harness: page.screenshot: Protocol error (Page.captureScreenshot): Unable to capture screenshot
    - taking page screenshot
    - waiting for fonts to load...
    - fonts loaded
  FAIL: 1 blocking/major finding(s)
  make[2]: *** [Makefile:352: validate-network-e2e-step] Error 1

This is NOT a product bug: the same runs log 'No DESYNC or MONOTONICITY VIOLATION errors detected' across all network-e2e games — the headless Chromium just chokes on Page.captureScreenshot when the host is CPU-saturated (observed during slot03's B1 Aladdin validate: run took 1169s at avg 32.4% util / 75% of time <50% util, i.e. 4 worktrees' validates contending). A clean re-run on the identical tip passed (screenshot flake did not recur).

COST: forces full ~20-min validate re-runs; has bitten Kismet and Aladdin already, and will keep costing the whole team re-runs under contention.

CANDIDATE FIXES (validate-infra; likely slot02 validate-overhaul mtg-717 / mtg-726 network-test hygiene):
- Retry page.screenshot N times with backoff before declaring a blocking finding.
- Treat a screenshot-capture protocol error as a NON-blocking harness warning (the screenshot is QA/diagnostic, not a correctness assertion) — distinct from an actual test failure.
- Serialize / nice the browser screenshot step, or skip screenshots when CONTENTION/CI load is detected.
- Increase the Playwright protocol timeout for captureScreenshot.

Do NOT bolt onto merge-critical work; capture + route.
