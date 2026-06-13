---
title: Worktree Playwright captureScreenshot fails under XDG cache isolation
status: open
priority: 2
issue_type: task
created_at: 2026-06-13T19:18:33.804088015+00:00
updated_at: 2026-06-13T19:18:33.804088015+00:00
---

# Description

Worktree Playwright captureScreenshot fails under XDG cache isolation — `network.landing` browser e2e cannot pass in agent worktrees (Page.captureScreenshot protocol error); the primary checkout's Playwright env works fine. Effect: agent worktrees cannot run a fully-green `make validate` (browser screenshot step fails); landings must run the authoritative full validate in the primary checkout. Fix: make worktree Playwright/XDG cache setup match the primary checkout so worktrees can run network.landing.
