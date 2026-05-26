---
title: Unify Native/WASM TUI Event Handling and Add Mouse Support
status: closed
priority: 3
issue_type: task
labels:
- enhancement
- wasm
- refactoring
created_at: 2025-12-06T15:30:42.084566029+00:00
updated_at: 2025-12-06T16:07:20.591934059+00:00
---

# Description

## Summary

Analysis of native vs WASM TUI reveals significant code duplication and feature gaps. The shared `FancyTuiRenderer` (2,044 lines) is well-designed, but event handling has ~840 lines of duplication (25% waste).

## Status: COMPLETED

All priority items implemented in commits 297f304 and 9a26984:

### ✅ Priority 1: Extract Shared Event Handler (DONE)
- Created `fancy_tui_events.rs` module with:
  - `KeyInput` enum for cross-platform key abstraction
  - `EventResult` enum for action outcomes
  - `handle_key_event()` - shared keyboard processing
  - `handle_mouse_click()` - shared mouse click handling
  - 2D grid navigation helpers (CARDS_PER_ROW = 4)

### ✅ Priority 2: Add Mouse Support to WASM (DONE)
- Added `on_mouse_event` handler using RatZilla's API
- Converts pixel coordinates to terminal cells (width/10, height/20)
- Handles left mouse button press events
- Calls shared `handle_mouse_click()` function

### ✅ Priority 3: Hand Card Hit Testing (DONE)
- Added `Entity::HandCard` variant with card_id and index
- Modified `draw_hand()` to create `EntityPosition` entries for each card
- Clicking hand cards now selects them and shows details in both platforms

### Deferred: Backend Switching
- Filed new issue mtg-fho9v for RatZilla backend switching (dom/canvas/webgl2)
- DomBackend works well as default; other backends are optimization

## Test Results

- 386 unit tests passed (native-tui)
- Playwright e2e test: all steps passed (wasm-tui)

## Files Changed

- `mtg-engine/src/game/fancy_tui_events.rs` (NEW - ~530 lines)
- `mtg-engine/src/game/mod.rs` (module exports)
- `mtg-engine/src/game/fancy_tui_renderer.rs` (Entity::HandCard, draw_hand)
- `mtg-engine/src/wasm/fancy_tui.rs` (keyboard + mouse handlers)
- `web/tui_game.html` (14px font, collapsible controls)
