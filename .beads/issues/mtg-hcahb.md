---
title: Web GUI implementation with shared TUI/GUI architecture
status: open
priority: 1
issue_type: task
created_at: 2025-12-06T18:30:59.310895823+00:00
updated_at: 2025-12-06T19:30:27.580794805+00:00
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

## Current Architecture (RatZilla-based)

The WASM TUI uses **RatZilla** v0.2 for fast DOM-based terminal rendering:
- `DomBackend` renders directly to HTML elements
- `FancyTuiRenderer` (shared with native) does all drawing
- Human input uses interrupt pattern via `WasmHumanController`

## Key Decisions

- **Rendering approach**: Extend RatZilla TUI with DOM image overlays
- **Image source**: Scryfall API (`https://api.scryfall.com/cards/{set}/{collector}`)
- **Image caching**: IndexedDB in browser
- **Layout sharing**: Same pane structure between TUI and GUI (already shared)

## Implementation Phases

### Phase 1: Image Infrastructure (No Visual Changes)
- [ ] Add `CardImageCache` module with IndexedDB storage
- [ ] Implement Scryfall URL builder
- [ ] Add async image fetching via `web-sys` fetch
- [ ] Store set/collector_number in card metadata

### Phase 2: Image Overlay System
- [ ] Create `ImageOverlayManager` for DOM image elements
- [ ] Hook into `FancyTuiRenderer::render_entity()` to create overlays
- [ ] Position images correctly over terminal card boxes
- [ ] Handle image loading states (pending, loaded, error)

### Phase 3: UI Integration
- [ ] Add "Show Images" toggle to UI
- [ ] Update card detail pane to show full card image
- [ ] Add loading indicators for pending images
- [ ] Handle hover/click to show larger image

### Phase 4: Polish and Optimization
- [ ] Implement image preloading for deck cards
- [ ] Add memory management (LRU cache eviction)
- [ ] Progressive image quality (small -> normal)
- [ ] Cross-browser testing

## Related Files

- `mtg-engine/src/game/fancy_tui_renderer.rs` - Shared TUI rendering (~2045 lines)
- `mtg-engine/src/wasm/fancy_tui.rs` - RatZilla WASM TUI implementation (~680 lines)
- `mtg-engine/src/wasm/human_controller.rs` - Human input pattern
- `ai_docs/WEB_GUI_DESIGN_PLAN.md` - Full design document

## Open Questions

1. Image resolution: "small" (146x204) for battlefield, "normal" for detail?
2. Hover/zoom support for cards?
3. Animation for image loading?
4. Mobile touch-friendly UI?
