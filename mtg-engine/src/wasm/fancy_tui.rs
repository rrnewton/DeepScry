//! WASM Fancy TUI - RatZilla-based TUI rendering for browser
//!
//! This module provides the fancy TUI experience in the browser using RatZilla.
//! It uses the shared FancyTuiRenderer for consistent rendering between native and WASM.
//!
//! ## Architecture
//!
//! - Uses RatZilla's DomBackend for fast DOM-based terminal rendering
//! - Uses FancyTuiRenderer (shared with native) for all TUI drawing
//! - Game state is managed via Rc<RefCell<>> for the render callback

use crate::core::PlayerId;
use crate::game::controller::GameStateView;
use crate::game::fancy_tui_events::{handle_key_event, handle_mouse_click, EventResult, KeyInput};
use crate::game::logger::OutputMode;
use crate::game::{FancyTuiRenderer, GameLoop, GameState, VerbosityLevel};
use crate::game::{HeuristicController, PlayerController, RandomController, ZeroController};
use crate::loader::CardDefinition;
use ratzilla::event::{KeyCode, MouseButton, MouseEventKind};
use ratzilla::ratatui::Terminal;
use ratzilla::{DomBackend, WebRenderer};

/// RatZilla uses these magic numbers for pixel-to-cell conversion
const CELL_WIDTH_PX: u32 = 10;
const CELL_HEIGHT_PX: u32 = 20;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use wasm_bindgen::prelude::*;

use super::{WasmCardDatabase, WasmControllerType};

// Thread-local storage for the global TUI state (for button callbacks)
thread_local! {
    static GLOBAL_TUI_STATE: RefCell<Option<Rc<RefCell<WasmFancyTuiState>>>> = const { RefCell::new(None) };
}

/// Run one turn - called from JavaScript button
#[wasm_bindgen]
pub fn tui_run_turn() {
    GLOBAL_TUI_STATE.with(|state| {
        if let Some(ref state) = *state.borrow() {
            state.borrow_mut().run_one_turn();
        }
    });
}

/// Toggle auto-run mode - called from JavaScript button
#[wasm_bindgen]
pub fn tui_toggle_auto() {
    GLOBAL_TUI_STATE.with(|state| {
        if let Some(ref state) = *state.borrow() {
            let mut s = state.borrow_mut();
            s.auto_run = !s.auto_run;
        }
    });
}

/// WASM Fancy TUI Application State
///
/// This struct holds all the game state and is shared via Rc<RefCell<>>
/// for access from the render callback.
struct WasmFancyTuiState {
    /// The game state
    game: GameState,
    /// The TUI renderer (shared logic with native)
    renderer: FancyTuiRenderer,
    /// Player 1 controller type
    p1_controller_type: WasmControllerType,
    /// Player 2 controller type
    p2_controller_type: WasmControllerType,
    /// Current prompt text
    current_prompt: Option<String>,
    /// Current choices (text, is_highlighted)
    current_choices: Vec<(String, bool)>,
    /// Whether the game is over
    game_over: bool,
    /// Error message if any
    error_message: Option<String>,
    /// Auto-run mode (AI vs AI)
    auto_run: bool,
}

impl WasmFancyTuiState {
    /// Create a new WASM fancy TUI state from a GameState
    fn new(
        game: GameState,
        p1_controller_type: WasmControllerType,
        p2_controller_type: WasmControllerType,
    ) -> Self {
        // Create renderer for player 1's perspective
        let player_id = game.players[0].id;
        let renderer = FancyTuiRenderer::new(player_id, true);

        Self {
            game,
            renderer,
            p1_controller_type,
            p2_controller_type,
            current_prompt: Some("Game ready. Press Space to advance turn.".to_string()),
            current_choices: Vec::new(),
            game_over: false,
            error_message: None,
            auto_run: false,
        }
    }

    /// Run one turn of the game with AI controllers
    fn run_one_turn(&mut self) {
        if self.game_over {
            return;
        }

        let p1_id = self.game.players[0].id;
        let p2_id = self.game.players[1].id;

        // Create controllers based on type
        let mut p1_controller = self.create_controller(self.p1_controller_type, p1_id);
        let mut p2_controller = self.create_controller(self.p2_controller_type, p2_id);

        // Run one turn using the GameLoop
        let mut game_loop = GameLoop::new(&mut self.game).with_verbosity(VerbosityLevel::Normal);

        match game_loop.run_turns(p1_controller.as_mut(), p2_controller.as_mut(), 1) {
            Ok(result) => {
                if result.winner.is_some() {
                    self.game_over = true;
                    self.current_prompt = Some("Game Over!".to_string());
                }
            }
            Err(e) => {
                self.error_message = Some(format!("Game error: {}", e));
                self.game_over = true;
            }
        }
    }

