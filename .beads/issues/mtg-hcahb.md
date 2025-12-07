---
title: Web GUI implementation with shared TUI/GUI architecture
status: open
priority: 1
issue_type: task
created_at: 2025-12-06T18:30:59.310895823+00:00
updated_at: 2025-12-07T17:02:01.837925223+00:00
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

### Phase 1: Image Infrastructure (COMPLETED 2025-12-07_#166)
- [x] Create `ImageOverlayManager` Rust module (mtg-engine/src/wasm/image_overlay.rs)
  - Scryfall URL builder (Small/Normal/ArtCrop versions)
  - Cell-to-pixel conversion (10px x 20px cells)
  - DOM image element management
- [x] Add web-sys features: HtmlImageElement, CssStyleDeclaration, NodeList
- [x] Implement JavaScript CardImageOverlay manager (web/fancy.html)
  - enable/disable, setCardImage, removeOverlay, clearAll
  - Test demo showing Lightning Bolt at fixed position
- [x] Add "Show Card Images" checkbox to UI
- [ ] Add `CardImageCache` module with IndexedDB storage (TODO)
- [ ] Store set/collector_number in card metadata (TODO)

**Current state**: Basic overlay plumbing works. JavaScript can position images
over the TUI using absolute positioning with pointer-events:none. Test demo
successfully shows a Lightning Bolt image from Scryfall at a fixed position.

**Next**: Extract card metadata from game state and map TUI layout positions to
image coordinates for real integration.

### Phase 2: Image Overlay System (IN PROGRESS)
- [ ] Extract card set/collector_number from game state
- [ ] Hook into `FancyTuiRenderer::render_entity()` to get card positions
- [ ] Map TUI card boxes to overlay coordinates
- [ ] Handle image loading states (pending, loaded, error)
- [ ] Implement lifecycle: show/hide/update as cards move between zones

### Phase 3: UI Integration
- [ ] Update card detail pane to show full card image
- [ ] Add loading indicators for pending images
- [ ] Handle hover/click to show larger image

### Phase 4: Polish and Optimization
- [ ] Implement image preloading for deck cards
- [ ] Add memory management (LRU cache eviction)
- [ ] Progressive image quality (small -> normal)
- [ ] Cross-browser testing

## Recent Commits

- 1fae137b feat(wasm): Add card image overlay infrastructure for GUI enhancement
- cc863a8a feat(wasm): Add JavaScript card image overlay system with test demo

## Related Files

- `mtg-engine/src/wasm/image_overlay.rs` - Rust image overlay utilities (NEW)
- `web/fancy.html` - JavaScript CardImageOverlay manager (UPDATED)
- `mtg-engine/src/game/fancy_tui_renderer.rs` - Shared TUI rendering (~2045 lines)
- `mtg-engine/src/wasm/fancy_tui.rs` - RatZilla WASM TUI implementation (~680 lines)
- `ai_docs/WEB_GUI_DESIGN_PLAN.md` - Full design document

## Open Questions

1. Image resolution: "small" (146x204) for battlefield, "normal" for detail?
   - **DECIDED**: Start with "small" for overlays
2. Hover/zoom support for cards?
3. Animation for image loading?
4. Mobile touch-friendly UI?
5. How to extract set/collector_number from CardDefinition?
   - Need to check CardDefinition structure and see if we need to add metadata
