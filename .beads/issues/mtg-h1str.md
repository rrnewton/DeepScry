---
title: Unify WASM and native TUI rendering - zero code duplication
status: closed
priority: 2
issue_type: task
labels:
- refactor
created_at: 2025-12-01T20:01:17.703287058+00:00
updated_at: 2025-12-01T20:01:17.703287058+00:00
---

# Description

## Goal

Achieve EXACT visual parity between WASM browser TUI and native CLI TUI with:
- **Zero code duplication** between backends
- **Minimal per-backend handling** (only event loop and terminal setup)
- **Same ASCII characters displayed** in both environments

## Completed

All tasks have been completed. The refactoring achieved:

1. **All rendering code now lives in `FancyTuiRenderer`** (~2000 lines)
   - `draw_ui()` - main entry point
   - `draw_player_info()` - status bar with life/zones/turn/phase
   - `draw_battlefield()` - battlefield with card groups
   - `render_entity()` - ASCII art card boxes with colored borders
   - `render_visual_stack()` - diagonal stacking visualization
   - `render_card_group()` - organizing cards by type with labels
   - `draw_card_details()`, `draw_hand()`, `draw_stack()` - other panels
   - All helper methods and constants (card dimensions, aspect ratios, etc.)

2. **`FancyTuiController` now only handles native-specific concerns** (~1100 lines, down from ~2900)
   - Terminal setup/teardown (crossterm)
   - Event loop (keyboard/mouse input)
   - Delegation to `FancyTuiRenderer` via `self.renderer.draw_ui()`

3. **`WasmFancyTuiApp` already uses the same rendering path**
   - Uses `FancyTuiRenderer` for all UI drawing
   - eframe/egui setup and browser event handling only

## Tasks

- [x] Move `draw_player_info()` to `FancyTuiRenderer`
- [x] Move `render_entity()` to `FancyTuiRenderer`
- [x] Move `render_visual_stack()` to `FancyTuiRenderer`
- [x] Move `render_card_group()` to `FancyTuiRenderer`
- [x] Move helper methods (`get_entity_dimensions`, constants like `CARD_SPACING`, etc.)
- [x] Update `draw_battlefield()` in renderer to use card boxes instead of lists
- [x] Refactor `FancyTuiController` to delegate all rendering to `FancyTuiRenderer`
- [x] Update WASM `WasmFancyTuiApp` to use same rendering path (already done)
- [x] Run `make validate` to ensure no regressions

## Success Criteria - ACHIEVED

1. ✅ Running native `mtg tui` shows identical ASCII output to WASM browser version
2. ✅ `FancyTuiController` contains ONLY:
   - Terminal setup/teardown (crossterm)
   - Event loop (keyboard/mouse input)
   - Delegation to `FancyTuiRenderer`
3. ✅ `WasmFancyTuiApp` contains ONLY:
   - eframe/egui setup
   - Browser event handling
   - Delegation to `FancyTuiRenderer`
4. ✅ All visual rendering code lives in `FancyTuiRenderer`

## Files Involved

- `mtg-engine/src/game/fancy_tui_renderer.rs` - Shared renderer (2028 lines)
- `mtg-engine/src/game/fancy_tui_controller.rs` - Native controller (1151 lines, simplified)
- `mtg-engine/src/wasm/fancy_tui.rs` - WASM app (473 lines, uses renderer)
