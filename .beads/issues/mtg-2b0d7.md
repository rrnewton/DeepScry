---
title: Worktree Playwright captureScreenshot fails under XDG cache isolation
status: open
priority: 2
issue_type: task
created_at: 2026-06-13T19:18:33.804088015+00:00
updated_at: 2026-06-13T19:52:02.863240556+00:00
---

# Description

Worktree Playwright captureScreenshot fails under XDG cache isolation — `network.landing` browser e2e cannot pass in agent worktrees (Page.captureScreenshot protocol error).

ROOT CAUSE: `new_worktree.sh` / the validate harness redirects `XDG_CACHE_HOME` to a session-isolated path, but Playwright's browser binaries live in `~/.cache/ms-playwright`. When `XDG_CACHE_HOME` is redirected, Playwright cannot find its browser installation and fails with captureScreenshot protocol errors.

WORKAROUND (immediate): Export `PLAYWRIGHT_BROWSERS_PATH=/home/newton/.cache/ms-playwright` before running `make validate` in any worktree — this bypasses the XDG redirection and lets Playwright find the browsers: `PLAYWRIGHT_BROWSERS_PATH=/home/newton/.cache/ms-playwright make validate`

REAL FIX: Make `new_worktree.sh` and the worktree validate invocation export `PLAYWRIGHT_BROWSERS_PATH=/home/newton/.cache/ms-playwright` (or stop redirecting `XDG_CACHE_HOME` for Playwright's needs) so worktrees validate green by default without needing the env-var workaround.

Effect: agent worktrees cannot run a fully-green `make validate` without the workaround; the primary checkout's Playwright env works fine.
