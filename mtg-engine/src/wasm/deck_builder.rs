//! WASM Deck Builder - RatZilla-based TUI rendering for browser
//!
//! This module provides the deck builder TUI experience in the browser using RatZilla.
//! Event handling uses shared handlers from `deck_builder/events.rs`.

use crate::deck_builder::{
    draw_ui, handle_deck_builder_click, handle_deck_builder_key, handle_exit_dialog_key, DeckBuilderAction,
    DeckBuilderKey, DeckBuilderState,
};
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

// Thread-local storage for the global deck builder state (for callbacks)
thread_local! {
    static GLOBAL_DECK_BUILDER_STATE: RefCell<Option<Rc<RefCell<WasmDeckBuilderState>>>> = const { RefCell::new(None) };
    static DECK_BUILDER_CELL_DIMS: RefCell<(f32, f32)> = const { RefCell::new((10.0, 20.0)) };
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
        let mut all_cards: Vec<String> = card_db.cards.keys().cloned().collect();
        all_cards.sort();
        let card_definitions: HashMap<String, Arc<CardDefinition>> = card_db.cards.clone();
        let state = DeckBuilderState::new(all_cards, card_definitions);

        Self {
            state,
            on_save_callback: None,
            on_exit_callback: None,
        }
    }

    /// Load an existing deck into the builder
    pub fn load_deck(&mut self, deck_json: &str) {
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
        serde_json::json!({ "main_deck": main_deck, "sideboard": [] }).to_string()
    }

    fn handle_save(&self) {
        if let Some(ref callback) = self.on_save_callback {
            let deck_json = self.export_deck_json();
            let this = JsValue::null();
            let _ = callback.call1(&this, &JsValue::from_str(&deck_json));
        }
    }

    fn handle_exit(&self) {
        if let Some(ref callback) = self.on_exit_callback {
            let this = JsValue::null();
            let _ = callback.call0(&this);
        }
    }
}

#[wasm_bindgen]
pub fn cleanup_deck_builder_state() {
    GLOBAL_DECK_BUILDER_STATE.with(|s| {
        *s.borrow_mut() = None;
    });
}

#[wasm_bindgen]
pub fn deck_builder_set_save_callback(callback: js_sys::Function) {
    GLOBAL_DECK_BUILDER_STATE.with(|s| {
        if let Some(ref state) = *s.borrow() {
            state.borrow_mut().on_save_callback = Some(callback);
        }
    });
}

#[wasm_bindgen]
pub fn deck_builder_set_exit_callback(callback: js_sys::Function) {
    GLOBAL_DECK_BUILDER_STATE.with(|s| {
        if let Some(ref state) = *s.borrow() {
            state.borrow_mut().on_exit_callback = Some(callback);
        }
    });
}

#[wasm_bindgen]
pub fn deck_builder_load_deck(deck_json: &str) {
    GLOBAL_DECK_BUILDER_STATE.with(|s| {
        if let Some(ref state) = *s.borrow() {
            state.borrow_mut().load_deck(deck_json);
        }
    });
}

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

#[wasm_bindgen]
pub fn deck_builder_get_stats() -> String {
    GLOBAL_DECK_BUILDER_STATE.with(|s| {
        if let Some(ref state) = *s.borrow() {
            let s = state.borrow();
            serde_json::json!({ "total_cards": s.state.total_cards(), "unique_cards": s.state.unique_cards() })
                .to_string()
        } else {
            r#"{"total_cards": 0, "unique_cards": 0}"#.to_string()
        }
    })
}

#[wasm_bindgen]
pub fn deck_builder_set_cell_dimensions(width_px: f32, height_px: f32) {
    DECK_BUILDER_CELL_DIMS.with(|dims| {
        *dims.borrow_mut() = (width_px, height_px);
    });
}

fn pixels_to_cells(x: u32, y: u32) -> (u16, u16) {
    DECK_BUILDER_CELL_DIMS.with(|dims| {
        let (w, h) = *dims.borrow();
        ((x as f32 / w) as u16, (y as f32 / h) as u16)
    })
}

