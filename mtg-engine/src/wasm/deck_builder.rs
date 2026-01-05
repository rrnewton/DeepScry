//! WASM Deck Builder - RatZilla-based TUI rendering for browser
//!
//! This module provides the deck builder TUI experience in the browser using RatZilla.
//! It reuses the shared DeckBuilderState and draw_ui functions from the deck_builder module.
//!
//! ## Architecture
//!
//! - Uses RatZilla's DomBackend for fast DOM-based terminal rendering
//! - Shares state management with native deck builder (DeckBuilderState)
//! - Game state is managed via Rc<RefCell<>> for the render callback
//! - Keyboard/mouse events are handled via RatZilla callbacks

use crate::deck_builder::{draw_ui, DeckBuilderState, FocusedPane};
use crate::loader::CardDefinition;
use ratzilla::event::{KeyCode, MouseButton, MouseEventKind};
use ratzilla::ratatui::Terminal;
use ratzilla::{DomBackend, WebRenderer};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use wasm_bindgen::prelude::*;

use super::WasmCardDatabase;

/// RatZilla uses these magic numbers for pixel-to-cell conversion
const CELL_WIDTH_PX: u32 = 10;
const CELL_HEIGHT_PX: u32 = 20;

// Thread-local storage for the global deck builder state (for callbacks)
thread_local! {
    static GLOBAL_DECK_BUILDER_STATE: RefCell<Option<Rc<RefCell<WasmDeckBuilderState>>>> = const { RefCell::new(None) };
}

/// WASM wrapper for the deck builder state
pub struct WasmDeckBuilderState {
    /// The shared deck builder state
    pub state: DeckBuilderState,
    /// Callback to invoke when deck is saved (passes deck JSON to JavaScript)
    pub on_save_callback: Option<js_sys::Function>,
    /// Callback to invoke when deck builder is exited without saving
    pub on_exit_callback: Option<js_sys::Function>,
}

impl WasmDeckBuilderState {
    /// Create a new WASM deck builder state from card database
    pub fn new(card_db: &WasmCardDatabase) -> Self {
        // Get all card names sorted
        let mut all_cards: Vec<String> = card_db.cards.keys().cloned().collect();
        all_cards.sort();

        // Clone card definitions for lookup
        let card_definitions: HashMap<String, Arc<CardDefinition>> = card_db.cards.clone();

        // Create the shared state
        let state = DeckBuilderState::new(all_cards, card_definitions);

        Self {
            state,
            on_save_callback: None,
            on_exit_callback: None,
        }
    }

    /// Load an existing deck into the builder
    pub fn load_deck(&mut self, deck_json: &str) {
        // Parse deck JSON: { "main_deck": [["card_name", count], ...] }
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(deck_json) {
            if let Some(main_deck) = parsed.get("main_deck").and_then(|v| v.as_array()) {
                self.state.deck.clear();
                for entry in main_deck {
                    if let Some(arr) = entry.as_array() {
                        if arr.len() >= 2 {
                            if let (Some(name), Some(count)) = (arr[0].as_str(), arr[1].as_u64()) {
                                self.state.deck.insert(name.to_string(), count as u8);
                            }
                        }
                    }
                }
                self.state.needs_redraw = true;
                log::info!("Loaded deck with {} unique cards", self.state.deck.len());
            }
        }
    }

    /// Export the current deck as JSON
    pub fn export_deck_json(&self) -> String {
        let deck_list = self.state.to_deck_list();
        let main_deck: Vec<(String, u8)> = deck_list
            .main_deck
            .iter()
            .map(|e| (e.card_name.clone(), e.count))
            .collect();

        serde_json::json!({
            "main_deck": main_deck,
            "sideboard": []
        })
        .to_string()
    }

    /// Handle save action - invoke callback with deck JSON
    fn handle_save(&self) {
        if let Some(ref callback) = self.on_save_callback {
            let deck_json = self.export_deck_json();
            let this = JsValue::null();
            let _ = callback.call1(&this, &JsValue::from_str(&deck_json));
        }
    }

    /// Handle exit without save - invoke callback
    fn handle_exit(&self) {
        if let Some(ref callback) = self.on_exit_callback {
            let this = JsValue::null();
            let _ = callback.call0(&this);
        }
    }
}

