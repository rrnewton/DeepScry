//! Fast Deck Entry Mode - Interactive TUI for rapid deck building
//!
//! Provides a streamlined interface for entering paper decks with minimal keystrokes:
//! - Fuzzy search with real-time preview (prefix matches prioritized)
//! - Number keys (1-9) to add multiple copies
//! - Enter to add single copy
//! - Arrow keys to navigate results
//! - First ESC clears search, second ESC saves and exits
//!
//! ## Architecture
//!
//! The deck builder is designed to work both as a native TUI (via crossterm) and
//! in the browser via WASM (using RatZilla). The core state and rendering logic
//! is shared; only the event loop differs between platforms.
//!
//! - `state`: Core state management (shared between native and WASM)
//! - `ui`: Rendering logic using ratatui (shared between native and WASM)
//! - Native entry point: `run_deck_builder()` with crossterm event loop
//! - WASM entry point: `wasm/deck_builder.rs` with RatZilla event handling

// Shared modules (available for both native and WASM)
pub mod events;
pub mod state;
pub mod ui;

// Re-export shared types
pub use events::{
    handle_deck_builder_click, handle_deck_builder_key, handle_exit_dialog_key, DeckBuilderAction, DeckBuilderKey,
};
pub use state::{
    card_sort_key, match_score, mtg_color_to_term, truncate_name, CardCategory, CardEntryGroup, DeckBuilderState,
    FocusedPane, CARD_COLUMN_WIDTH, CARD_NAME_WIDTH,
};
pub use ui::draw_ui;

// Native-only implementation
#[cfg(feature = "native")]
mod native;

#[cfg(feature = "native")]
pub use native::{run_deck_builder, DeckBuilderConfig};
