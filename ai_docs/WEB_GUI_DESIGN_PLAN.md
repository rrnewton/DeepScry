# Web GUI Design Plan

## Overview

This document outlines the design plan for adding a real web-based GUI to MTG Forge-rs on top of our WASM target. The key goals are:

1. **Basic text rendering of cards** (fallback, always available)
2. **Card image support** (fetched from Scryfall, like Java Forge)
3. **Maximum code sharing** between TUI and GUI modes

## Current Architecture Analysis

### Existing TUI Structure

The project already has a well-abstracted TUI architecture that shares code between native and WASM:

- **`FancyTuiRenderer`** (`src/game/fancy_tui_renderer.rs`): ~2045 lines of shared rendering logic
  - All rendering methods take `&mut Frame` from ratatui (backend-agnostic)
  - No crossterm/terminal-specific code
  - State management kept simple for serialization across JS boundary

- **`FancyTuiController`** (native): Uses crossterm + blocking event loop
- **`WasmFancyTui`** (`src/wasm/fancy_tui.rs`): Uses egui_ratatui + browser event loop

### Current WASM TUI Implementation

The WASM TUI uses:
- **eframe** (egui application framework, v0.33)
- **egui_ratatui** (RataguiBackend for terminal rendering within egui, v2.0)
- **soft_ratatui** (CosmicText font rendering)

This renders ratatui terminal output as pixels in an egui canvas.

### Layout Structure (Shared)

```
┌──────────────────────────────────────────────────────────────────┐
│ [Left 25%]           [Middle 50%]           [Right 25%]          │
│ ┌────────────────┐ ┌────────────────────┐ ┌────────────────────┐ │
│ │ Info (Combat/  │ │ Opponent Info Bar  │ │ Card Details       │ │
│ │ Log tabs)      │ ├────────────────────┤ ├────────────────────┤ │
│ ├────────────────┤ │ Opponent           │ │ Hand               │ │
│ │ Actions/Prompt │ │ Battlefield        │ ├────────────────────┤ │
│ │                │ ├────────────────────┤ │ Stack              │ │
│ │                │ │ Your Battlefield   │ │                    │ │
│ │                │ ├────────────────────┤ │                    │ │
│ │                │ │ Your Info Bar      │ │                    │ │
│ └────────────────┘ └────────────────────┘ └────────────────────┘ │
└──────────────────────────────────────────────────────────────────┘
```

## 2D Graphics Options for Browser/WASM

### Recommended Approach: **Canvas2D via web-sys**

After research, the recommended approach is to use the HTML Canvas 2D API directly via `web-sys`, similar to what the TUI already does through egui. This is the most pragmatic choice because:

1. **Performance**: Canvas2D is extremely fast for 2D rendering (< 1ms per frame)
2. **Simplicity**: No need for WebGL/WebGPU shader complexity for a card game
3. **Text support**: Native text rendering is excellent
4. **Image support**: Built-in image loading and drawing
5. **Already partially in use**: egui/eframe already uses Canvas2D under the hood

### Alternative Options Evaluated

| Library | Pros | Cons |
|---------|------|------|
| **wgpu** | Modern, cross-platform (WebGPU/WebGL2) | Overkill for 2D, shader complexity |
| **Quicksilver** | WASM support, 2D focused | Less maintained recently |
| **tiny-skia** | Skia port, nice API | Slowest option tested |
| **piet-web** | Ergonomic API | Another abstraction layer |

### Recommendation: Hybrid Approach

Use a **hybrid approach**:

1. **Continue using egui/eframe** as the application framework (already working)
2. **Add a new GUI render mode** that draws cards as images/shapes instead of terminal characters
3. **Share the same layout logic** (pane structure, sizing) between TUI and GUI
4. **Use egui's image handling** for card images (it already supports texture loading)

## Card Image Sources

### Primary Source: Scryfall API

Java Forge uses the Scryfall API for card images:

```
Base URL: https://api.scryfall.com/cards/
Format: {set_code}/{collector_number}?format=image&version={art_crop|normal}
```

Example URLs:
- Normal: `https://api.scryfall.com/cards/lea/231?format=image&version=normal`
- Art crop: `https://api.scryfall.com/cards/lea/231?format=image&version=art_crop`

### Local Storage (Java Forge Convention)

Images are cached locally at:
- **Cards**: `{cache_dir}/pics/cards/{SET_CODE}/{card_name}.jpg`
- **Tokens**: `{cache_dir}/pics/tokens/{token_name}.jpg`