/// Clean up global deck builder state when exiting
#[wasm_bindgen]
pub fn cleanup_deck_builder_state() {
    GLOBAL_DECK_BUILDER_STATE.with(|s| {
        *s.borrow_mut() = None;
    });
    log::debug!(target: "wasm_deck_builder", "Cleaned up global deck builder state");
}

/// Set the save callback for the deck builder
#[wasm_bindgen]
pub fn deck_builder_set_save_callback(callback: js_sys::Function) {
    GLOBAL_DECK_BUILDER_STATE.with(|s| {
        if let Some(ref state) = *s.borrow() {
            state.borrow_mut().on_save_callback = Some(callback);
        }
    });
}

/// Set the exit callback for the deck builder
#[wasm_bindgen]
pub fn deck_builder_set_exit_callback(callback: js_sys::Function) {
    GLOBAL_DECK_BUILDER_STATE.with(|s| {
        if let Some(ref state) = *s.borrow() {
            state.borrow_mut().on_exit_callback = Some(callback);
        }
    });
}

/// Load a deck into the deck builder from JSON
#[wasm_bindgen]
pub fn deck_builder_load_deck(deck_json: &str) {
    GLOBAL_DECK_BUILDER_STATE.with(|s| {
        if let Some(ref state) = *s.borrow() {
            state.borrow_mut().load_deck(deck_json);
        }
    });
}

/// Export the current deck as JSON
#[wasm_bindgen]
pub fn deck_builder_export_deck() -> String {
    GLOBAL_DECK_BUILDER_STATE.with(|s| {
        if let Some(ref state) = *s.borrow() {
            state.borrow().export_deck_json()
        } else {
            "{}".to_string()
        }
    })
}

/// Get deck stats as JSON { total_cards, unique_cards }
#[wasm_bindgen]
pub fn deck_builder_get_stats() -> String {
    GLOBAL_DECK_BUILDER_STATE.with(|s| {
        if let Some(ref state) = *s.borrow() {
            let s = state.borrow();
            serde_json::json!({
                "total_cards": s.state.total_cards(),
                "unique_cards": s.state.unique_cards()
            })
            .to_string()
        } else {
            r#"{"total_cards": 0, "unique_cards": 0}"#.to_string()
        }
    })
}

