# UI Architecture - Separation of Concerns

## Overview

The MTG Forge UI architecture is designed for maximum code sharing between different rendering backends while maintaining clean separation of concerns.

## Core Principles

1. **Backend Independence**: Game logic and rendering decisions don't care which backend is used
2. **Shared Layout**: All backends use the same TUI layout geometry as the standard
3. **Event-Driven**: State changes trigger redraws, no polling
4. **Click/Keyboard Agnostic**: Event handling doesn't depend on backend specifics

## Architecture Layers

```
┌─────────────────────────────────────────────────────────────┐
│                     Game Engine                              │
│  (GameState, GameLoop, Controllers)                         │
│  - Game logic, rules, state management                      │
│  - No rendering knowledge                                   │
└──────────────────────┬───────────────────────────────────────┘
                       │
                       │ GameStateView
                       ↓
┌─────────────────────────────────────────────────────────────┐
│             FancyTuiRenderer (Shared)                       │
│  - Decides WHAT to draw and WHERE                           │
│  - Maintains entity_positions for hit testing              │
│  - Layout engine: how to fit cards in panes                │
│  - Shared between ALL backends                             │
└──────────────────────┬───────────────────────────────────────┘
                       │
                       │ draw_ui(Frame, GameStateView)
                       ↓
┌─────────────────────────────────────────────────────────────┐
│          Rendering Backend (Pluggable)                      │
│                                                             │
│  Native TUI:    crossterm + ratatui                        │
│  WASM TUI:      RatZilla (DOM-based terminal)              │
│  Image Overlay: Post-render hook (optional enhancement)    │
│                                                             │
│  - HOW to actually draw (pixels, DOM elements, etc)        │
│  - Terminal I/O, canvas rendering, etc                     │
└─────────────────────────────────────────────────────────────┘
```

## Detailed Components

### 1. Game Engine Layer

**Responsibilities**:
- Game state management (`GameState`)
- Rules execution (`GameLoop`)
- Player controllers
- No UI knowledge whatsoever

**Does NOT**:
- Know about rendering
- Care about terminal size
- Handle keyboard/mouse directly

### 2. Renderer Layer (`FancyTuiRenderer`)

**Responsibilities**:
- **Layout Engine**: Decides card arrangement within panes
  - How many cards per row
  - Wrapping and sizing logic
  - Pane dimensions
- **Hit Testing**: Maintains `entity_positions` for mouse clicks
- **Backend-Agnostic**: Uses `ratatui` Frame abstraction
- **Shared Code**: Same logic for native and WASM

**Key Principle**:
> The renderer decides geometry (X, Y, width, height) based on content and available space. It doesn't care HOW the backend draws those rectangles.

**State Tracking**:
```rust
pub struct FancyTuiState {
    /// Positions of all rendered entities (cards, panes, etc)
    pub entity_positions: Vec<EntityPosition>,
    /// Other UI state...
}

pub struct EntityPosition {
    pub entity: Entity,
    pub area: Rect,  // The actual geometry!
}
```

### 3. Backend Layer

**Responsibilities**:
- **Drawing**: Actually render rectangles, text, etc
- **I/O**: Keyboard events, mouse events, terminal resize
- **Frame Timing**: When to redraw (event loop, RAF, etc)

**Backend Types**:

#### Native TUI (`crossterm` + `ratatui`)
- Terminal-based rendering
- Direct character cells
- `crossterm` for I/O

#### WASM TUI (`RatZilla`)
- DOM-based terminal emulation
- Uses HTML elements for terminal cells
- Web events (keyboard, mouse)

#### Image Overlay (Enhancement)
- **Post-render hook** that runs AFTER TUI draws
- Queries `entity_positions` from renderer
- Creates DOM `<img>` overlays
- Positioned via CSS absolute positioning
- **Optional**: Can be enabled/disabled

## Event Architecture

### Render Loop

**Current Implementation (WASM)**:

```rust
terminal.draw_web(move |f| {
    // 1. Auto-run logic (if enabled)
    if state.auto_run && !state.game_over {
        state.run_until_choice();  // Modifies game state
    }

    // 2. Render TUI
    renderer.draw_ui(f, &view, prompt, &choices);

    // 3. [NEW] Post-render hooks
    // TODO: Image overlay callback goes here!
});
```

