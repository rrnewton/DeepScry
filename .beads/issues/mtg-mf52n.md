---
title: Unify Native/WASM TUI Event Handling and Add Mouse Support
status: open
priority: 3
issue_type: task
labels:
- enhancement
- wasm
- refactoring
created_at: 2025-12-06T15:30:42.084566029+00:00
updated_at: 2025-12-06T15:30:42.084566029+00:00
---

# Description

## Summary

Analysis of native vs WASM TUI reveals significant code duplication and feature gaps. The shared `FancyTuiRenderer` (2,044 lines) is well-designed, but event handling has ~840 lines of duplication (25% waste).

## Current State

### What's Shared (Good)
- `FancyTuiRenderer` module: Core UI types, rendering methods, layout logic, hit testing infrastructure
- `FancyTuiState`, `FocusedPane`, `Entity`, `EntityPosition` types
- Card rendering, grouping, optimal sizing calculations

### What's Duplicated (Problem)
1. **Keyboard handler (90 lines exact duplication)**: H/I/Y/O/S keys for pane focus exist in both native and WASM with nearly identical code
2. **Navigation logic (275 lines)**: Arrow key routing, 2D grid navigation - exists in native only, missing in WASM
3. **Mouse handling (45 lines)**: Hit test loop, pane boundary checking - completely absent in WASM

## Critical Feature Gap: Hand Card Clicking

**Observation**: In WASM browser TUI, clicking cards in the Hand pane shows them in Card Details. This feature does NOT exist in native TUI.

**Root cause**: 
- WASM TUI doesn't actually have mouse click handling implemented
- The "clicking works" observation may be incorrect, or there's hidden JS handling
- Native has mouse handling but hand cards are rendered as `List` widget without individual `EntityPosition` entries
- Only battlefield cards create hit-testable `EntityPosition` entries

## Proposed Solution

### Goal: Common Infrastructure For
1. Panes - all layout handling and geometry calculation (already shared)
2. Common X/Y coordinate plane definition
3. Click event handling with bounding boxes and geometry
4. Shared object callbacks for handling events

### Priority 1: Extract Shared Event Handler
- Create `fancy_tui_event_handler.rs` module
- Functions: `handle_keyboard_event()`, `handle_mouse_click()`, `navigate_within_pane()`
- ~200 lines deduplication

### Priority 2: Add Mouse Support to WASM
- Set up RatZilla mouse event listener
- Call WASM function with click coordinates
- Use shared handler to update state

### Priority 3: Hand Card Hit Testing
- Modify `draw_hand()` to create `EntityPosition` entries for individual cards
- Enable clicking on hand cards in both native and WASM

### Priority 4: Navigation Helpers
- Extract 2D grid logic (CARDS_PER_ROW, wrapping) to shared static methods
- ~50 lines additional deduplication

## Files Involved

- `mtg-engine/src/game/fancy_tui_renderer.rs` (2,044 lines - shared)
- `mtg-engine/src/game/fancy_tui_controller.rs` (1,189 lines - native)
- `mtg-engine/src/wasm/fancy_tui.rs` (349 lines - WASM)
- `web/fancy.html` (browser interface)

## Acceptance Criteria

- [ ] Keyboard event handling extracted to shared module
- [ ] Mouse click handling works identically in native and WASM
- [ ] Clicking hand cards shows details in both platforms
- [ ] Arrow key navigation works in WASM (currently missing)
- [ ] No duplicate event handling code between native/WASM
