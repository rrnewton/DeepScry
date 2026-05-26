---
title: Web GUI implementation with shared TUI/GUI architecture
status: open
priority: 1
issue_type: task
created_at: 2025-12-06T18:30:59.310895823+00:00
updated_at: 2025-12-07T19:16:53.792660783+00:00
---

# Description

## Web GUI Implementation

Track progress on building a real web-based GUI for MTG Forge-rs on top of WASM.

## Goals

1. Basic text rendering of cards (fallback, always available) ✅
2. Card image support from Scryfall (like Java Forge) ✅ **WORKING!**
3. Maximum code sharing between TUI and GUI modes ✅

## Design Document

See `ai_docs/WEB_GUI_DESIGN_PLAN.md` for full technical design.

## Current Status (2025-12-07_#170)

**Card image overlays are now fully functional!** Screenshots confirm cards display correctly
during gameplay with proper positioning within TUI battlefield sections.

### What Works:
- ✅ Card images load from Scryfall API using card names
- ✅ Images positioned as overlays within battlefield panes  
- ✅ Toggle button in floating controls to show/hide images
- ✅ Auto-refresh every 2 seconds during gameplay
- ✅ Images clear properly on toggle off or game exit
- ✅ Works with AI vs AI games (tested via Puppeteer automation)

### Current Implementation Details:
- Opponent cards: positioned at row 2, col 43 in 4-per-row grid
- Player cards: positioned at row 21, col 43 in 4-per-row grid  
- Card size: 14x18 cells (140px x 360px)
- Scryfall named card API: `https://api.scryfall.com/cards/named?exact={name}`
- Periodic refresh: 2-second interval when images enabled

## Implementation Phases

### Phase 1: Image Infrastructure ✅ COMPLETED (2025-12-07)
- [x] Create `ImageOverlayManager` Rust module (mtg-engine/src/wasm/image_overlay.rs)
  - Scryfall URL builder (Small/Normal/ArtCrop versions)
  - Cell-to-pixel conversion (10px x 20px cells)
  - DOM image element management
- [x] Add web-sys features: HtmlImageElement, CssStyleDeclaration, NodeList
- [x] Implement JavaScript CardImageOverlay manager (web/tui_game.html)
  - enable/disable, toggle, setCardImage, removeOverlay, clearAll
  - scryfallNamedUrl for name-based image fetching
  - displayBattlefieldCards with proper positioning
- [x] Add "Show Card Images" checkbox to launch UI
- [x] Add toggle button to floating game controls
- [x] Export tui_get_battlefield_cards() WASM function
- [x] Test with real gameplay - confirmed working!

### Phase 2: Positioning & Refresh ✅ COMPLETED (2025-12-07)
- [x] Position images within TUI battlefield panes
- [x] Split cards into opponent/player sections  
- [x] Implement auto-refresh during gameplay (2-second interval)
- [x] Handle image lifecycle (show/hide/update as cards move)
- [x] Browser testing with Puppeteer screenshots

### Phase 3: Future Enhancements (TODO)
- [ ] Add set/collector_number metadata to card data for exact printings
- [ ] Implement IndexedDB caching for better performance
- [ ] Get actual card owner info from WASM (currently splits by count)
- [ ] Extract real card positions from TUI renderer (currently hardcoded grid)
- [ ] Add loading indicators for pending images
- [ ] Handle image load errors gracefully
- [ ] Support for different image sizes (small for battlefield, large for detail)
- [ ] Hover/click to show larger image
- [ ] Progressive image quality

## Recent Commits

- 1fae137b feat(wasm): Add card image overlay infrastructure for GUI enhancement
- cc863a8a feat(wasm): Add JavaScript card image overlay system with test demo
- 5ffcf69b feat(wasm): Add runtime card image toggle and battlefield card API
- 61523f64 feat(wasm): Display actual battlefield card images with Scryfall integration
- 88b2d228 feat(wasm): Improve card positioning and add auto-refresh

## Related Files

- `mtg-engine/src/wasm/image_overlay.rs` - Rust image overlay utilities
- `mtg-engine/src/wasm/fancy_tui.rs` - WASM TUI with tui_get_battlefield_cards()
- `web/tui_game.html` - JavaScript CardImageOverlay manager (UPDATED)
- `mtg-engine/src/game/fancy_tui_renderer.rs` - Shared TUI rendering (~2045 lines)
- `ai_docs/WEB_GUI_DESIGN_PLAN.md` - Full design document

## Testing Evidence

Puppeteer automated testing confirms:
- 10 battlefield cards displayed during AI vs AI game
- Images load from Scryfall: Badlands, Mishra's Factory, Scrubland, Bayou, Sengir Vampire, etc.
- Positioning aligns with TUI battlefield panes (rows 2 & 21, col 43)
- Toggle button works correctly (shows/hides all images)
- Auto-refresh updates images every 2 seconds

## Open Questions

1. ~~Image resolution: "small" (146x204) for battlefield, "normal" for detail?~~
   - **RESOLVED**: Using "small" (140x360px) works well for battlefield
2. Hover/zoom support for cards? - Future enhancement
3. Animation for image loading? - Future enhancement
4. Mobile touch-friendly UI? - Future consideration
5. ~~How to extract set/collector_number from CardDefinition?~~
   - **DECISION**: Using name-based lookup is acceptable MVP, can add metadata later
