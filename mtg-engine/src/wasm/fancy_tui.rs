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
//! - Human input uses the interrupt pattern via run_until_input()

use crate::core::PlayerId;
use crate::game::controller::{ChoiceContext, GameStateView};
use crate::game::fancy_tui_events::{handle_key_event, handle_mouse_click, EventResult, KeyInput};
use crate::game::logger::OutputMode;
use crate::game::{FancyTuiRenderer, GameLoop, GameLoopState, GameState, VerbosityLevel};
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

use super::human_controller::{PendingChoice, WasmHumanController};
use super::{WasmCardDatabase, WasmControllerType};

// Thread-local storage for the global TUI state (for button callbacks)
thread_local! {
    static GLOBAL_TUI_STATE: RefCell<Option<Rc<RefCell<WasmFancyTuiState>>>> = const { RefCell::new(None) };
}

/// Run one turn or continue game - called from JavaScript button
#[wasm_bindgen]
pub fn tui_run_turn() {
    GLOBAL_TUI_STATE.with(|state| {
        if let Some(ref state) = *state.borrow() {
            state.borrow_mut().run_until_choice();
        }
    });
}

/// Select current choice - called from JavaScript or keyboard Enter
#[wasm_bindgen]
pub fn tui_select_choice() {
    GLOBAL_TUI_STATE.with(|state| {
        if let Some(ref state) = *state.borrow() {
            state.borrow_mut().select_current_choice();
        }
    });
}

/// Move to previous choice in the list
#[wasm_bindgen]
pub fn tui_prev_choice() {
    GLOBAL_TUI_STATE.with(|state| {
        if let Some(ref state) = *state.borrow() {
            state.borrow_mut().select_previous_choice();
        }
    });
}

