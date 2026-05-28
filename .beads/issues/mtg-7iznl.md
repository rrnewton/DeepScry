---
title: 'Native web GUI battlefield layout broken: 1 section per row, not shared with Web TUI'
status: open
priority: 2
issue_type: task
created_at: 2026-05-28T03:41:45.875074708+00:00
updated_at: 2026-05-28T03:42:02.562404538+00:00
---

# Description

CLUSTER: UI/infra.

In the native web GUI (native_game.html), the battlefield wastes space: cards split across 3 rows with only ONE section (artifacts | creatures | lands) per row and 1-2 cards per row. The Web TUI (tui_game.html) renders the same game state efficiently. They should share battlefield-layout logic but clearly don't.

Investigate: is there a shared layout module (Rust/WASM `battlefield_layout.rs`?) that the Web TUI uses but native_game.html reimplements in JS? Either make native_game.html consume the shared layout, or fix its CSS grid/flow so sections pack horizontally and cards wrap densely within a section. Cross-reference mtg-zheke (same-name stacking) — both are battlefield-render quality issues.