    /// Create a controller based on type
    fn create_controller(
        &self,
        controller_type: WasmControllerType,
        player_id: PlayerId,
    ) -> Box<dyn PlayerController> {
        match controller_type {
            WasmControllerType::Zero => Box::new(ZeroController::new(player_id)),
            WasmControllerType::Random => Box::new(RandomController::with_seed(player_id, 42)),
            WasmControllerType::Heuristic => Box::new(HeuristicController::new(player_id)),
        }
    }
}

/// Launch the WASM fancy TUI in the browser
///
/// This function creates and runs the RatZilla-based TUI application.
#[wasm_bindgen]
pub fn launch_fancy_tui(
    card_db: &WasmCardDatabase,
    p1_deck_name: &str,
    p2_deck_name: &str,
    starting_life: i32,
    seed: u64,
    p1_controller: WasmControllerType,
    p2_controller: WasmControllerType,
    _canvas_width: u32,
    _canvas_height: u32,
) -> Result<(), JsValue> {
    // Create the game
    let game = create_game_from_database(card_db, p1_deck_name, p2_deck_name, starting_life, seed)?;

    // Create the shared state
    let state = Rc::new(RefCell::new(WasmFancyTuiState::new(
        game,
        p1_controller,
        p2_controller,
    )));

    // Create the RatZilla backend, targeting our specific container element
    let backend = DomBackend::new_by_id("ratzilla-terminal")
        .map_err(|e| JsValue::from_str(&format!("Failed to create backend: {}", e)))?;
    let terminal =
        Terminal::new(backend).map_err(|e| JsValue::from_str(&format!("Failed to create terminal: {}", e)))?;

    // Set up keyboard event handling using shared event handler
    terminal.on_key_event({
        let state = state.clone();
        move |key_event| {
            let mut state = state.borrow_mut();

            // Convert RatZilla KeyCode to our abstract KeyInput
            let key_input = match key_event.code {
                KeyCode::Char(' ') => Some(KeyInput::Space),
                KeyCode::Char('a') | KeyCode::Char('A') => {
                    // A: toggle auto-run (WASM-specific, not shared)
                    state.auto_run = !state.auto_run;
                    return;
                }
                KeyCode::Char('q') | KeyCode::Char('Q') => Some(KeyInput::Pass),
                KeyCode::Esc => Some(KeyInput::Escape),
                KeyCode::Char('h') | KeyCode::Char('H') => Some(KeyInput::FocusHand),
                KeyCode::Char('i') | KeyCode::Char('I') => Some(KeyInput::FocusInfo),
                KeyCode::Char('y') | KeyCode::Char('Y') => Some(KeyInput::FocusYourBf),
                KeyCode::Char('o') | KeyCode::Char('O') => Some(KeyInput::FocusOpponentBf),
                KeyCode::Char('s') | KeyCode::Char('S') => Some(KeyInput::FocusStack),
                KeyCode::Char('c') | KeyCode::Char('C') => {
                    // C: toggle controls panel visibility (WASM-specific)
                    let _ = js_sys::eval("document.getElementById('btn-toggle-controls')?.click()");
                    return;
                }
                KeyCode::Tab => Some(KeyInput::Tab),
                KeyCode::Up => Some(KeyInput::Up),
                KeyCode::Down => Some(KeyInput::Down),
                KeyCode::Left => Some(KeyInput::Left),
                KeyCode::Right => Some(KeyInput::Right),
                KeyCode::Enter => Some(KeyInput::Enter),
                KeyCode::Char(c) if c.is_ascii_digit() => Some(KeyInput::Digit(c.to_digit(10).unwrap() as u8)),
                _ => None,
            };

            if let Some(key) = key_input {
                // Get values we need before creating the view
                let num_choices = state.current_choices.len();

                // Use shared event handler
                // Split borrows: we need &game and &mut renderer.state
                let WasmFancyTuiState {
                    ref game,
                    ref mut renderer,
                    ..
                } = *state;

                let view = GameStateView::new(game, renderer.player_id);
                let result = handle_key_event(&mut renderer.state, key, &view, num_choices);
                drop(view); // Explicitly drop view to end borrow

                match result {
                    EventResult::Handled => {
                        // State was updated, will redraw on next frame
                    }
                    EventResult::NotHandled => {
                        // For Space key (not handled by shared handler), run one turn
                        if matches!(key, KeyInput::Space) {
                            state.run_one_turn();
                        }
                    }
                    EventResult::Pass | EventResult::Exit => {
                        // Exit the TUI
                        let _ = js_sys::eval("window.exitFancyTui && window.exitFancyTui()");
                    }
                    EventResult::SelectChoice(_idx) => {
                        // Choice selection - not currently used in AI-only mode
                        // but would be used for human player interaction
                    }
                    _ => {}
                }
            }
        }
    });

    // Set up mouse event handling
    terminal.on_mouse_event({
        let state = state.clone();
        move |mouse_event| {
            // Only handle left mouse button press
            if mouse_event.button != MouseButton::Left
                || mouse_event.event != MouseEventKind::Pressed
            {
                return;
            }

            let mut state = state.borrow_mut();

            // Convert pixel coordinates to terminal cell coordinates
            // RatZilla uses window.innerWidth / 10 for cols and innerHeight / 20 for rows
            let cell_x = (mouse_event.x / CELL_WIDTH_PX) as u16;
            let cell_y = (mouse_event.y / CELL_HEIGHT_PX) as u16;

            // Split borrows for mouse handling
            let WasmFancyTuiState {
                ref game,
                ref mut renderer,
                ..
            } = *state;

            let view = GameStateView::new(game, renderer.player_id);
            handle_mouse_click(&mut renderer.state, cell_x, cell_y, &view);
            // State was updated, will redraw on next frame
        }
    });

    // Store state in global for button callbacks
    GLOBAL_TUI_STATE.with(|s| {
        *s.borrow_mut() = Some(state.clone());
    });

    // Set up the render callback
    terminal.draw_web({
        let state = state.clone();
        move |f| {
            let mut state = state.borrow_mut();

            // Auto-run: advance one turn per frame if enabled
            if state.auto_run && !state.game_over {
                state.run_one_turn();
            }

            // Update the turn info in the header
            let turn_number = state.game.turn.turn_number;
            let game_over = state.game_over;
            let _ = js_sys::eval(&format!(
                "window.updateTurnInfo && window.updateTurnInfo({}, {})",
                turn_number, game_over
            ));

            // Split borrows to avoid conflict: we need &game and &mut renderer
            let WasmFancyTuiState {
                ref game,
                ref mut renderer,
                ref current_prompt,
                ref current_choices,
                ..
            } = *state;

            let player_id = renderer.player_id;
            let view = GameStateView::new(game, player_id);
            let prompt = current_prompt.as_deref();
            let choices: Vec<(String, bool)> = current_choices.clone();

            // Draw the TUI using the shared renderer
            renderer.draw_ui(f, &view, prompt, &choices);
        }
    });

    Ok(())
}

