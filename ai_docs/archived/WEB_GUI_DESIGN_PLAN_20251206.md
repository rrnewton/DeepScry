# Web GUI Design Plan

## Overview

This document outlines the design plan for adding a real web-based GUI to MTG Forge-rs on top of our WASM target. The key goals are:

1. **Basic text rendering of cards** (fallback, always available)
2. **Card image support** (fetched from Scryfall, like Java Forge)
3. **Maximum code sharing** between TUI and GUI modes

## Current Architecture Analysis (Updated 2025-12)

### Existing TUI Structure

The project has a well-abstracted TUI architecture that shares code between native and WASM:

- **`FancyTuiRenderer`** (`src/game/fancy_tui_renderer.rs`): ~2045 lines of shared rendering logic
  - All rendering methods take `&mut Frame` from ratatui (backend-agnostic)
  - No crossterm/terminal-specific code
  - State management kept simple for serialization across JS boundary

- **`FancyTuiController`** (native): Uses crossterm + blocking event loop
- **`WasmFancyTuiState`** (`src/wasm/fancy_tui.rs`): Uses **RatZilla** + browser event loop

### Current WASM TUI Implementation - RatZilla

The WASM TUI now uses **RatZilla** (v0.2) instead of the previous egui/eframe approach:

```toml
# Cargo.toml features
wasm-tui = [
    "wasm",
    "ratatui",  # Needed for FancyTuiRenderer shared code
    "ratzilla",
]
```

**RatZilla** provides:
- **`DomBackend`**: Fast DOM-based terminal rendering directly to HTML elements
- **WebGL2 rendering** option for enhanced performance
- **Direct event handling**: `terminal.on_key_event()` and `terminal.on_mouse_event()`
- **Render callback pattern**: `terminal.draw_web(|f| { ... })`

**Key constants for pixel-to-cell conversion:**
```rust
const CELL_WIDTH_PX: u32 = 10;
const CELL_HEIGHT_PX: u32 = 20;
```

### WASM TUI Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                     JavaScript / HTML                            │
│  ┌─────────────────────────────────────────────────────────────┐│
│  │  <div id="ratzilla-terminal">  (RatZilla renders here)     ││
│  └─────────────────────────────────────────────────────────────┘│
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                    RatZilla Backend                              │
│  - DomBackend::new_by_id("ratzilla-terminal")                   │
│  - Terminal<DomBackend>                                          │
│  - Keyboard/Mouse event callbacks                                │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                 WasmFancyTuiState (Rc<RefCell<>>)               │
│  - game: GameState                                               │
│  - renderer: FancyTuiRenderer (shared with native)              │
│  - p1/p2_controller_type: WasmControllerType                    │
│  - p1_human_controller: Option<WasmHumanController>             │
│  - current_prompt, current_choices                               │
│  - pending_context: Option<ChoiceContext>                        │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                FancyTuiRenderer (shared code)                    │
│  - draw_ui(frame, view, prompt, choices)                        │
│  - All pane rendering methods                                    │
│  - Layout calculations, entity grouping                          │
└─────────────────────────────────────────────────────────────────┘
```

### Human Input Pattern

The WASM implementation uses the **interrupt pattern** for human input:

1. `GameLoop::run_until_input()` returns `GameLoopState::AwaitingInput(ChoiceContext)`
2. UI displays choices in `current_choices` vector
3. User makes selection via keyboard/mouse
4. `WasmHumanController::set_pending_choice(PendingChoice)` stores the choice
5. Game continues with `run_until_choice()` which resumes the loop

### Global State Pattern

JavaScript button callbacks use thread-local storage:
```rust
thread_local! {
    static GLOBAL_TUI_STATE: RefCell<Option<Rc<RefCell<WasmFancyTuiState>>>> = ...;
}

#[wasm_bindgen]
pub fn tui_run_turn() { ... }
pub fn tui_select_choice() { ... }
pub fn tui_toggle_auto() { ... }
```

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

### Current State: RatZilla TUI is Excellent

The RatZilla-based TUI is already fast and functional. For the GUI enhancement:

**Option A: Extend RatZilla with Image Support**
- RatZilla renders to DOM elements
- Could inject `<img>` elements for card images alongside the terminal
- Hybrid approach: text TUI + floating image overlays

**Option B: Parallel Canvas-based GUI**
- Create a separate Canvas2D or WebGL-based renderer
- Share layout logic with TUI (same pane structure)
- Different rendering backend (pixel graphics vs terminal cells)

**Option C: WebGL2 Overlay on RatZilla**
- RatZilla supports WebGL2 rendering mode
- Could overlay card images as WebGL textures
- More complex but potentially smoothest

### Recommended Approach: Option A (DOM/Image Hybrid)

Given that RatZilla already works well, the simplest path is:

1. **Keep RatZilla TUI** as the primary interface
2. **Add image support via DOM injection**:
   - When rendering a card, inject an `<img>` element positioned over the card's terminal area
   - Images float above the terminal text
   - Fallback to text when images not loaded
3. **Use web-sys for fetch/image loading**
4. **Cache images in IndexedDB**

This preserves all existing functionality while adding visual enhancement.

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
3. **Background fetching**: Use browser Fetch API via `web-sys`
4. **IndexedDB caching**: Store fetched images for persistence across sessions
5. **Progressive enhancement**: Show text -> show image when ready

## Abstraction Architecture

### Current Shared Code (Keep As-Is)

The existing `FancyTuiRenderer` is already well-abstracted:

```rust
// Already backend-agnostic - takes Frame<impl Backend>
pub fn draw_ui(
    &mut self,
    f: &mut Frame,
    view: &GameStateView,
    current_prompt: Option<&str>,
    choices: &[(String, bool)],
)
```

### New Components for Image Support

```rust
/// Image cache for card artwork
pub struct CardImageCache {
    /// Loaded images keyed by card name + set
    images: HashMap<String, ImageData>,
    /// Pending fetch requests
    pending: HashSet<String>,
}