/// Move to next choice in the list
#[wasm_bindgen]
pub fn tui_next_choice() {
    GLOBAL_TUI_STATE.with(|state| {
        if let Some(ref state) = *state.borrow() {
            state.borrow_mut().select_next_choice();
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
    /// Human controller for player 1 (only if p1 is Human)
    p1_human_controller: Option<WasmHumanController>,
    /// Current prompt text
    current_prompt: Option<String>,
    /// Current choices (text, is_highlighted)
    current_choices: Vec<(String, bool)>,
    /// Pending choice context from game loop (waiting for human input)
    pending_context: Option<ChoiceContext>,
    /// Currently selected choice index (for keyboard navigation)
    selected_choice_idx: usize,
    /// Whether the game is over
    game_over: bool,
    /// Error message if any
    error_message: Option<String>,
    /// Auto-run mode (AI vs AI)
    auto_run: bool,
    /// Whether game setup (shuffle, draw opening hands) is done
    setup_done: bool,
}

impl WasmFancyTuiState {
    /// Create a new WASM fancy TUI state from a GameState
    fn new(game: GameState, p1_controller_type: WasmControllerType, p2_controller_type: WasmControllerType) -> Self {
        // Create renderer for player 1's perspective
        let player_id = game.players[0].id;
        let renderer = FancyTuiRenderer::new(player_id, true);

        // Create human controller if player 1 is human
        let p1_human_controller = if p1_controller_type == WasmControllerType::Human {
            Some(WasmHumanController::new(player_id))
        } else {
            None
        };

        let prompt = if p1_controller_type == WasmControllerType::Human {
            "Game ready. Your turn to play!".to_string()
        } else {
            "Game ready. Press Space to advance turn.".to_string()
        };

        Self {
            game,
            renderer,
            p1_controller_type,
            p2_controller_type,
            p1_human_controller,
            current_prompt: Some(prompt),
            current_choices: Vec::new(),
            pending_context: None,
            selected_choice_idx: 0,
            game_over: false,
            error_message: None,
            auto_run: false,
            setup_done: false,
        }
    }

    /// Run the game until input is needed or game ends
    ///
    /// For AI vs AI games, this runs one full turn.
    /// For human player games, this runs until a choice is needed.
    fn run_until_choice(&mut self) {
        if self.game_over {
            return;
        }

        let p1_id = self.game.players[0].id;
        let p2_id = self.game.players[1].id;

        // Create controllers based on type
        // If player 1 is human, we use the stored controller to preserve pending_choice
        let mut p2_controller = self.create_ai_controller(self.p2_controller_type, p2_id);

        // Run using run_until_input for proper human input support
        let result = if self.p1_controller_type == WasmControllerType::Human {
            // Human player - use run_until_input with stored controller
            if let Some(ref mut human) = self.p1_human_controller {
                let mut game_loop = GameLoop::new(&mut self.game).with_verbosity(VerbosityLevel::Normal);
                game_loop.run_until_input(human, p2_controller.as_mut())
            } else {
                // Shouldn't happen, but handle gracefully
                self.error_message = Some("Human controller not initialized".to_string());
                return;
            }
        } else {
            // AI vs AI - run one turn at a time
            let mut p1_controller = self.create_ai_controller(self.p1_controller_type, p1_id);
            let mut game_loop = GameLoop::new(&mut self.game).with_verbosity(VerbosityLevel::Normal);
            game_loop.run_until_input(p1_controller.as_mut(), p2_controller.as_mut())
        };

        match result {
            Ok(GameLoopState::Complete(game_result)) => {
                // Game ended
                self.game_over = true;
                self.pending_context = None;
                self.current_choices.clear();
                if let Some(winner) = game_result.winner {
                    let winner_name = self
                        .game
                        .get_player(winner)
                        .map(|p| p.name.clone())
                        .unwrap_or_else(|_| "Unknown".to_string());
                    self.current_prompt = Some(format!("Game Over! {} wins!", winner_name));
                } else {
                    self.current_prompt = Some("Game Over! Draw!".to_string());
                }
            }
            Ok(GameLoopState::AwaitingInput(context)) => {
                // Need human input - display choices
                self.pending_context = Some(context.clone());
                self.selected_choice_idx = 0;
                self.update_choices_from_context(&context);
            }
            Err(e) => {
                self.error_message = Some(format!("Game error: {}", e));
                self.game_over = true;
            }
        }
    }

    /// Update the current_choices display from a ChoiceContext
    fn update_choices_from_context(&mut self, context: &ChoiceContext) {
        self.current_choices.clear();
        let choices: Vec<String> = match context {
            ChoiceContext::SpellAbility { formatted_choices, .. } => formatted_choices.clone(),
            ChoiceContext::Targets { formatted_targets, .. } => formatted_targets.clone(),
            ChoiceContext::ManaSources { formatted_sources, .. } => formatted_sources.clone(),
            ChoiceContext::Attackers {
                formatted_creatures, ..
            } => {
                let mut choices = vec!["Done (no more attackers)".to_string()];
                choices.extend(formatted_creatures.clone());
                choices
            }
            ChoiceContext::Blockers {
                formatted_blockers,
                formatted_attackers,
                ..
            } => {
                let mut choices = vec!["Done (no blockers)".to_string()];
                for (i, blocker) in formatted_blockers.iter().enumerate() {
                    for (j, attacker) in formatted_attackers.iter().enumerate() {
                        choices.push(format!("{} blocks {} (b{}a{})", blocker, attacker, i, j));
                    }
                }
                choices
            }
            ChoiceContext::DamageOrder { formatted_blockers, .. } => formatted_blockers.clone(),
            ChoiceContext::Discard {
                formatted_hand, count, ..
            } => {
                self.current_prompt = Some(format!("Discard {} card(s):", count));
                formatted_hand.clone()
            }
            ChoiceContext::LibrarySearch { formatted_cards, .. } => {
                let mut choices = vec!["Fail to find".to_string()];
                choices.extend(formatted_cards.clone());
                choices
            }
        };

        // Set prompt based on context type
        let prompt = match context {
            ChoiceContext::SpellAbility { .. } => "Choose an action:".to_string(),
            ChoiceContext::Targets { .. } => "Choose a target:".to_string(),
            ChoiceContext::ManaSources { .. } => "Choose mana sources:".to_string(),
            ChoiceContext::Attackers { .. } => "Declare attackers:".to_string(),
            ChoiceContext::Blockers { .. } => "Declare blockers:".to_string(),
            ChoiceContext::DamageOrder { .. } => "Choose damage order:".to_string(),
            ChoiceContext::Discard { count, .. } => format!("Discard {} card(s):", count),
            ChoiceContext::LibrarySearch { .. } => "Search library:".to_string(),
        };
        self.current_prompt = Some(prompt);

        // Add choices with highlight on first one
        for (idx, choice) in choices.iter().enumerate() {
            self.current_choices
                .push((choice.clone(), idx == self.selected_choice_idx));
        }
    }

    /// Handle selection of current choice index
    fn select_current_choice(&mut self) {
        if self.pending_context.is_none() {
            return;
        }

        let context = self.pending_context.take().unwrap();
        let idx = self.selected_choice_idx;

        // Convert selection index to PendingChoice based on context type
        let pending = match context {
            ChoiceContext::SpellAbility { .. } => {
                // idx 0 = pass, idx 1+ = ability index - 1
                if idx == 0 {
                    PendingChoice::SpellAbility(None)
                } else {
                    PendingChoice::SpellAbility(Some(idx))
                }
            }
            ChoiceContext::Targets { .. } => PendingChoice::Targets(vec![idx]),
            ChoiceContext::ManaSources { .. } => PendingChoice::ManaSources(vec![idx]),
            ChoiceContext::Attackers { .. } => {
                if idx == 0 {
                    PendingChoice::Attackers(vec![]) // Done
                } else {
                    PendingChoice::Attackers(vec![idx - 1])
                }
            }
            ChoiceContext::Blockers { attackers, .. } => {
                if idx == 0 {
                    PendingChoice::Blockers(vec![]) // Done
                } else {
                    // Decode blocker-attacker pair from index
                    let num_attackers = attackers.len();
                    let pair_idx = idx - 1;
                    let blocker_idx = pair_idx / num_attackers;
                    let attacker_idx = pair_idx % num_attackers;
                    PendingChoice::Blockers(vec![(blocker_idx, attacker_idx)])
                }
            }
            ChoiceContext::DamageOrder { .. } => PendingChoice::DamageOrder(vec![idx]),
            ChoiceContext::Discard { .. } => PendingChoice::Discard(vec![idx]),
            ChoiceContext::LibrarySearch { .. } => {
                if idx == 0 {
                    PendingChoice::LibrarySearch(None)
                } else {
                    PendingChoice::LibrarySearch(Some(idx - 1))
                }
            }
        };

        // Set the pending choice on the human controller
        if let Some(ref mut human) = self.p1_human_controller {
            human.set_pending_choice(pending);
        }

        // Clear choices display
        self.current_choices.clear();
        self.selected_choice_idx = 0;

        // Continue running the game
        self.run_until_choice();
    }

    /// Move selection up in the choice list
    fn select_previous_choice(&mut self) {
        if !self.current_choices.is_empty() && self.selected_choice_idx > 0 {
            self.selected_choice_idx -= 1;
            self.update_choice_highlights();
        }
    }

    /// Move selection down in the choice list
    fn select_next_choice(&mut self) {
        if !self.current_choices.is_empty() && self.selected_choice_idx < self.current_choices.len() - 1 {
            self.selected_choice_idx += 1;
            self.update_choice_highlights();
        }
    }

    /// Update highlight state in current_choices based on selected_choice_idx
    fn update_choice_highlights(&mut self) {
        for (idx, (_, highlighted)) in self.current_choices.iter_mut().enumerate() {
            *highlighted = idx == self.selected_choice_idx;
        }
    }

    /// Create an AI controller based on type
    fn create_ai_controller(
        &self,
        controller_type: WasmControllerType,
        player_id: PlayerId,
    ) -> Box<dyn PlayerController> {
        match controller_type {
            WasmControllerType::Zero => Box::new(ZeroController::new(player_id)),
            WasmControllerType::Random => Box::new(RandomController::with_seed(player_id, 42)),
            WasmControllerType::Heuristic => Box::new(HeuristicController::new(player_id)),
            WasmControllerType::Human => {
                // For P2 as human, we'd need a separate controller
                // For now, fall back to Zero
                Box::new(ZeroController::new(player_id))
            }
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
    let state = Rc::new(RefCell::new(WasmFancyTuiState::new(game, p1_controller, p2_controller)));

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
                // Handle human player input for choice selection
                let has_pending_choice = state.pending_context.is_some();

                if has_pending_choice {
                    // Human player making a choice - handle navigation and selection
                    match key {
                        KeyInput::Up => {
                            state.select_previous_choice();
                            return;
                        }
                        KeyInput::Down => {
                            state.select_next_choice();
                            return;
                        }
                        KeyInput::Enter | KeyInput::Space => {
                            state.select_current_choice();
                            return;
                        }
                        KeyInput::Digit(n) => {
                            // Direct number selection (1-9 for choices 0-8)
                            let idx = if n == 0 { 9 } else { (n - 1) as usize };
                            if idx < state.current_choices.len() {
                                state.selected_choice_idx = idx;
                                state.update_choice_highlights();
                                state.select_current_choice();
                            }
                            return;
                        }
                        _ => {}
                    }
                }

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
                        // For Space key (not handled by shared handler), run game
                        if matches!(key, KeyInput::Space) {
                            state.run_until_choice();
                        }
                    }
                    EventResult::Pass | EventResult::Exit => {
                        // Exit the TUI
                        let _ = js_sys::eval("window.exitFancyTui && window.exitFancyTui()");
                    }
                    EventResult::SelectChoice(idx) => {
                        // Choice selection from hand click - set selection and confirm
                        if idx < state.current_choices.len() {
                            state.selected_choice_idx = idx;
                            state.update_choice_highlights();
                            state.select_current_choice();
                        }
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
            if mouse_event.button != MouseButton::Left || mouse_event.event != MouseEventKind::Pressed {
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

            // Auto-run: advance game per frame if enabled (only for AI vs AI)
            // Don't auto-run if there's a pending choice (waiting for human)
            if state.auto_run && !state.game_over && state.pending_context.is_none() {
                state.run_until_choice();
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