**Desired Architecture**:

```rust
terminal.draw_web(move |f| {
    // 1. Auto-run logic
    if state.auto_run && !state.game_over {
        state.run_until_choice();
    }

    // 2. Render TUI
    renderer.draw_ui(f, &view, prompt, &choices);

    // 3. Post-render hooks (extensible)
    for hook in &post_render_hooks {
        hook(&renderer.state.entity_positions, &view);
    }
});
```

### Event Handling (Keyboard/Mouse)

**Shared Event Handler** (`fancy_tui_events.rs`):
```rust
pub fn handle_key_event(
    state: &mut FancyTuiState,
    key: KeyInput,  // Abstract key representation
    view: &GameStateView,
    num_choices: usize,
) -> EventResult
```

**Backend Translates**:
- Native: `crossterm::KeyCode` → `KeyInput`
- WASM: `ratzilla::KeyCode` → `KeyInput`

**Benefits**:
- Event logic doesn't care about backend
- Easy to add new backends
- Testable without actual I/O

## Image Overlay Integration

### Current Problem (WRONG)

```javascript
// ❌ BAD: Polling every 2 seconds
setInterval(() => {
    if (cardImagesEnabled) {
        CardImageOverlay.displayBattlefieldCards();
    }
}, 2000);
```

**Why This Is Wrong**:
- Wastes CPU checking when nothing changed
- May miss rapid state changes
- Not tied to actual render cycle
- Violates event-driven principle

### Correct Approach (Event-Driven)

**1. JavaScript Callback Registration**:
```javascript
// Register callback when images enabled
window.onRenderComplete = function() {
    if (cardImagesEnabled) {
        CardImageOverlay.displayBattlefieldCards();
    }
};
```

**2. Rust Calls Callback After Render**:
```rust
terminal.draw_web(move |f| {
    // ... game logic ...
    renderer.draw_ui(f, &view, prompt, &choices);

    // Notify JavaScript that render is complete
    let _ = js_sys::eval("window.onRenderComplete && window.onRenderComplete()");
});
```

**Benefits**:
- No polling
- Runs exactly when needed
- Synchronized with TUI rendering
- Can be disabled cleanly

## Geometry Standardization

**Key Decision**: We standardize on the TUI layout geometry.

Even if we add more graphical elements (card images, animations, etc), they **follow the TUI's geometry**.

**Why**:
- Single source of truth for layout
- Shared code between native/WASM
- Gradual enhancement (TUI works, images enhance it)
- Simpler architecture

**How Image Overlays Work**:
1. TUI renders and fills `entity_positions`
2. Image overlay queries `tui_get_card_positions()` (exports `entity_positions`)
3. Images positioned at **exact same coordinates** as TUI cards
4. Images overlay on top (CSS `z-index`)

## Future: Additional Backends

The architecture supports adding new backends:

### Possible Future Backends

**Canvas-Based GUI**:
- Native 2D drawing (no terminal emulation)
- Still uses `FancyTuiRenderer` for layout decisions
- Draws card images directly instead of text
- Same `entity_positions` for hit testing

**OpenGL/WebGPU GUI**:
- 3D card rendering, animations
- Renderer provides layout, GPU backend draws
- Same event handling

**Mobile Touch UI**:
- Touch events instead of mouse
- Different layout constraints (smaller screen)
- Still uses shared renderer core

## Summary

| Layer | Responsibility | Shared? |
|-------|---------------|---------|
| Game Engine | Rules, state | ✅ Yes |
| FancyTuiRenderer | Layout, geometry decisions | ✅ Yes |
| Event Handlers | Keyboard/mouse logic | ✅ Yes (abstracted) |
| Backend | Drawing, I/O | ❌ No (pluggable) |
| Post-Render Hooks | Optional enhancements | ✅ Yes (callback API) |

**Current Status (2025-12-07)**:
- ✅ Shared renderer works
- ✅ Native and WASM TUI backends working
- ✅ Image overlay EXISTS but uses polling (WRONG)
- ❌ TODO: Convert image overlay to callback-based (CORRECT)

## Action Items

1. **Remove** 2-second `setInterval` polling from tui_game.html
2. **Add** `window.onRenderComplete` callback mechanism
3. **Call** callback from WASM render loop after `draw_ui()`
4. **Document** post-render hook API for future extensions