/// Image overlay manager for DOM injection
pub struct ImageOverlayManager {
    /// Currently displayed image elements
    overlays: HashMap<CardId, web_sys::HtmlImageElement>,
}

impl ImageOverlayManager {
    /// Create or update an image overlay for a card
    fn set_card_image(&mut self, card_id: CardId, area: Rect, image_url: &str);

    /// Remove image overlay (card left battlefield, etc.)
    fn remove_overlay(&mut self, card_id: CardId);

    /// Convert terminal cell coordinates to CSS pixels
    fn cell_to_pixels(x: u16, y: u16) -> (f32, f32) {
        (x as f32 * CELL_WIDTH_PX as f32, y as f32 * CELL_HEIGHT_PX as f32)
    }
}
```

### Scryfall URL Builder

```rust
/// Build Scryfall image URL for a card
pub fn scryfall_url(set_code: &str, collector_number: &str, version: ImageVersion) -> String {
    let version_str = match version {
        ImageVersion::Normal => "normal",
        ImageVersion::ArtCrop => "art_crop",
        ImageVersion::Small => "small",
    };
    format!(
        "https://api.scryfall.com/cards/{}/{}?format=image&version={}",
        set_code.to_lowercase(),
        collector_number,
        version_str
    )
}
```

## Implementation Phases

### Phase 1: Image Infrastructure (No Visual Changes)
- [ ] Add `CardImageCache` module with IndexedDB storage
- [ ] Implement Scryfall URL builder
- [ ] Add async image fetching via `web-sys` fetch
- [ ] Store set/collector_number in card metadata (if not already present)

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

## Technical Considerations

### WASM-Specific Constraints

1. **No std::thread**: Use browser async APIs via `web-sys`
2. **No filesystem**: Use IndexedDB via `web-sys` for caching
3. **Single-threaded**: All operations non-blocking
4. **CORS**: Scryfall API supports CORS, direct fetch works

### RatZilla-Specific Integration Points

```rust
// RatZilla provides these hooks:
terminal.on_key_event(|key_event| { ... });
terminal.on_mouse_event(|mouse_event| { ... });
terminal.draw_web(|frame| {
    // This is where we render AND where we'd update image overlays
    renderer.draw_ui(frame, ...);

    // After TUI render, update image positions to match card locations
    image_manager.sync_overlays(&renderer.state.entity_positions);
});
```

### Performance Targets

- **60 FPS** for smooth UI (RatZilla already achieves this)
- **< 16ms** per frame render time
- **< 100ms** initial load (excluding images)
- **Background image loading** should not block UI

### Image Size Estimates

- Normal card image: ~100-200KB each
- Art crop: ~50-100KB each
- Small: ~10-20KB each
- Typical deck (60 cards, ~40 unique): ~4-8MB total with normal images
- Should use "small" for in-game, "normal" for detail view

## Open Questions

1. **Image resolution**: Use "small" (146x204) for battlefield, "normal" for detail view?
2. **Zoom support**: Hover to show full-size card?
3. **Animation**: Fade-in when images load?
4. **Mobile support**: Touch-friendly image viewing?
5. **Offline mode**: IndexedDB should persist, but how to handle cache limits?

## References

### RatZilla
- [RatZilla on crates.io](https://crates.io/crates/ratzilla)
- DOM-based and WebGL2 ratatui backend for WASM
- Version 0.2 currently in use

### Scryfall API
- [Scryfall API Docs](https://scryfall.com/docs/api)
- Image endpoint: `GET /cards/:code/:number?format=image`
- Supports CORS

### Java Forge Image Sources
- Primary: `https://api.scryfall.com/cards/{set}/{collector_number}?format=image`
- Backup: `https://downloads.cardforge.org/images/cards/`
- Storage: `{cache}/pics/cards/{SET}/{cardname}.fullborder.jpg`

### Related Files in This Codebase
- `mtg-engine/src/game/fancy_tui_renderer.rs` - Shared TUI rendering (~2045 lines)
- `mtg-engine/src/wasm/fancy_tui.rs` - RatZilla WASM TUI implementation (~680 lines)
- `mtg-engine/src/wasm/human_controller.rs` - Human input pattern (~366 lines)
- `mtg-engine/src/wasm/mod.rs` - WASM module exports and WasmGame API
- `mtg-engine/Cargo.toml` - Feature flags: `wasm-tui` (RatZilla), `wasm-tui-egui` (legacy)

### Legacy Implementation (Deprecated)
The previous egui/eframe-based approach is still available under the `wasm-tui-egui` feature flag but is no longer the primary implementation. The RatZilla approach is faster and simpler.