/// Convert RatZilla `KeyCode` to shared `DeckBuilderKey`.
#[allow(clippy::wildcard_enum_match_arm)]
fn ratzilla_to_deck_builder_key(code: KeyCode) -> Option<DeckBuilderKey> {
    match code {
        KeyCode::Up => Some(DeckBuilderKey::Up),
        KeyCode::Down => Some(DeckBuilderKey::Down),
        KeyCode::Left => Some(DeckBuilderKey::Left),
        KeyCode::Right => Some(DeckBuilderKey::Right),
        KeyCode::Tab => Some(DeckBuilderKey::Tab),
        KeyCode::Enter => Some(DeckBuilderKey::Enter),
        KeyCode::Esc => Some(DeckBuilderKey::Escape),
        KeyCode::PageUp => Some(DeckBuilderKey::PageUp),
        KeyCode::PageDown => Some(DeckBuilderKey::PageDown),
        KeyCode::Home => Some(DeckBuilderKey::Home),
        KeyCode::End => Some(DeckBuilderKey::End),
        KeyCode::Delete => Some(DeckBuilderKey::Delete),
        KeyCode::Backspace => Some(DeckBuilderKey::Backspace),
        KeyCode::Char(c) => Some(DeckBuilderKey::Char(c)),
        _ => None,
    }
}

/// Launch the deck builder TUI.
///
/// # Errors
///
/// Returns a JavaScript error if the RatZilla DOM backend or terminal cannot
/// be created.
#[wasm_bindgen]
pub fn launch_deck_builder(
    card_db: &WasmCardDatabase,
    initial_deck_json: Option<String>,
    deck_name: Option<String>,
) -> Result<(), JsValue> {
    log::info!("Launching WASM deck builder TUI");

    let mut wasm_state = WasmDeckBuilderState::new(card_db);
    if deck_name.is_some() {
        wasm_state.state.deck_name = deck_name;
    }
    if let Some(deck_json) = initial_deck_json {
        wasm_state.load_deck(&deck_json);
    }

    let state = Rc::new(RefCell::new(wasm_state));

    let backend = DomBackend::new_by_id("ratzilla-terminal")
        .map_err(|e| JsValue::from_str(&format!("Failed to create DomBackend: {}", e)))?;
    let terminal = Terminal::new(backend).map_err(|e| JsValue::from_str(&format!("Terminal error: {}", e)))?;

    // Keyboard via shared handlers
    terminal.on_key_event({
        let state = Rc::clone(&state);
        move |key_event| {
            let mut s = state.borrow_mut();
            let db_key = match ratzilla_to_deck_builder_key(key_event.code) {
                Some(k) => k,
                None => return,
            };

            if let Some(action) = handle_exit_dialog_key(&mut s.state, db_key) {
                match action {
                    DeckBuilderAction::SaveAndExit => {
                        s.handle_save();
                        return;
                    }
                    DeckBuilderAction::ExitWithoutSaving => {
                        s.handle_exit();
                        return;
                    }
                    DeckBuilderAction::Handled => {
                        s.state.needs_redraw = true;
                    }
                    DeckBuilderAction::NotHandled => {}
                }
                return;
            }

            let action = handle_deck_builder_key(&mut s.state, db_key);
            match action {
                DeckBuilderAction::SaveAndExit => s.handle_save(),
                DeckBuilderAction::ExitWithoutSaving => s.handle_exit(),
                DeckBuilderAction::Handled | DeckBuilderAction::NotHandled => {
                    s.state.needs_redraw = true;
                }
            }
        }
    });

    // Mouse via shared handlers
    terminal.on_mouse_event({
        let state = Rc::clone(&state);
        move |mouse_event| {
            if mouse_event.button != MouseButton::Left || mouse_event.event != MouseEventKind::Pressed {
                return;
            }
            let mut s = state.borrow_mut();
            let (col, row) = pixels_to_cells(mouse_event.x, mouse_event.y);
            if handle_deck_builder_click(&mut s.state, col, row) {
                s.state.needs_redraw = true;
            }
        }
    });

    GLOBAL_DECK_BUILDER_STATE.with(|s| {
        *s.borrow_mut() = Some(Rc::clone(&state));
    });

    terminal.draw_web({
        let state = state;
        move |f| {
            let mut state = state.borrow_mut();
            draw_ui(f, &mut state.state);
            if state.state.needs_redraw {
                state.state.needs_redraw = false;
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