File naming:
- `Card Name.fullborder.jpg` (generic)
- `Card Name1.fullborder.jpg` (alternate art #1)
- `{SET_CODE}/Card Name.fullborder.jpg` (set-specific)

### Image Loading Strategy for GUI

1. **Fallback first**: Always show text-based rendering while images load
2. **Lazy loading**: Only fetch images for cards in current decks
3. **Background fetching**: Use `wasm-bindgen-futures` for async image loading
4. **IndexedDB caching**: Store fetched images in browser IndexedDB for persistence
5. **Progressive enhancement**: Show text -> show low-res -> show full-res

## Abstraction Architecture

### Proposed Trait Hierarchy

```rust
/// Core layout trait - shared between all renderers
pub trait GameLayout {
    /// Get the layout areas for the main panes
    fn compute_layout(&self, total_area: Rect) -> LayoutAreas;

    /// Group battlefield cards into entities (stacking logic)
    fn group_entities(&self, cards: &[CardId], view: &GameStateView) -> Vec<Entity>;

    /// Calculate optimal card size for a given area
    fn calculate_card_size(&self, area: Rect, card_count: usize) -> (u16, u16);
}

/// Backend-agnostic rendering primitives
pub trait RenderPrimitives {
    type Color;
    type Area;

    fn draw_rect(&mut self, area: Self::Area, color: Self::Color, border: Option<Self::Color>);
    fn draw_text(&mut self, area: Self::Area, text: &str, style: TextStyle);
    fn draw_image(&mut self, area: Self::Area, image: &ImageHandle);
}

/// High-level game UI renderer
pub trait GameRenderer: RenderPrimitives {
    fn render_card(&mut self, area: Self::Area, card: &CardView, show_image: bool);
    fn render_battlefield(&mut self, area: Self::Area, entities: &[Entity], view: &GameStateView);
    fn render_hand(&mut self, area: Self::Area, cards: &[CardId], view: &GameStateView);
    fn render_stack(&mut self, area: Self::Area, stack: &[CardId], view: &GameStateView);
    fn render_prompt(&mut self, area: Self::Area, prompt: &str, choices: &[Choice]);
}
```

### Concrete Implementations

1. **`TuiRenderer`** (existing, refactored)
   - Implements `RenderPrimitives` for ratatui `Frame`
   - Text-based card rendering with ASCII boxes
   - Works in both native terminal and WASM egui_ratatui

2. **`GuiRenderer`** (new)
   - Implements `RenderPrimitives` for egui `Ui` or direct Canvas2D
   - Draws cards as colored rectangles with text overlay
   - Optional: draws actual card images when available

3. **`HybridRenderer`** (optional)
   - Combines TUI aesthetic with GUI capabilities
   - Text-based by default, images as enhancement

## Implementation Phases

### Phase 1: Abstract Shared Layout Logic
- Extract `GameLayout` trait from `FancyTuiRenderer`
- Move pane sizing, card grouping, entity logic to shared module
- Keep `FancyTuiRenderer` working unchanged

### Phase 2: Create RenderPrimitives Trait
- Define `RenderPrimitives` for drawing rectangles, text, images
- Create `TuiRenderPrimitives` wrapping ratatui `Frame`
- Refactor `FancyTuiRenderer` to use primitives

### Phase 3: Implement GUI Renderer (No Images)
- Create `GuiRenderPrimitives` using egui directly
- Draw cards as colored shapes with text
- Same layout as TUI, different rendering

### Phase 4: Add Card Image Support
- Implement Scryfall URL construction (port from Java)
- Add async image fetching via `web-sys` fetch API
- Add IndexedDB caching layer
- Integrate images into `GuiRenderer`

### Phase 5: Polish and Optimize
- Loading indicators while images fetch
- Progressive image quality (thumbnail -> full)
- Memory management for image cache
- Cross-browser testing

## Technical Considerations

### WASM-Specific Constraints

1. **No std::thread**: Use `wasm-bindgen-futures` for async operations
2. **No filesystem**: Use IndexedDB or localStorage for caching
3. **Single-threaded**: All rendering must be non-blocking
4. **CORS**: Scryfall API supports CORS, should work directly

### Performance Targets

- **60 FPS** for smooth animations
- **< 16ms** per frame render time
- **< 100ms** initial load (excluding images)
- **Background image loading** should not block UI

### Image Size Estimates

- Normal card image: ~100-200KB each
- Art crop: ~50-100KB each
- Typical deck (60 cards, ~40 unique): ~4-8MB total
- Should implement lazy loading, not preload everything

## Open Questions

1. **Image resolution**: Use "normal" (488x680) or "small" (146x204) for game view?
2. **Zoom support**: Should users be able to zoom in on cards?
3. **Animation**: Any card movement animations desired?
4. **Mobile support**: Touch-friendly UI considerations?
5. **Offline mode**: How much to cache for offline play?

## References

### Search Results - Rust WASM 2D Graphics
- [wgpu](https://wgpu.rs/) - Cross-platform WebGPU implementation
- [egui_ratatui](https://docs.rs/egui_ratatui) - Already in use
- [Quicksilver](https://github.com/ryanisaacg/quicksilver) - 2D game framework

### Java Forge Image Sources
- Primary: `https://api.scryfall.com/cards/{set}/{collector_number}?format=image`
- Backup: `https://downloads.cardforge.org/images/cards/`
- Storage: `{cache}/pics/cards/{SET}/{cardname}.fullborder.jpg`

### Related Files in This Codebase
- `mtg-engine/src/game/fancy_tui_renderer.rs` - Shared TUI rendering
- `mtg-engine/src/wasm/fancy_tui.rs` - WASM TUI implementation
- `mtg-engine/src/wasm/mod.rs` - WASM module exports
