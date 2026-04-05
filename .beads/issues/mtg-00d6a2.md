---
title: 'UI Audit: TUI vs Web shared/duplicated code analysis'
status: closed
priority: 3
issue_type: task
labels:
- audit
- ui
created_at: 2026-04-05T03:03:35.912518544+00:00
updated_at: 2026-04-05T03:03:43.160233238+00:00
closed_at: 2026-04-05T03:03:43.160233167+00:00
---

# Description

UI AUDIT COMPLETE - Architecture is well-designed 3-layer system.

SHARED (38%, 8073 lines): FancyTuiRenderer, events, display, controller, logger
NATIVE-ONLY (13%, 2728 lines): fancy_tui_controller, interactive_controller
WASM/WEB-ONLY (49%, 10298 lines): wasm/*.rs, web/*.html, web/*.js

KEY WINS: FancyTuiRenderer is backend-agnostic (ratatui Frame API), KeyInput enum abstracts events, GameStateView is universal.

ONLY DUPLICATION: InteractiveController display methods (~200 lines) duplicate display.rs

RECOMMENDATIONS:
1. Extract InteractiveController display into shared display.rs
2. Split fancy_tui_renderer.rs (4449 lines) into layout/battlefield/panes modules
3. Split fancy.html (4023 lines) into separate JS modules
4. Log scroll in web: need JS event handler wiring
5. Card details in TUI: shared draw_card_details() exists, just needs click routing

Date: 2026-04-04