/// Helper function to create a game from database (mirrors WasmGame::from_database logic)
fn create_game_from_database(
    card_db: &WasmCardDatabase,
    p1_deck_name: &str,
    p2_deck_name: &str,
    starting_life: i32,
    seed: u64,
) -> Result<GameState, JsValue> {
    // Look up decks
    let p1_deck = card_db
        .decks
        .get(p1_deck_name)
        .ok_or_else(|| JsValue::from_str(&format!("Deck '{}' not found", p1_deck_name)))?;
    let p2_deck = card_db
        .decks
        .get(p2_deck_name)
        .ok_or_else(|| JsValue::from_str(&format!("Deck '{}' not found", p2_deck_name)))?;

    // Create game state
    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), starting_life);
    game.seed_rng(seed);

    // Configure logger for WASM: capture to memory, enable normal verbosity
    game.logger.set_output_mode(OutputMode::Memory);
    game.logger.set_verbosity(VerbosityLevel::Normal);

    let p1_id = game.players[0].id;
    let p2_id = game.players[1].id;

    // Helper to add cards from a deck entry
    let add_deck_cards = |game: &mut GameState,
                          owner: PlayerId,
                          entry: &crate::loader::DeckEntry,
                          cards: &HashMap<String, Arc<CardDefinition>>|
     -> Result<(), String> {
        let card_def = cards
            .get(&entry.card_name)
            .ok_or_else(|| format!("Card '{}' not found in database", entry.card_name))?;

        for _ in 0..entry.count {
            let card_id = game.next_entity_id();
            let card = card_def.instantiate(card_id, owner);
            game.cards.insert(card_id, card);
            if let Some(zones) = game.get_player_zones_mut(owner) {
                zones.library.add(card_id);
            }
        }
        Ok(())
    };

    // Add player 1's deck
    for entry in &p1_deck.main_deck {
        add_deck_cards(&mut game, p1_id, entry, &card_db.cards).map_err(|e| JsValue::from_str(&e))?;
    }

    // Add player 2's deck
    for entry in &p2_deck.main_deck {
        add_deck_cards(&mut game, p2_id, entry, &card_db.cards).map_err(|e| JsValue::from_str(&e))?;
    }

    // Shuffle libraries
    game.shuffle_library(p1_id);
    game.shuffle_library(p2_id);

    // Draw opening hands (7 cards each)
    for _ in 0..7 {
        let _ = game.draw_card(p1_id);
        let _ = game.draw_card(p2_id);
    }

    Ok(game)
}
