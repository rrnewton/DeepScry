---
title: 'Research: Server, UI, and WebSocket patterns for bug report feature'
status: open
priority: 0
issue_type: task
created_at: 2026-04-04T01:50:09.441061094+00:00
updated_at: 2026-04-04T01:50:09.441061094+00:00
---

# Description

Files: mtg-forge-rs/mtg-engine/src/server.rs, mtg-forge-rs/web/tui_game.html, mtg-forge-rs/mtg-engine/src/fancy_tui_controller.rs, mtg-forge-rs/mtg-engine/src/main.rs

Action: Deep-dive into existing codebase to understand:
1. How WebSocket messages are structured (message types, JSON schema, request/response pattern)
2. How the server handles incoming messages and dispatches them
3. How the floating controls widget in tui_game.html is structured (existing buttons, event handlers)
4. How CLI flags/args are parsed (for adding new trusted-password flag)
5. How game logs are currently captured/stored (both server-side and client-side)
6. What test patterns exist (look at tests/, test_fancy_tui.js, test_human_input.js, SUMMARY_JS_TESTING.md)
7. The directory layout of mtg-forge-rs/ (all crates, web/, tests/)

Why: All implementation tasks depend on understanding these patterns to integrate cleanly.

Verify: Notes on the task with clear answers to all 7 questions above.
