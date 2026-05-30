---
title: 'WASM GUI: ''?'' help popup too short — enlarge (less scrolling)'
status: open
priority: 3
issue_type: bug
created_at: 2026-05-30T19:33:38.447279252+00:00
updated_at: 2026-05-30T19:33:38.447279252+00:00
---

# Description

UI bug (user-reported 2026-05-30). In the WASM GUI, pressing '?' opens an "OK"-dismissable help pop-up that is very short in height and forces a lot of scrolling to read the keybindings/help text. Make the popup taller (and/or scroll-free for typical content).

Locations:
- web/native_game.html:2141 — `case '?':` handler that builds/shows the help modal (card-style native web GUI). The modal's height/max-height CSS is the likely culprit — bump max-height (e.g. to ~80vh) and width, ensure it's vertically centered and only scrolls if it genuinely overflows.
- mtg-engine/src/wasm/fancy_tui.rs:3213 — `KeyCode::Char('?') => KeyInput::Help` (ratatui-style TUI-in-browser). Check the Help overlay's rendered area/Rect sizing there too if it's similarly cramped.
- web/tui_game.html may share the overlay.

Fix: enlarge the help modal so the full keybinding list is visible without (or with minimal) scrolling; cap at a sane viewport fraction. Verify with a Playwright screenshot of the '?' help open in each affected UI (screenshots to gitignored debug/, cite paths — NEVER commit images).

Owner: web-ui-agentplay stream (same surface as the graveyard-relocation + web-agentplay work). Group with other UI polish.
