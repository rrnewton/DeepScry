---
title: Game pages silent on missing WASM bundle (404 on pkg/mtg_forge_rs.js)
status: open
priority: 4
issue_type: bug
labels:
- web
- wasm
created_at: 2026-05-27T18:33:46.480430076+00:00
updated_at: 2026-05-27T18:33:46.480430076+00:00
---

# Description

MINOR finding from Playwright QA. When pkg/mtg_forge_rs.js returns 404 (bundle not built), native_game.html / tui_game.html / demo.html show 'Loading…' indefinitely with no user-facing error. Catch the dynamic import failure and render an explicit 'WASM bundle missing — run `make wasm-export`' message.