/// Launch the deck builder TUI
///
/// # Arguments
/// * `card_db` - The loaded card database with all cards
/// * `initial_deck_json` - Optional JSON string with initial deck to load
/// * `deck_name` - Optional name/title of the deck being edited
///
/// Note: Wildcards are intentional - ratzilla KeyCode has 25+ variants;
/// we handle the subset used in the deck builder.
#[wasm_bindgen]
#[allow(clippy::wildcard_enum_match_arm)]
pub fn launch_deck_builder(
    card_db: &WasmCardDatabase,
    initial_deck_json: Option<String>,
    deck_name: Option<String>,
) -> Result<(), JsValue> {
    log::info!("Launching WASM deck builder TUI");

    // Create state
    let mut wasm_state = WasmDeckBuilderState::new(card_db);

    // Set deck name if provided
    if deck_name.is_some() {
        wasm_state.state.deck_name = deck_name;
    }

    // Load initial deck if provided
    if let Some(deck_json) = initial_deck_json {
        wasm_state.load_deck(&deck_json);
    }

    let state = Rc::new(RefCell::new(wasm_state));

    // Create RatZilla terminal
    let backend = DomBackend::new_by_id("ratzilla-terminal")
        .map_err(|e| JsValue::from_str(&format!("Failed to create DomBackend: {}", e)))?;
    let terminal = Terminal::new(backend).map_err(|e| JsValue::from_str(&format!("Terminal error: {}", e)))?;

    // Set up keyboard event handling
    terminal.on_key_event({
        let state = state.clone();
        move |key_event| {
            let mut state = state.borrow_mut();
            let deck_state = &mut state.state;

            // Clear status message on any key
            deck_state.status_message = None;

            // Handle exit dialog first
            if deck_state.show_exit_dialog {
                match key_event.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') => {
                        // Save and exit
                        state.handle_save();
                        return;
                    }
                    KeyCode::Char('q') | KeyCode::Char('Q') => {
                        // Exit without saving
                        state.handle_exit();
                        return;
                    }
                    KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                        // Cancel, go back
                        deck_state.show_exit_dialog = false;
                        deck_state.needs_redraw = true;
                    }
                    _ => {}
                }
                return;
            }

            match key_event.code {
                KeyCode::Tab => {
                    deck_state.toggle_focus();
                }
                KeyCode::Esc => {
                    // First ESC clears search, second ESC shows exit dialog
                    if !deck_state.search_query.is_empty() {
                        deck_state.search_query.clear();
                        deck_state.update_search();
                        deck_state.needs_redraw = true;
                    } else {
                        deck_state.show_exit_dialog = true;
                        deck_state.needs_redraw = true;
                    }
                }
                KeyCode::Up => match deck_state.focused_pane {
                    FocusedPane::Search => {
                        deck_state.select_previous();
                    }
                    FocusedPane::DeckSummary => {
                        let num_cols = deck_state.deck_num_columns;
                        deck_state.deck_select_previous(num_cols);
                        deck_state.needs_redraw = true;
                    }
                    FocusedPane::Problems => {} // Not supported in WASM
                },
                KeyCode::Down => match deck_state.focused_pane {
                    FocusedPane::Search => {
                        deck_state.select_next();
                    }
                    FocusedPane::DeckSummary => {
                        let num_cols = deck_state.deck_num_columns;
                        deck_state.deck_select_next(num_cols);
                        deck_state.needs_redraw = true;
                    }
                    FocusedPane::Problems => {} // Not supported in WASM
                },
                KeyCode::Left => {
                    if deck_state.focused_pane == FocusedPane::DeckSummary {
                        let num_cols = deck_state.deck_num_columns;
                        deck_state.deck_select_left(num_cols);
                        deck_state.needs_redraw = true;
                    }
                }
                KeyCode::Right => {
                    if deck_state.focused_pane == FocusedPane::DeckSummary {
                        let num_cols = deck_state.deck_num_columns;
                        deck_state.deck_select_right(num_cols);
                        deck_state.needs_redraw = true;
                    }
                }
                KeyCode::PageUp => {
                    match deck_state.focused_pane {
                        FocusedPane::Search => {
                            let page_size = deck_state.max_results;
                            deck_state.selected_index = deck_state.selected_index.saturating_sub(page_size);
                            deck_state.scroll_offset = deck_state.scroll_offset.saturating_sub(page_size);
                        }
                        FocusedPane::DeckSummary => {
                            deck_state.deck_selected_index = deck_state.deck_selected_index.saturating_sub(10);
                        }
                        FocusedPane::Problems => {} // Not supported in WASM
                    }
                    deck_state.needs_redraw = true;
                }
                KeyCode::PageDown => {
                    match deck_state.focused_pane {
                        FocusedPane::Search => {
                            let page_size = deck_state.max_results;
                            let max_idx = deck_state.search_results.len().saturating_sub(1);
                            deck_state.selected_index = (deck_state.selected_index + page_size).min(max_idx);
                            if deck_state.selected_index >= deck_state.scroll_offset + page_size {
                                deck_state.scroll_offset = deck_state.selected_index.saturating_sub(page_size - 1);
                            }
                        }
                        FocusedPane::DeckSummary => {
                            let max_idx = deck_state.deck.len().saturating_sub(1);
                            deck_state.deck_selected_index = (deck_state.deck_selected_index + 10).min(max_idx);
                        }
                        FocusedPane::Problems => {} // Not supported in WASM
                    }
                    deck_state.needs_redraw = true;
                }
                KeyCode::Home => {
                    match deck_state.focused_pane {
                        FocusedPane::Search => {
                            deck_state.selected_index = 0;
                            deck_state.scroll_offset = 0;
                        }
                        FocusedPane::DeckSummary => {
                            deck_state.deck_selected_index = 0;
                        }
                        FocusedPane::Problems => {} // Not supported in WASM
                    }
                    deck_state.needs_redraw = true;
                }
                KeyCode::End => {
                    match deck_state.focused_pane {
                        FocusedPane::Search => {
                            let max_idx = deck_state.search_results.len().saturating_sub(1);
                            deck_state.selected_index = max_idx;
                            deck_state.scroll_offset = max_idx.saturating_sub(deck_state.max_results - 1);
                        }
                        FocusedPane::DeckSummary => {
                            deck_state.deck_selected_index = deck_state.deck.len().saturating_sub(1);
                        }
                        FocusedPane::Problems => {} // Not supported in WASM
                    }
                    deck_state.needs_redraw = true;
                }
                KeyCode::Enter => match deck_state.focused_pane {
                    FocusedPane::Search => deck_state.add_selected(1),
                    FocusedPane::DeckSummary => deck_state.increment_deck_selected(),
                    FocusedPane::Problems => {}
                },
                KeyCode::Delete => {
                    deck_state.remove_selected();
                }
                KeyCode::Backspace => match deck_state.focused_pane {
                    FocusedPane::Search => {
                        deck_state.search_query.pop();
                        deck_state.update_search();
                        deck_state.needs_redraw = true;
                    }
                    FocusedPane::DeckSummary => {
                        deck_state.remove_selected();
                    }
                    FocusedPane::Problems => {}
                },
                KeyCode::Char(c) => {
                    if c.is_ascii_digit() && c != '0' {
                        // Number keys 1-9 SET count (not add)
                        let count = c.to_digit(10).unwrap() as u8;
                        match deck_state.focused_pane {
                            FocusedPane::Search => deck_state.set_selected(count),
                            FocusedPane::DeckSummary => deck_state.set_deck_selected(count),
                            FocusedPane::Problems => {}
                        }
                    } else if deck_state.focused_pane == FocusedPane::Search {
                        // Type character into search
                        deck_state.search_query.push(c);
                        deck_state.update_search();
                        deck_state.needs_redraw = true;
                    }
                }
                _ => {}
            }
        }
    });

    // Set up mouse event handling
    terminal.on_mouse_event({
        let state = state.clone();
        move |mouse_event| {
            // Only handle left mouse button press
            if mouse_event.button != MouseButton::Left || mouse_event.event != MouseEventKind::Pressed {
                return;
            }

            let mut state = state.borrow_mut();
            let deck_state = &mut state.state;

            // Convert pixel coordinates to terminal cell coordinates
            let cell_x = (mouse_event.x / CELL_WIDTH_PX) as u16;
            let cell_y = (mouse_event.y / CELL_HEIGHT_PX) as u16;

            // Check if click is within deck summary area
            if let Some(area) = deck_state.deck_summary_area {
                if cell_x >= area.x && cell_x < area.x + area.width && cell_y >= area.y && cell_y < area.y + area.height
                {
                    deck_state.focused_pane = FocusedPane::DeckSummary;
                    deck_state.status_message = None;
                    deck_state.needs_redraw = true;
                }
            }

            // Check if click is within search input area (same effect as results)
            if let Some(area) = deck_state.search_input_area {
                if cell_x >= area.x && cell_x < area.x + area.width && cell_y >= area.y && cell_y < area.y + area.height
                {
                    deck_state.focused_pane = FocusedPane::Search;
                    deck_state.status_message = None;
                    deck_state.needs_redraw = true;
                }
            }

            // Check if click is within search results area
            if let Some(area) = deck_state.search_results_area {
                if cell_x >= area.x && cell_x < area.x + area.width && cell_y >= area.y && cell_y < area.y + area.height
                {
                    deck_state.focused_pane = FocusedPane::Search;
                    deck_state.status_message = None;
                    deck_state.needs_redraw = true;
                }
            }
        }
    });

    // Store state in global for JavaScript callbacks
    GLOBAL_DECK_BUILDER_STATE.with(|s| {
        *s.borrow_mut() = Some(state.clone());
    });

    // Set up the render callback
    terminal.draw_web({
        let state = state.clone();
        move |f| {
            let mut state = state.borrow_mut();

            // Draw the UI using the shared draw function
            draw_ui(f, &mut state.state);

            // Update JavaScript with deck stats when state changes
            if state.state.needs_redraw {
                state.state.needs_redraw = false;

                // Notify JavaScript of state change
                let stats = serde_json::json!({
                    "total_cards": state.state.total_cards(),
                    "unique_cards": state.state.unique_cards()
                });
                let js_code = format!("window.onDeckBuilderUpdate && window.onDeckBuilderUpdate({})", stats);
                let _ = js_sys::eval(&js_code);
            }
        }
    });

    log::info!("Deck builder TUI launched successfully");
    Ok(())
}
