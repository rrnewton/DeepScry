---
title: Web GUI implementation with shared TUI/GUI architecture
status: open
priority: 1
issue_type: task
created_at: 2025-12-06T18:30:59.310895823+00:00
updated_at: 2025-12-06T18:30:59.310895823+00:00
---

# Description

## Web GUI Implementation

Track progress on building a real web-based GUI for MTG Forge-rs on top of WASM.

## Goals

1. Basic text rendering of cards (fallback, always available)
2. Card image support from Scryfall (like Java Forge)
3. Maximum code sharing between TUI and GUI modes

## Design Document

See `ai_docs/WEB_GUI_DESIGN_PLAN.md` for full technical design.

## Key Decisions

- **Rendering approach**: Continue using egui/eframe as application framework
- **Image source**: Scryfall API (`https://api.scryfall.com/cards/{set}/{collector}`)
- **Image caching**: IndexedDB in browser
- **Layout sharing**: Same pane structure between TUI and GUI

## Implementation Phases

### Phase 1: Abstract Shared Layout Logic
- [ ] Extract `GameLayout` trait from `FancyTuiRenderer`
- [ ] Move pane sizing, card grouping to shared module
- [ ] Keep `FancyTuiRenderer` working unchanged

### Phase 2: Create RenderPrimitives Trait
- [ ] Define `RenderPrimitives` for drawing rectangles, text, images
- [ ] Create `TuiRenderPrimitives` wrapping ratatui `Frame`
- [ ] Refactor `FancyTuiRenderer` to use primitives

### Phase 3: Implement GUI Renderer (No Images)
- [ ] Create `GuiRenderPrimitives` using egui directly
- [ ] Draw cards as colored shapes with text
- [ ] Same layout as TUI, different rendering

### Phase 4: Add Card Image Support
- [ ] Implement Scryfall URL construction
- [ ] Add async image fetching via web-sys
- [ ] Add IndexedDB caching layer
- [ ] Integrate images into `GuiRenderer`

### Phase 5: Polish and Optimize
- [ ] Loading indicators while images fetch
- [ ] Progressive image quality
- [ ] Memory management for image cache

## Related Files

- `mtg-engine/src/game/fancy_tui_renderer.rs` - Shared TUI rendering (~2045 lines)
- `mtg-engine/src/wasm/fancy_tui.rs` - WASM TUI implementation
- `ai_docs/WEB_GUI_DESIGN_PLAN.md` - Full design document

## Open Questions

1. Image resolution: "normal" (488x680) or "small" (146x204)?
2. Zoom support for cards?
3. Animation for card movements?
4. Mobile touch-friendly UI?
