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
use crate::game::controller::{
    prompt_discard, prompt_spell_ability, prompt_target, ChoiceContext, GameStateView, PROMPT_ATTACKERS,
    PROMPT_BLOCKERS, PROMPT_DAMAGE_ORDER, PROMPT_LIBRARY_SEARCH,
};
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
use super::rich_input_controller::WasmRichInputController;
use super::{WasmCardDatabase, WasmControllerType};
use crate::game::replay_controller::ReplayChoice;
use crate::game::ReplayController;
use crate::undo::GameAction;

// Network controller imports (conditional on wasm-network feature)
#[cfg(feature = "wasm-network")]
use super::network::{ensure_client, SharedNetworkClient, WasmNetworkLocalController, WasmRemoteController};

// Thread-local storage for the global TUI state (for button callbacks)
thread_local! {
    static GLOBAL_TUI_STATE: RefCell<Option<Rc<RefCell<WasmFancyTuiState>>>> = const { RefCell::new(None) };
    // Thread-local storage for the fixed script (set before launching TUI)
    static FIXED_SCRIPT: RefCell<Option<Vec<String>>> = const { RefCell::new(None) };
    // Thread-local storage for measured cell dimensions from JavaScript
    // Default to RatZilla's magic numbers (10x20 pixels per cell)
    static CELL_DIMENSIONS: RefCell<(f32, f32)> = const { RefCell::new((10.0, 20.0)) };
}

/// Set the fixed script for player 1's Fixed controller
///
/// Call this before launch_fancy_tui() when using WasmControllerType::Fixed.
/// The script is a list of commands, one per line. Commands include:
/// - `play <land_name>` - Play a land
/// - `cast <spell_name>` - Cast a spell
/// - `activate <card_name>` - Activate an ability
/// - `pass` or `p` or `0` - Pass priority
/// - `*` - Enter wildcard mode (pass until next command matches)
/// - `1`, `2`, etc. - Select by menu index
#[wasm_bindgen]
pub fn set_p1_fixed_script(script: &str) {
    let commands: Vec<String> = script
        .lines()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect();
    FIXED_SCRIPT.with(|s| {
        *s.borrow_mut() = Some(commands);
    });
}

/// Clear the fixed script for player 1
#[wasm_bindgen]
pub fn clear_p1_fixed_script() {
    FIXED_SCRIPT.with(|s| {
        *s.borrow_mut() = None;
    });
}

/// Clean up global state when exiting the TUI
///
/// Call this before exiting to ensure a clean slate for the next launch.
/// This clears the GLOBAL_TUI_STATE and FIXED_SCRIPT thread-local storage.
#[wasm_bindgen]
pub fn cleanup_tui_state() {
    GLOBAL_TUI_STATE.with(|s| {
        *s.borrow_mut() = None;
    });
    FIXED_SCRIPT.with(|s| {
        *s.borrow_mut() = None;
    });
    log::debug!(target: "wasm_tui", "Cleaned up global TUI state");
}

/// Set cell dimensions measured by JavaScript
///
/// Call this after measuring the browser's font metrics to get accurate
/// pixel dimensions for layout calculations. The values are used by
/// `tui_get_card_positions()` to calculate `layout_width_px` and `layout_height_px`.
///
/// This also updates the renderer's cell dimensions for layout calculations.
///
/// # Arguments
/// * `width_px` - Cell width in pixels (typically 10.0 for RatZilla)
/// * `height_px` - Cell height in pixels (typically 20.0 for RatZilla)
#[wasm_bindgen]
pub fn tui_set_cell_dimensions(width_px: f32, height_px: f32) {
    CELL_DIMENSIONS.with(|dims| {
        *dims.borrow_mut() = (width_px, height_px);
    });

    // Also update the renderer's cell dimensions
    GLOBAL_TUI_STATE.with(|state| {
        if let Some(ref state) = *state.borrow() {
            let mut s = state.borrow_mut();
            s.renderer.set_cell_dimensions(width_px, height_px);
        }
    });

    log::debug!(target: "wasm_tui", "Cell dimensions set: {}x{} px", width_px, height_px);
}

/// Run one turn or continue game - called from JavaScript button
#[wasm_bindgen]
pub fn tui_run_turn() {
    GLOBAL_TUI_STATE.with(|state| {
        if let Some(ref state) = *state.borrow() {
            let mut s = state.borrow_mut();
            s.run_until_choice();
            s.needs_redraw = true; // State changed, need redraw
        }
    });
}

/// Select current choice - called from JavaScript or keyboard Enter
#[wasm_bindgen]
pub fn tui_select_choice() {
    GLOBAL_TUI_STATE.with(|state| {
        if let Some(ref state) = *state.borrow() {
            let mut s = state.borrow_mut();
            s.select_current_choice();
            s.needs_redraw = true; // State changed, need redraw
        }
    });
}

/// Move to previous choice in the list
#[wasm_bindgen]
pub fn tui_prev_choice() {
    GLOBAL_TUI_STATE.with(|state| {
        if let Some(ref state) = *state.borrow() {
            let mut s = state.borrow_mut();
            s.select_previous_choice();
            // needs_redraw already set by select_previous_choice()
        }
    });
}

/// Move to next choice in the list
#[wasm_bindgen]
pub fn tui_next_choice() {
    GLOBAL_TUI_STATE.with(|state| {
        if let Some(ref state) = *state.borrow() {
            let mut s = state.borrow_mut();
            s.select_next_choice();
            // needs_redraw already set by select_next_choice()
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
            s.needs_redraw = true; // UI state changed, need redraw
        }
    });
}

/// Get current battlefield cards as JSON for image overlay
/// Returns a JSON array of {name: string, instance_id: number}[]
#[wasm_bindgen]
pub fn tui_get_battlefield_cards() -> String {
    GLOBAL_TUI_STATE.with(|state| {
        if let Some(ref state) = *state.borrow() {
            let s = state.borrow();
            let mut cards = Vec::new();

            // Get all cards on the battlefield (shared zone)
            for &card_id in &s.game.battlefield.cards {
                if let Ok(card) = s.game.cards.get(card_id) {
                    cards.push(serde_json::json!({
                        "name": format!("{}", card.name),
                        "instance_id": format!("{:?}", card_id),
                    }));
                }
            }

            serde_json::to_string(&cards).unwrap_or_else(|_| "[]".to_string())
        } else {
            "[]".to_string()
        }
    })
}

/// Helper function to export card positions from renderer state
/// This is called from within the render loop, so it doesn't need to borrow GLOBAL_TUI_STATE
///
/// EntityPosition now stores:
/// - `area`: MAX bounds (cells) - the card's public bounding box
/// - `layout_area_px`: Optional pixel-based layout (if in GUI mode)
///
/// Note: Wildcard is intentional - Entity enum has many variants (hand cards, etc.);
/// we only export battlefield card positions for overlays.
#[allow(clippy::wildcard_enum_match_arm)]
fn export_card_positions_from_renderer(
    entity_positions: &[crate::game::fancy_tui_renderer::EntityPosition],
    game: &GameState,
    player_id: PlayerId,
) -> String {
    use crate::game::fancy_tui_renderer::{CardBounds, Entity};

    let mut positions = Vec::new();
    let view = GameStateView::new(game, player_id);

    // Get cell dimensions from thread-local storage (for fallback calculation)
    let (cell_w_px, cell_h_px) = CELL_DIMENSIONS.with(|dims| *dims.borrow());

    // Helper to create JSON object with layout bounds
    // If layout_area_px is provided, use it; otherwise calculate from cell dimensions
    let make_card_json = |card_id: &crate::core::CardId,
                          name: &str,
                          x: u16,
                          y: u16,
                          width: u16,
                          height: u16,
                          is_tapped: bool,
                          layout_area_px: Option<&crate::game::fancy_tui_renderer::LayoutAreaPx>|
     -> serde_json::Value {
        // Use stored pixel layout if available, otherwise calculate using CardBounds
        let (layout_w_px, layout_h_px) = if let Some(layout) = layout_area_px {
            (layout.width_px, layout.height_px)
        } else {
            // Fallback: calculate MAX bounds using CardBounds for correct MTG aspect ratio
            let bounds = if is_tapped {
                CardBounds::for_gui_tapped(width, height, cell_w_px, cell_h_px)
            } else {
                CardBounds::for_gui(width, height, cell_w_px, cell_h_px)
            };
            (bounds.max_width_px, bounds.max_height_px)
        };

        serde_json::json!({
            "card_id": format!("{:?}", card_id),
            "name": name,
            "x": x,
            "y": y,
            "width": width,
            "height": height,
            "is_tapped": is_tapped,
            "layout_width_px": layout_w_px,
            "layout_height_px": layout_h_px,
        })
    };

    // Extract card positions from renderer state
    // EntityPosition.area is now MAX bounds (the card's public bounding box)
    // We include SingleCard entities and cards from VisualStack/SimpleStack
    for entity_pos in entity_positions {
        match &entity_pos.entity {
            Entity::SingleCard { card_id, .. } => {
                // Check if this card is on the battlefield
                if game.battlefield.cards.contains(card_id) {
                    if let Ok(card) = game.cards.get(*card_id) {
                        let is_tapped = view.is_tapped(*card_id);
                        positions.push(make_card_json(
                            card_id,
                            &card.name.to_string(),
                            entity_pos.area.x,
                            entity_pos.area.y,
                            entity_pos.area.width,
                            entity_pos.area.height,
                            is_tapped,
                            entity_pos.layout_area_px.as_ref(),
                        ));
                    }
                }
            }
            Entity::VisualStack {
                card_ids, tapped_count, ..
            } => {
                // For visual stacks, target the TOP card with diagonal offset
                // The visual stack renderer draws cards diagonally from bottom-left to top-right
                // So we need to offset to the top-right position
                if let Some(&card_id) = card_ids.last() {
                    // LAST card is on top visually
                    if game.battlefield.cards.contains(&card_id) {
                        if let Ok(card) = game.cards.get(card_id) {
                            let is_tapped = *tapped_count > 0;
                            let stack_depth = card_ids.len() as u16;
                            let offset = stack_depth.saturating_sub(1); // DIAGONAL_OFFSET = 1

                            // Adjust layout_area_px for offset if present
                            let adjusted_layout = entity_pos.layout_area_px.as_ref().map(|l| {
                                crate::game::fancy_tui_renderer::LayoutAreaPx {
                                    x_px: l.x_px + f32::from(offset) * cell_w_px,
                                    y_px: l.y_px + f32::from(offset) * cell_h_px,
                                    width_px: l.width_px - f32::from(offset) * cell_w_px,
                                    height_px: l.height_px - f32::from(offset) * cell_h_px,
                                }
                            });

                            positions.push(make_card_json(
                                &card_id,
                                &card.name.to_string(),
                                entity_pos.area.x + offset,
                                entity_pos.area.y + offset,
                                entity_pos.area.width.saturating_sub(offset),
                                entity_pos.area.height.saturating_sub(offset),
                                is_tapped,
                                adjusted_layout.as_ref(),
                            ));
                        }
                    }
                }
            }
            Entity::SimpleStack {
                card_ids, is_tapped, ..
            } => {
                // For simple stacks, use the is_tapped field from the entity
                if let Some(&card_id) = card_ids.first() {
                    if game.battlefield.cards.contains(&card_id) {
                        if let Ok(card) = game.cards.get(card_id) {
                            positions.push(make_card_json(
                                &card_id,
                                &card.name.to_string(),
                                entity_pos.area.x,
                                entity_pos.area.y,
                                entity_pos.area.width,
                                entity_pos.area.height,
                                *is_tapped,
                                entity_pos.layout_area_px.as_ref(),
                            ));
                        }
                    }
                }
            }
            _ => {} // Skip hand cards and other entities
        }
    }

    serde_json::to_string(&positions).unwrap_or_else(|_| "[]".to_string())
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
    /// Fixed script controller for player 1 (only if p1 is Fixed)
    p1_fixed_controller: Option<WasmRichInputController>,
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
    /// Whether we need to replay choices after user makes a new choice
    /// When true, we rewind to turn start and replay all choices including the new one
    needs_replay: bool,
    /// Whether the UI needs to be redrawn on the next frame
    /// This dirty bit prevents unnecessary redraws at 60Hz when nothing has changed
    needs_redraw: bool,
    /// Whether we're in network mode (separate from controller type)
    /// In network mode, P1 uses WasmNetworkLocalController and P2 uses WasmRemoteController
    #[cfg(feature = "wasm-network")]
    is_network_mode: bool,
    /// Controller seed for deterministic RandomController (network mode)
    /// IMPORTANT: This must match the seed used by native client to ensure identical behavior.
    /// See mtg-d0jg3 for WASM/native behavioral identity requirements.
    #[cfg(feature = "wasm-network")]
    controller_seed: u64,
    /// High-water mark for action count (for monotonicity invariant checking)
    /// This tracks the maximum action count seen during FORWARD progress.
    /// During rewind/replay, action count is allowed to decrease, but after
    /// replay completes, it should exceed this value.
    high_water_action_count: usize,
    /// High-water mark for log count (for monotonicity invariant checking)
    /// Similar to action count - log should only grow during forward progress.
    high_water_log_count: usize,
    /// Whether we're currently in a rewind/replay cycle
    /// Used to suppress monotonicity checks during replay
    in_rewind_replay: bool,
}

impl WasmFancyTuiState {
    /// Create a new WASM fancy TUI state from a GameState (local game mode)
    fn new(game: GameState, p1_controller_type: WasmControllerType, p2_controller_type: WasmControllerType) -> Self {
        Self::new_with_network_mode(game, p1_controller_type, p2_controller_type, false, 0)
    }

    /// Create a new WASM fancy TUI state with explicit network mode flag
    ///
    /// # Parameters
    /// - `controller_seed`: Seed for RandomController. MUST match native client's seed
    ///   for behavioral identity (see mtg-d0jg3).
    fn new_with_network_mode(
        game: GameState,
        p1_controller_type: WasmControllerType,
        p2_controller_type: WasmControllerType,
        #[allow(unused_variables)] is_network_mode: bool,
        #[allow(unused_variables)] controller_seed: u64,
    ) -> Self {
        // In network mode, determine which player we control.
        // The non-Remote controller is ours; Remote is the opponent.
        let (our_player_idx, our_controller) = if is_network_mode && p1_controller_type == WasmControllerType::Remote {
            (1, p2_controller_type) // We are P2
        } else {
            (0, p1_controller_type) // We are P1 (or local mode)
        };

        // Create renderer for OUR player's perspective in GUI mode
        let player_id = game.players[our_player_idx].id;
        let (cell_w, cell_h) = CELL_DIMENSIONS.with(|dims| *dims.borrow());
        let renderer = FancyTuiRenderer::new_gui(player_id, true, cell_w, cell_h);

        // Create human controller for our player (could be P1 or P2 in network mode)
        // In network mode with Human controller, this is wrapped by WasmNetworkLocalController
        let p1_human_controller = if our_controller == WasmControllerType::Human {
            Some(WasmHumanController::new(player_id))
        } else {
            None
        };

        // Create fixed script controller if our player is Fixed
        let p1_fixed_controller = if our_controller == WasmControllerType::Fixed {
            // Get the script from thread-local storage (set via set_p1_fixed_script)
            let commands = FIXED_SCRIPT.with(|s| s.borrow().clone()).unwrap_or_default();
            Some(WasmRichInputController::new(player_id, commands))
        } else {
            None
        };

        let prompt = match our_controller {
            WasmControllerType::Human => "Game ready. Your turn to play!".to_string(),
            WasmControllerType::Fixed => "Game ready. Running fixed script...".to_string(),
            WasmControllerType::Random
            | WasmControllerType::Heuristic
            | WasmControllerType::Zero
            | WasmControllerType::Network
            | WasmControllerType::Remote => {
                if is_network_mode {
                    "Network AI game running...".to_string()
                } else {
                    "Game ready. Press Space to advance turn.".to_string()
                }
            }
        };

        // Auto-run for AI controllers in network mode (they don't need user input)
        // Also auto-run for Fixed controller (scripted play)
        #[cfg(feature = "wasm-network")]
        let auto_run = is_network_mode && !matches!(our_controller, WasmControllerType::Human);
        #[cfg(not(feature = "wasm-network"))]
        let auto_run = false;

        Self {
            game,
            renderer,
            p1_controller_type,
            p2_controller_type,
            p1_human_controller,
            p1_fixed_controller,
            current_prompt: Some(prompt),
            current_choices: Vec::new(),
            pending_context: None,
            selected_choice_idx: 0,
            game_over: false,
            error_message: None,
            auto_run,
            needs_replay: false,
            needs_redraw: true, // Initial draw required
            #[cfg(feature = "wasm-network")]
            is_network_mode,
            #[cfg(feature = "wasm-network")]
            controller_seed,
            high_water_action_count: 0,
            high_water_log_count: 0,
            in_rewind_replay: false,
        }
    }

    /// Check and update monotonicity invariants
    ///
    /// This verifies that action count and log count are monotonically increasing
    /// during forward progress. Violations indicate a bug in the rewind/replay logic.
    ///
    /// Returns an error message if an invariant is violated, None otherwise.
    fn check_monotonicity_invariants(&mut self) -> Option<String> {
        // Skip checks during rewind/replay - values are expected to decrease
        if self.in_rewind_replay {
            return None;
        }

        let current_action_count = self.game.undo_log.len();
        let current_log_count = self.game.logger.log_count();

        // Check action count monotonicity
        if current_action_count < self.high_water_action_count {
            let msg = format!(
                "MONOTONICITY VIOLATION: Action count decreased from {} to {} outside of rewind!",
                self.high_water_action_count, current_action_count
            );
            log::error!(target: "wasm_tui", "{}", msg);
            return Some(msg);
        }

        // Check log count monotonicity
        if current_log_count < self.high_water_log_count {
            let msg = format!(
                "MONOTONICITY VIOLATION: Log count decreased from {} to {} outside of rewind!",
                self.high_water_log_count, current_log_count
            );
            log::error!(target: "wasm_tui", "{}", msg);
            return Some(msg);
        }

        // Update high-water marks
        self.high_water_action_count = current_action_count;
        self.high_water_log_count = current_log_count;

        None
    }

    /// Rewind game state to turn start and return P1's choices made so far
    ///
    /// This undoes all game state changes since the start of the turn,
    /// returning only P1's ReplayChoice entries that were logged.
    ///
    /// IMPORTANT: We only extract choices for P1 (the human player) because:
    /// - P1's choices will be replayed via ReplayController
    /// - P2's choices will be re-made by the AI controller during replay
    /// - If we included P2's choices in P1's replay queue, P1 would try to
    ///   execute P2's actions (e.g., casting P2's spells) causing "Card not in hand" errors
    fn rewind_to_turn_start(&mut self) -> Vec<ReplayChoice> {
        let log_len_before = self.game.undo_log.len();
        log::debug!(target: "wasm_tui", "REWIND: Undo log has {} actions before rewind", log_len_before);

        let mut undo_log = std::mem::take(&mut self.game.undo_log);
        let result = undo_log.rewind_to_turn_start(&mut self.game);
        self.game.undo_log = undo_log;

        let log_len_after = self.game.undo_log.len();

        // rewind_to_turn_start returns None only if undo log is disabled
        // (which shouldn't happen for WASM TUI, but handle gracefully)
        let (turn_number, choice_actions, actions_rewound, log_size_at_turn) = match result {
            Some(r) => r,
            None => {
                log::warn!(target: "wasm_tui", "REWIND: Undo log disabled!");
                return Vec::new();
            }
        };

        // Truncate game logs to match the rewound state
        // This removes log entries generated after the turn started, preventing duplicates
        // when we replay the choices
        self.game.logger.truncate_to(log_size_at_turn);

        // Get P1's player ID for filtering choices
        let p1_id = self.game.players[0].id;

        // Count total and P1's choices for logging
        let total_choices = choice_actions
            .iter()
            .filter(|a| matches!(a, GameAction::ChoicePoint { choice: Some(_), .. }))
            .count();

        log::debug!(
            target: "wasm_tui",
            "REWIND: Rewound to turn {}, {} actions undone, log now {} actions, {} total choice points",
            turn_number, actions_rewound, log_len_after, total_choices
        );

        // Extract ReplayChoice from ChoicePoint actions, filtering to only P1's choices
        // This prevents the "Card not in hand" bug where P1's ReplayController
        // would try to replay P2's actions (like casting P2's spells)
        let p1_choices: Vec<ReplayChoice> = choice_actions
            .into_iter()
            .filter_map(|action| {
                if let GameAction::ChoicePoint {
                    player_id,
                    choice: Some(c),
                    ..
                } = action
                {
                    // Only include choices made by P1 (human player)
                    if player_id == p1_id {
                        Some(c)
                    } else {
                        log::debug!(
                            target: "wasm_tui",
                            "REWIND: Skipping P2 choice (will be re-made by AI): {:?}",
                            c
                        );
                        None
                    }
                } else {
                    None
                }
            })
            .collect();

        log::debug!(
            target: "wasm_tui",
            "REWIND: Extracted {} P1 choices for replay",
            p1_choices.len()
        );

        p1_choices
    }

    /// Convert a PendingChoice to a ReplayChoice using the current pending_context
    #[allow(clippy::collapsible_match)]
    fn pending_choice_to_replay_choice(&self, pending: &PendingChoice) -> ReplayChoice {
        match pending {
            PendingChoice::SpellAbility(opt_idx) => {
                if let Some(ref context) = self.pending_context {
                    if let ChoiceContext::SpellAbility { available, .. } = context {
                        match opt_idx {
                            None | Some(0) => ReplayChoice::SpellAbility(None),
                            Some(idx) => {
                                let ability_idx = idx - 1;
                                if ability_idx < available.len() {
                                    ReplayChoice::SpellAbility(Some(available[ability_idx].clone()))
                                } else {
                                    ReplayChoice::SpellAbility(None)
                                }
                            }
                        }
                    } else {
                        ReplayChoice::SpellAbility(None)
                    }
                } else {
                    ReplayChoice::SpellAbility(None)
                }
            }
            PendingChoice::Targets(indices) => {
                if let Some(ref context) = self.pending_context {
                    if let ChoiceContext::Targets { valid_targets, .. } = context {
                        let targets: smallvec::SmallVec<[crate::core::CardId; 4]> =
                            indices.iter().filter_map(|i| valid_targets.get(*i).copied()).collect();
                        ReplayChoice::Targets(targets)
                    } else {
                        ReplayChoice::Targets(smallvec::SmallVec::new())
                    }
                } else {
                    ReplayChoice::Targets(smallvec::SmallVec::new())
                }
            }
            PendingChoice::ManaSources(indices) => {
                if let Some(ref context) = self.pending_context {
                    if let ChoiceContext::ManaSources { available_sources, .. } = context {
                        let sources: smallvec::SmallVec<[crate::core::CardId; 8]> = indices
                            .iter()
                            .filter_map(|i| available_sources.get(*i).copied())
                            .collect();
                        ReplayChoice::ManaSources(sources)
                    } else {
                        ReplayChoice::ManaSources(smallvec::SmallVec::new())
                    }
                } else {
                    ReplayChoice::ManaSources(smallvec::SmallVec::new())
                }
            }
            PendingChoice::Attackers(indices) => {
                if let Some(ref context) = self.pending_context {
                    if let ChoiceContext::Attackers {
                        available_creatures, ..
                    } = context
                    {
                        let attackers: smallvec::SmallVec<[crate::core::CardId; 8]> = indices
                            .iter()
                            .filter_map(|i| available_creatures.get(*i).copied())
                            .collect();
                        ReplayChoice::Attackers(attackers)
                    } else {
                        ReplayChoice::Attackers(smallvec::SmallVec::new())
                    }
                } else {
                    ReplayChoice::Attackers(smallvec::SmallVec::new())
                }
            }
            PendingChoice::Blockers(pairs) => {
                if let Some(ref context) = self.pending_context {
                    if let ChoiceContext::Blockers {
                        available_blockers,
                        attackers,
                        ..
                    } = context
                    {
                        let blockers: smallvec::SmallVec<[(crate::core::CardId, crate::core::CardId); 8]> = pairs
                            .iter()
                            .filter_map(|(bi, ai)| {
                                let blocker = available_blockers.get(*bi).copied()?;
                                let attacker = attackers.get(*ai).copied()?;
                                Some((blocker, attacker))
                            })
                            .collect();
                        ReplayChoice::Blockers(blockers)
                    } else {
                        ReplayChoice::Blockers(smallvec::SmallVec::new())
                    }
                } else {
                    ReplayChoice::Blockers(smallvec::SmallVec::new())
                }
            }
            PendingChoice::DamageOrder(indices) => {
                if let Some(ref context) = self.pending_context {
                    if let ChoiceContext::DamageOrder { blockers, .. } = context {
                        let order: smallvec::SmallVec<[crate::core::CardId; 4]> =
                            indices.iter().filter_map(|i| blockers.get(*i).copied()).collect();
                        ReplayChoice::DamageOrder(order)
                    } else {
                        ReplayChoice::DamageOrder(smallvec::SmallVec::new())
                    }
                } else {
                    ReplayChoice::DamageOrder(smallvec::SmallVec::new())
                }
            }
            PendingChoice::Discard(indices) => {
                if let Some(ref context) = self.pending_context {
                    if let ChoiceContext::Discard { hand, .. } = context {
                        let cards: smallvec::SmallVec<[crate::core::CardId; 7]> =
                            indices.iter().filter_map(|i| hand.get(*i).copied()).collect();
                        ReplayChoice::Discard(cards)
                    } else {
                        ReplayChoice::Discard(smallvec::SmallVec::new())
                    }
                } else {
                    ReplayChoice::Discard(smallvec::SmallVec::new())
                }
            }
            PendingChoice::LibrarySearch(opt_idx) => {
                // Return the index directly - the game loop converts to CardId
                match opt_idx {
                    None => ReplayChoice::LibrarySearch(None),
                    Some(idx) => ReplayChoice::LibrarySearch(Some(*idx)),
                }
            }
            PendingChoice::Sacrifice(indices) => {
                if let Some(ref context) = self.pending_context {
                    if let ChoiceContext::SacrificePermanents { valid_permanents, .. } = context {
                        let permanents: smallvec::SmallVec<[crate::core::CardId; 8]> = indices
                            .iter()
                            .filter_map(|i| valid_permanents.get(*i).copied())
                            .collect();
                        ReplayChoice::Sacrifice(permanents)
                    } else {
                        ReplayChoice::Sacrifice(smallvec::SmallVec::new())
                    }
                } else {
                    ReplayChoice::Sacrifice(smallvec::SmallVec::new())
                }
            }
            PendingChoice::Modes(indices) => {
                // Convert mode indices to ReplayChoice
                let modes: smallvec::SmallVec<[usize; 4]> = indices.iter().copied().collect();
                ReplayChoice::Modes(modes)
            }
        }
    }

    /// Run the game until input is needed or game ends
    ///
    /// For AI vs AI games, this runs one turn at a time (for step-through mode).
    /// For human player games, this uses the rewind/replay pattern:
    ///
    /// 1. If needs_replay is true (user just made a choice):
    ///    - Rewind game state to the start of the current turn
    ///    - Create a ReplayController with all choices from this turn (including new one)
    ///    - Run the game - it will replay deterministically and continue
    ///
    /// 2. If needs_replay is false (initial run or after turn boundary):
    ///    - Run until we hit NeedInput or game ends
    ///
    /// This pattern is necessary because when NeedInput is thrown, the call stack
    /// unwinds completely. ChoiceContext is NOT a continuation - it doesn't capture
    /// the stack state. To resume mid-turn, we must rewind and replay all choices.
    fn run_until_choice(&mut self) {
        if self.game_over {
            return;
        }

        let p1_id = self.game.players[0].id;
        let p2_id = self.game.players[1].id;

        // Network mode takes precedence - check early and handle separately
        #[cfg(feature = "wasm-network")]
        if self.is_network_mode {
            self.run_network_mode(p1_id, p2_id);
            return;
        }

        // Create P2 controller (for local games)
        let mut p2_controller = self.create_ai_controller(self.p2_controller_type, p2_id);

        if self.p1_controller_type == WasmControllerType::Human {
            // Human player - use rewind/replay pattern
            if self.needs_replay {
                // User just made a choice - rewind and replay
                self.needs_replay = false;

                // Mark that we're in rewind/replay mode (suppresses monotonicity checks)
                self.in_rewind_replay = true;

                let turn_before = self.game.turn.turn_number;
                log::debug!(target: "wasm_tui", "REPLAY: Starting replay on turn {}", turn_before);

                // Get the new choice from the human controller
                let new_choice = if let Some(ref mut human) = self.p1_human_controller {
                    if let Some(pending) = human.pending_choice.take() {
                        // Convert PendingChoice to ReplayChoice using current context
                        let choice = self.pending_choice_to_replay_choice(&pending);
                        log::debug!(target: "wasm_tui", "REPLAY: New choice = {:?}", choice);
                        Some(choice)
                    } else {
                        log::debug!(target: "wasm_tui", "REPLAY: No pending choice");
                        None
                    }
                } else {
                    None
                };

                // Rewind game state and get previous choices from this turn
                let mut replay_choices = self.rewind_to_turn_start();
                let turn_after_rewind = self.game.turn.turn_number;
                log::debug!(
                    target: "wasm_tui",
                    "REPLAY: After rewind - turn {}, {} existing choices to replay",
                    turn_after_rewind, replay_choices.len()
                );

                // Add the new choice if we have one
                if let Some(choice) = new_choice {
                    replay_choices.push(choice);
                }
                log::debug!(target: "wasm_tui", "REPLAY: Total choices to replay: {}", replay_choices.len());

                // Create a fresh human controller for the replay.
                // The WasmHumanController doesn't need persistent state - all choices
                // are captured in the replay_choices, and any NEW choice will be
                // handled via handle_game_result() setting pending_context, which
                // then prompts the user for input via the UI.
                let human_controller = WasmHumanController::new(p1_id);

                // Create ReplayController that will replay choices then delegate to human
                let mut replay_controller = ReplayController::new(p1_id, Box::new(human_controller), replay_choices);

                // Run the game with replay controller
                // Scope game_loop tightly so self can be accessed afterwards
                let result = {
                    let mut game_loop = GameLoop::new(&mut self.game).with_verbosity(VerbosityLevel::Normal);
                    log::debug!(target: "wasm_tui", "REPLAY: Running game loop with replay controller...");
                    game_loop.run_until_input(&mut replay_controller, p2_controller.as_mut())
                };

                let turn_after_run = self.game.turn.turn_number;
                log::debug!(
                    target: "wasm_tui",
                    "REPLAY: Game loop returned on turn {}, result type: {}",
                    turn_after_run,
                    match &result {
                        Ok(GameLoopState::Complete(_)) => "Complete",
                        Ok(GameLoopState::AwaitingInput(_)) => "AwaitingInput",
                        Err(_) => "Error",
                    }
                );

                // Note: We don't need to recover the inner controller because:
                // 1. Any choices already made are captured in replay_choices
                // 2. New choices come through handle_game_result -> pending_context -> UI
                // 3. The self.p1_human_controller is used for getting the pending_choice,
                //    but we've already extracted it above

                // Replay complete - clear the rewind flag
                self.in_rewind_replay = false;

                // Reset high water marks to establish new baseline after replay
                // During replay, P2 (AI) makes fresh decisions that may result in different
                // action counts than the original path. This is expected behavior, not a bug.
                // We reset the baseline here rather than checking for violations.
                self.high_water_action_count = self.game.undo_log.len();
                self.high_water_log_count = self.game.logger.log_count();
                log::debug!(
                    target: "wasm_tui",
                    "REPLAY: Reset high water marks after replay - action_count={}, log_count={}",
                    self.high_water_action_count, self.high_water_log_count
                );

                // Handle the game result - this may set pending_context for the next choice
                self.handle_game_result(result);
                self.needs_redraw = true; // State changed, need redraw
            } else {
                // Normal run - no replay needed
                log::debug!(
                    target: "wasm_tui",
                    "NORMAL: Running game loop (no replay), turn {}",
                    self.game.turn.turn_number
                );
                if let Some(ref mut human) = self.p1_human_controller {
                    // Scope game_loop tightly so self can be accessed afterwards
                    let result = {
                        let mut game_loop = GameLoop::new(&mut self.game).with_verbosity(VerbosityLevel::Normal);
                        game_loop.run_until_input(human, p2_controller.as_mut())
                    };
                    let turn_after = self.game.turn.turn_number;
                    log::debug!(
                        target: "wasm_tui",
                        "NORMAL: Game loop returned on turn {}, result type: {}",
                        turn_after,
                        match &result {
                            Ok(GameLoopState::Complete(_)) => "Complete",
                            Ok(GameLoopState::AwaitingInput(_)) => "AwaitingInput",
                            Err(_) => "Error",
                        }
                    );

                    // Check monotonicity invariants after normal run
                    if let Some(violation_msg) = self.check_monotonicity_invariants() {
                        self.error_message = Some(violation_msg);
                        self.game_over = true;
                        self.needs_redraw = true;
                        return;
                    }

                    self.handle_game_result(result);
                    self.needs_redraw = true; // State changed, need redraw
                } else {
                    self.error_message = Some("Human controller not initialized".to_string());
                    self.needs_redraw = true; // Error message changed, need redraw
                }
            }
        } else if self.p1_controller_type == WasmControllerType::Fixed {
            // Fixed script controller - runs the script without user input
            // Uses the same rewind/replay pattern but choices come from the script
            if let Some(ref mut fixed) = self.p1_fixed_controller {
                // Scope game_loop tightly so self can be accessed in match arms
                let result = {
                    let mut game_loop = GameLoop::new(&mut self.game).with_verbosity(VerbosityLevel::Normal);
                    game_loop.run_until_input(fixed, p2_controller.as_mut())
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
                                .map(|p| p.name.to_string())
                                .unwrap_or_else(|_| "Unknown".to_string());
                            self.current_prompt = Some(format!("Game Over! {} wins!", winner_name));
                        } else {
                            self.current_prompt = Some("Game Over! Draw!".to_string());
                        }
                        self.needs_redraw = true; // State changed, need redraw
                    }
                    Ok(GameLoopState::AwaitingInput(context)) => {
                        // Script paused - show the context to user (similar to human)
                        self.pending_context = Some(context.clone());
                        self.selected_choice_idx = 0;
                        self.update_choices_from_context(&context);
                        self.current_prompt = Some("Fixed script waiting - press Space to continue".to_string());
                        self.needs_redraw = true; // State changed, need redraw
                    }
                    Err(e) => {
                        self.error_message = Some(format!("Fixed script error: {}", e));
                        self.game_over = true;
                        self.needs_redraw = true; // State changed, need redraw
                    }
                }
            } else {
                self.error_message = Some("Fixed controller not initialized".to_string());
                self.needs_redraw = true; // Error message changed, need redraw
            }
        } else {
            // AI vs AI - run one turn at a time for step-through mode
            let mut p1_controller = self.create_ai_controller(self.p1_controller_type, p1_id);

            // Scope game_loop tightly so self.game can be accessed in match arms
            let result = {
                let mut game_loop = GameLoop::new(&mut self.game).with_verbosity(VerbosityLevel::Normal);
                game_loop.run_one_turn(p1_controller.as_mut(), p2_controller.as_mut())
            };
            match result {
                Ok(Some(game_result)) => {
                    // Game ended
                    self.game_over = true;
                    self.pending_context = None;
                    self.current_choices.clear();
                    if let Some(winner) = game_result.winner {
                        let winner_name = self
                            .game
                            .get_player(winner)
                            .map(|p| p.name.to_string())
                            .unwrap_or_else(|_| "Unknown".to_string());
                        self.current_prompt = Some(format!("Game Over! {} wins!", winner_name));
                    } else {
                        self.current_prompt = Some("Game Over! Draw!".to_string());
                    }
                    self.needs_redraw = true; // State changed, need redraw
                }
                Ok(None) => {
                    // Turn completed, game continues
                    let turn = self.game.turn.turn_number;
                    self.current_prompt = Some(format!("Turn {} complete. Press Space for next turn.", turn));
                    self.needs_redraw = true; // State changed, need redraw
                }
                Err(e) => {
                    self.error_message = Some(format!("Game error: {}", e));
                    self.game_over = true;
                    self.needs_redraw = true; // State changed, need redraw
                }
            }
        }
    }

    /// Run the game in network mode
    ///
    /// Network mode coordinates with the server:
    /// - Our player uses WasmNetworkLocalController wrapping our controller type
    /// - Opponent uses WasmRemoteController receiving choices from server
    /// - Which player is "ours" depends on p1_controller_type vs p2_controller_type
    /// - For Human controller: uses rewind/replay pattern for resumable game loops
    /// - For AI controllers (Random, Heuristic, Zero): runs directly without replay
    #[cfg(feature = "wasm-network")]
    fn run_network_mode(&mut self, _p1_id: PlayerId, _p2_id: PlayerId) {
        // Get the shared network client
        let network_client = ensure_client();

        // Get our player ID directly from the server assignment (like native client)
        // This is authoritative - the server determines who is P1 and P2
        let our_player_id = match network_client.borrow().our_player_id() {
            Some(id) => id,
            None => {
                log::error!("run_network_mode: Server has not assigned player ID yet");
                self.error_message = Some("Server has not assigned player ID".to_string());
                self.needs_redraw = true;
                return;
            }
        };

        // Determine which player we are based on server assignment (like native client)
        // Native client: `let we_are_p1 = our_player_id.as_u32() == 0;`
        let we_are_p1 = our_player_id.as_u32() == 0;
        let opponent_id = if we_are_p1 { PlayerId::new(1) } else { PlayerId::new(0) };

        // Get our controller type based on server assignment
        let our_controller_type = if we_are_p1 {
            self.p1_controller_type
        } else {
            self.p2_controller_type
        };

        log::info!(
            "run_network_mode: server assigned us {:?}, we_are_p1={}, opponent_id={:?}, our_controller={:?}",
            our_player_id,
            we_are_p1,
            opponent_id,
            our_controller_type
        );

        // Create remote controller for opponent
        let mut remote_controller = WasmRemoteController::new(opponent_id, network_client.clone());

        // Handle based on our controller type
        match our_controller_type {
            WasmControllerType::Human => {
                // Human controller - use rewind/replay pattern (same as local Human mode)
                self.run_network_mode_human_v2(
                    our_player_id,
                    opponent_id,
                    we_are_p1,
                    network_client,
                    &mut remote_controller,
                );
            }
            WasmControllerType::Random => {
                // Random controller - runs directly without user input
                // IMPORTANT: Use self.controller_seed to match native client behavior (mtg-d0jg3)
                let inner = RandomController::with_seed(our_player_id, self.controller_seed);
                let mut network_local = WasmNetworkLocalController::new(inner, network_client.clone());
                self.run_network_mode_ai_v2(our_player_id, we_are_p1, &mut network_local, &mut remote_controller);
            }
            WasmControllerType::Heuristic => {
                // Heuristic controller
                let inner = HeuristicController::new(our_player_id);
                let mut network_local = WasmNetworkLocalController::new(inner, network_client.clone());
                self.run_network_mode_ai_v2(our_player_id, we_are_p1, &mut network_local, &mut remote_controller);
            }
            WasmControllerType::Zero => {
                // Zero controller (always passes)
                let inner = ZeroController::new(our_player_id);
                let mut network_local = WasmNetworkLocalController::new(inner, network_client.clone());
                self.run_network_mode_ai_v2(our_player_id, we_are_p1, &mut network_local, &mut remote_controller);
            }
            WasmControllerType::Remote => {
                // This shouldn't happen - our_controller_type should never be Remote
                self.error_message = Some("Bug: our_controller_type is Remote".to_string());
                self.needs_redraw = true;
            }
            _ => {
                self.error_message = Some(format!(
                    "Unsupported controller type {:?} for network mode",
                    our_controller_type
                ));
                self.needs_redraw = true;
            }
        }
    }

    /// Run network mode with Human controller (uses rewind/replay pattern)
    ///
    /// # Arguments
    /// * `our_id` - Our player ID
    /// * `opponent_id` - Opponent's player ID (unused, for symmetry)
    /// * `we_are_p1` - Whether we are player 1 (index 0) or player 2 (index 1)
    /// * `network_client` - The shared network client
    /// * `remote_controller` - The remote controller for opponent
    #[cfg(feature = "wasm-network")]
    #[allow(unused_variables)]
    fn run_network_mode_human_v2(
        &mut self,
        our_id: PlayerId,
        opponent_id: PlayerId,
        we_are_p1: bool,
        network_client: SharedNetworkClient,
        remote_controller: &mut WasmRemoteController,
    ) {
        if self.needs_replay {
            // User just made a choice - rewind and replay
            self.needs_replay = false;

            // Mark that we're in rewind/replay mode (suppresses monotonicity checks)
            self.in_rewind_replay = true;

            let turn_before = self.game.turn.turn_number;
            log::debug!(target: "wasm_tui", "NETWORK REPLAY: Starting replay on turn {}", turn_before);

            // Take the new pending choice from the stored human controller
            // IMPORTANT: Don't add this to replay_choices! It needs to go through
            // WasmNetworkLocalController so it gets submitted to the server.
            let new_pending_choice = if let Some(ref mut human) = self.p1_human_controller {
                human.pending_choice.take()
            } else {
                None
            };

            if let Some(ref choice) = new_pending_choice {
                log::debug!(target: "wasm_tui", "NETWORK REPLAY: New pending choice = {:?}", choice);
            }

            // Rewind game state and get previous choices from this turn
            let replay_choices = self.rewind_to_turn_start();
            log::debug!(
                target: "wasm_tui",
                "NETWORK REPLAY: After rewind - turn {}, {} existing choices to replay",
                self.game.turn.turn_number, replay_choices.len()
            );

            // NOTE: We do NOT add new_pending_choice to replay_choices!
            // The new choice must go through WasmNetworkLocalController to be
            // submitted to the server. If we add it to replay, it bypasses the server.

            // Create a fresh human controller and set the pending choice on it
            let mut human_controller = WasmHumanController::new(our_id);
            if let Some(pending) = new_pending_choice {
                human_controller.set_pending_choice(pending);
            }
            let network_local = WasmNetworkLocalController::new(human_controller, network_client.clone());

            // Create ReplayController that will replay choices then delegate to network local
            let mut replay_controller = ReplayController::new(our_id, Box::new(network_local), replay_choices);

            // Run the game with replay controller
            // Scope game_loop tightly so self can be accessed afterwards
            let result = {
                // Create sync callback that processes pending reveals
                let client_for_sync = network_client.clone();
                let local_player = our_id;
                let sync_callback = move |game: &mut GameState, _target_action: u64| {
                    let reveals = client_for_sync.borrow_mut().drain_reveals();
                    if !reveals.is_empty() {
                        log::debug!(
                            "WASM sync_callback (replay): processing {} reveals at action_count={}",
                            reveals.len(),
                            game.action_count()
                        );
                        for (owner, card, reason) in reveals {
                            process_card_reveal_wasm(game, owner, card, reason, Some(local_player));
                        }
                    }
                };

                let mut game_loop = GameLoop::new(&mut self.game)
                    .with_verbosity(VerbosityLevel::Normal)
                    .with_sync_callback(sync_callback)
                    .skip_opening_hands() // Server handles opening hands via CardRevealed
                    .with_deferred_game_end(); // Server is authoritative about game end

                // Pass controllers in correct order based on which player we are
                if we_are_p1 {
                    game_loop.run_until_input(&mut replay_controller, remote_controller)
                } else {
                    game_loop.run_until_input(remote_controller, &mut replay_controller)
                }
            };

            let turn_after_run = self.game.turn.turn_number;
            log::debug!(
                target: "wasm_tui",
                "NETWORK REPLAY: Game loop returned on turn {}",
                turn_after_run
            );

            // Replay complete - clear the rewind flag
            self.in_rewind_replay = false;

            // Check monotonicity invariants after network replay
            if let Some(violation_msg) = self.check_monotonicity_invariants() {
                self.error_message = Some(violation_msg);
                self.game_over = true;
                self.needs_redraw = true;
                return;
            }

            self.handle_game_result(result);
            self.needs_redraw = true;
        } else {
            // Normal run - no replay needed
            log::debug!(
                target: "wasm_tui",
                "NETWORK NORMAL: Running game loop, turn {}, we_are_p1={}",
                self.game.turn.turn_number,
                we_are_p1
            );

            if let Some(ref mut human) = self.p1_human_controller {
                // Create network local controller wrapping the human controller
                // Note: We need to take ownership for the game loop, but we clone the inner state
                let inner_clone = human.clone();
                let mut network_local = WasmNetworkLocalController::new(inner_clone, network_client.clone());

                // Scope game_loop so borrow of self.game ends before accessing self
                let result = {
                    // Create sync callback that processes pending reveals
                    let client_for_sync = network_client.clone();
                    let local_player = our_id;
                    let sync_callback = move |game: &mut GameState, _target_action: u64| {
                        let reveals = client_for_sync.borrow_mut().drain_reveals();
                        if !reveals.is_empty() {
                            log::debug!(
                                "WASM sync_callback (normal): processing {} reveals at action_count={}",
                                reveals.len(),
                                game.action_count()
                            );
                            for (owner, card, reason) in reveals {
                                process_card_reveal_wasm(game, owner, card, reason, Some(local_player));
                            }
                        }
                    };

                    let mut game_loop = GameLoop::new(&mut self.game)
                        .with_verbosity(VerbosityLevel::Normal)
                        .with_sync_callback(sync_callback)
                        .skip_opening_hands() // Server handles opening hands via CardRevealed
                        .with_deferred_game_end(); // Server is authoritative about game end

                    // Pass controllers in correct order based on which player we are
                    if we_are_p1 {
                        game_loop.run_until_input(&mut network_local, remote_controller)
                    } else {
                        game_loop.run_until_input(remote_controller, &mut network_local)
                    }
                };

                let turn_number = self.game.turn.turn_number;
                log::debug!(
                    target: "wasm_tui",
                    "NETWORK NORMAL: Game loop returned on turn {}",
                    turn_number
                );

                // Check monotonicity invariants after normal network run
                if let Some(violation_msg) = self.check_monotonicity_invariants() {
                    self.error_message = Some(violation_msg);
                    self.game_over = true;
                    self.needs_redraw = true;
                    return;
                }

                self.handle_game_result(result);
                self.needs_redraw = true;
            } else {
                self.error_message = Some("Human controller not initialized for network mode".to_string());
                self.needs_redraw = true;
            }
        }
    }

    /// Run network mode with an AI controller (no replay needed)
    ///
    /// # Arguments
    /// * `_our_id` - Our player ID (for logging)
    /// * `we_are_p1` - Whether we are player 1 (index 0) or player 2 (index 1)
    /// * `our_controller` - Our local controller
    /// * `remote_controller` - The remote controller for opponent
    #[cfg(feature = "wasm-network")]
    fn run_network_mode_ai_v2<C: PlayerController>(
        &mut self,
        our_id: PlayerId,
        we_are_p1: bool,
        our_controller: &mut WasmNetworkLocalController<C>,
        remote_controller: &mut WasmRemoteController,
    ) {
        let start_turn = self.game.turn.turn_number;
        log::debug!(
            target: "wasm_tui",
            "NETWORK AI: Running game loop, turn {}, we_are_p1={}",
            start_turn,
            we_are_p1
        );

        // Get the network client for the sync callback
        let network_client = ensure_client();

        // Scope game_loop so borrow of self.game ends before accessing self
        let result = {
            // Create sync callback that processes pending reveals
            // This is the WASM equivalent of the native client's sync_callback
            let client_for_sync = network_client.clone();
            let local_player = our_id;
            let sync_callback = move |game: &mut GameState, _target_action: u64| {
                // Drain all pending reveals and process them
                let reveals = client_for_sync.borrow_mut().drain_reveals();
                if !reveals.is_empty() {
                    log::debug!(
                        "WASM sync_callback: processing {} reveals at action_count={}",
                        reveals.len(),
                        game.action_count()
                    );
                    for (owner, card, reason) in reveals {
                        process_card_reveal_wasm(game, owner, card, reason, Some(local_player));
                    }
                }
            };

            let mut game_loop = GameLoop::new(&mut self.game)
                .with_verbosity(VerbosityLevel::Normal)
                .with_sync_callback(sync_callback)
                .skip_opening_hands() // Server handles opening hands via CardRevealed
                .with_deferred_game_end(); // Server is authoritative about game end

            // Pass controllers in the correct order based on which player we are
            // GameLoop expects (p1_controller, p2_controller)
            if we_are_p1 {
                game_loop.run_until_input(our_controller, remote_controller)
            } else {
                game_loop.run_until_input(remote_controller, our_controller)
            }
        };

        let end_turn = self.game.turn.turn_number;
        log::debug!(
            target: "wasm_tui",
            "NETWORK AI: Game loop returned on turn {}",
            end_turn
        );

        self.handle_game_result(result);
        self.needs_redraw = true;
    }

    /// Handle game result after running (for human player games)
    fn handle_game_result(&mut self, result: crate::Result<GameLoopState>) {
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
                        .map(|p| p.name.to_string())
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

                // Debug logging: show game state when waiting for input
                let turn = self.game.turn.turn_number;
                let phase = format!("{:?}", self.game.turn.current_step);
                let active_player = self.game.turn.active_player;
                let p1_id = self.game.players[0].id;
                let is_p1_turn = active_player == p1_id;
                let choice_count = self.current_choices.len();
                let context_type = match &context {
                    ChoiceContext::SpellAbility { available, .. } => format!("SpellAbility({})", available.len()),
                    ChoiceContext::Targets { .. } => "Targets".to_string(),
                    ChoiceContext::ManaSources { .. } => "ManaSources".to_string(),
                    ChoiceContext::Attackers { .. } => "Attackers".to_string(),
                    ChoiceContext::Blockers { .. } => "Blockers".to_string(),
                    ChoiceContext::DamageOrder { .. } => "DamageOrder".to_string(),
                    ChoiceContext::Discard { .. } => "Discard".to_string(),
                    ChoiceContext::LibrarySearch { .. } => "LibrarySearch".to_string(),
                    ChoiceContext::SacrificePermanents { .. } => "SacrificePermanents".to_string(),
                    ChoiceContext::Modes { mode_count, .. } => format!("Modes({})", mode_count),
                };
                log::debug!(
                    target: "wasm_tui",
                    "Turn {}, {}, P1's turn: {}, choices: {}, context: {}",
                    turn, phase, is_p1_turn, choice_count, context_type
                );
            }
            Err(e) => {
                self.error_message = Some(format!("Game error: {}", e));
                self.game_over = true;
            }
        }
    }

    /// Update the current_choices display from a ChoiceContext
    ///
    /// Uses shared formatting functions from controller.rs to ensure consistency
    /// with native TUI.
    fn update_choices_from_context(&mut self, context: &ChoiceContext) {
        self.current_choices.clear();
        let choices: Vec<String> = match context {
            ChoiceContext::SpellAbility { formatted_choices, .. } => formatted_choices.clone(),
            ChoiceContext::Targets { formatted_targets, .. } => {
                // Add "No target" as first option to match TUI
                std::iter::once("No target".to_string())
                    .chain(formatted_targets.clone())
                    .collect()
            }
            ChoiceContext::ManaSources { formatted_sources, .. } => formatted_sources.clone(),
            ChoiceContext::Attackers {
                formatted_creatures, ..
            } => {
                // Use "Done" to match TUI (not "Done (no more attackers)")
                let mut choices = vec!["Done".to_string()];
                choices.extend(formatted_creatures.clone());
                choices
            }
            ChoiceContext::Blockers {
                formatted_blockers,
                formatted_attackers,
                ..
            } => {
                // Use "Done" to match TUI and simpler block format without indices
                let mut choices = vec!["Done".to_string()];
                for blocker in formatted_blockers.iter() {
                    for attacker in formatted_attackers.iter() {
                        choices.push(format!("{} blocks {}", blocker, attacker));
                    }
                }
                choices
            }
            ChoiceContext::DamageOrder { formatted_blockers, .. } => formatted_blockers.clone(),
            ChoiceContext::Discard { formatted_hand, .. } => formatted_hand.clone(),
            ChoiceContext::LibrarySearch { formatted_cards, .. } => {
                let mut choices = vec!["Fail to find".to_string()];
                choices.extend(formatted_cards.clone());
                choices
            }
            ChoiceContext::SacrificePermanents {
                formatted_permanents, ..
            } => {
                let mut choices = vec!["Done".to_string()];
                choices.extend(formatted_permanents.clone());
                choices
            }
            ChoiceContext::Modes { formatted_modes, .. } => formatted_modes.clone(),
        };

        // Set prompt based on context type using shared prompt functions
        let prompt = match context {
            ChoiceContext::SpellAbility { .. } => {
                // Get player name from priority player
                if let Some(priority_player) = self.game.turn.priority_player {
                    let player_name = self
                        .game
                        .players
                        .iter()
                        .find(|p| p.id == priority_player)
                        .map(|p| p.name.as_str())
                        .unwrap_or("Player");
                    prompt_spell_ability(player_name)
                } else {
                    prompt_spell_ability("Player")
                }
            }
            ChoiceContext::Targets { spell_id, .. } => {
                // Get spell name from game state
                let spell_name = self
                    .game
                    .cards
                    .get(*spell_id)
                    .map(|c| c.name.as_str())
                    .unwrap_or("spell");
                prompt_target(spell_name)
            }
            ChoiceContext::ManaSources { .. } => "Choose mana source:".to_string(),
            ChoiceContext::Attackers { .. } => PROMPT_ATTACKERS.to_string(),
            ChoiceContext::Blockers { .. } => PROMPT_BLOCKERS.to_string(),
            ChoiceContext::DamageOrder { .. } => PROMPT_DAMAGE_ORDER.to_string(),
            ChoiceContext::Discard { count, .. } => prompt_discard(*count),
            ChoiceContext::LibrarySearch { .. } => PROMPT_LIBRARY_SEARCH.to_string(),
            ChoiceContext::SacrificePermanents {
                count,
                card_type_description,
                ..
            } => {
                format!("Choose {} {} to sacrifice:", count, card_type_description)
            }
            ChoiceContext::Modes {
                mode_count, spell_id, ..
            } => {
                let spell_name = self
                    .game
                    .cards
                    .get(*spell_id)
                    .map(|c| c.name.as_str())
                    .unwrap_or("spell");
                format!("Choose {} mode(s) for {}:", mode_count, spell_name)
            }
        };
        self.current_prompt = Some(prompt);

        // Format choices with numeric prefixes using shared function
        self.current_choices = crate::game::display::format_choices_with_numbers(&choices, self.selected_choice_idx);
    }

    /// Handle selection of current choice index
    ///
    /// When the user makes a selection:
    /// 1. Convert the selection index to a PendingChoice
    /// 2. Set it on the human controller
    /// 3. Set needs_replay = true (we'll need to rewind and replay)
    /// 4. Call run_until_choice() which will do the rewind/replay
    fn select_current_choice(&mut self) {
        if self.pending_context.is_none() {
            return;
        }

        // Check if this is a "waiting" context (network mode waiting for server/ack)
        // These contexts have empty available options and should not trigger selection
        if let Some(ChoiceContext::SpellAbility { available, .. }) = &self.pending_context {
            if available.is_empty() {
                // This is a "waiting for server" or "waiting for acknowledgment" context
                // Don't allow selection - just re-run the game loop to check for updates
                self.run_until_choice();
                return;
            }
        }

        // Don't take the context yet - we need it for pending_choice_to_replay_choice
        let idx = self.selected_choice_idx;

        // Convert selection index to PendingChoice based on context type
        let pending = {
            let context = self.pending_context.as_ref().unwrap();
            match context {
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
                ChoiceContext::SacrificePermanents { .. } => {
                    if idx == 0 {
                        PendingChoice::Sacrifice(vec![]) // Done
                    } else {
                        PendingChoice::Sacrifice(vec![idx - 1])
                    }
                }
                ChoiceContext::Modes { .. } => {
                    // idx directly maps to mode index (0-based)
                    PendingChoice::Modes(vec![idx])
                }
            }
        };

        // Set the pending choice on the human controller
        if let Some(ref mut human) = self.p1_human_controller {
            human.set_pending_choice(pending);
        }

        // Mark that we need to rewind and replay when run_until_choice is called
        // DON'T clear pending_context yet - we need it for pending_choice_to_replay_choice
        self.needs_replay = true;

        // Clear UI state
        self.current_choices.clear();
        self.selected_choice_idx = 0;

        // Continue running the game with rewind/replay
        // Note: run_until_choice() may set a NEW pending_context if we hit another choice point
        // So we don't clear pending_context here - it will be overwritten or left as-is
        self.run_until_choice();
    }

    /// Move selection up in the choice list
    fn select_previous_choice(&mut self) {
        if !self.current_choices.is_empty() && self.selected_choice_idx > 0 {
            self.selected_choice_idx -= 1;
            self.update_choice_highlights();
            self.needs_redraw = true; // UI state changed, need redraw
        }
    }

    /// Move selection down in the choice list
    fn select_next_choice(&mut self) {
        if !self.current_choices.is_empty() && self.selected_choice_idx < self.current_choices.len() - 1 {
            self.selected_choice_idx += 1;
            self.update_choice_highlights();
            self.needs_redraw = true; // UI state changed, need redraw
        }
    }

    /// Update highlight state in current_choices based on selected_choice_idx
    fn update_choice_highlights(&mut self) {
        for (idx, (_, highlighted)) in self.current_choices.iter_mut().enumerate() {
            *highlighted = idx == self.selected_choice_idx;
        }
    }

    /// Create an AI controller based on type
    ///
    /// NOTE: This is used for LOCAL (non-network) games only. In network mode,
    /// controllers are created directly with the proper seed from self.controller_seed
    /// to ensure behavioral identity with native client (see mtg-d0jg3).
    fn create_ai_controller(
        &self,
        controller_type: WasmControllerType,
        player_id: PlayerId,
    ) -> Box<dyn PlayerController> {
        match controller_type {
            WasmControllerType::Zero => Box::new(ZeroController::new(player_id)),
            // For local games, seed doesn't need to match native - use arbitrary seed
            WasmControllerType::Random => Box::new(RandomController::with_seed(player_id, 42)),
            WasmControllerType::Heuristic => Box::new(HeuristicController::new(player_id)),
            WasmControllerType::Human | WasmControllerType::Fixed | WasmControllerType::Network => {
                // Human, Fixed, and Network controllers for P1 are handled separately in run_until_choice
                // For P2 as human/fixed/network, fall back to Zero
                Box::new(ZeroController::new(player_id))
            }
            #[cfg(feature = "wasm-network")]
            WasmControllerType::Remote => {
                // Remote controller for network opponent - polls network client for choices
                let client = ensure_client();
                Box::new(WasmRemoteController::new(player_id, client))
            }
            #[cfg(not(feature = "wasm-network"))]
            WasmControllerType::Remote => {
                // Remote controller requires wasm-network feature
                log::warn!("Remote controller type requires wasm-network feature, falling back to Zero");
                Box::new(ZeroController::new(player_id))
            }
        }
    }
}

/// Launch the WASM fancy TUI in the browser
///
/// This function creates and runs the RatZilla-based TUI application.
///
/// Note: Wildcards are intentional - ratzilla KeyCode has 25+ variants, KeyInput
/// and FocusedPane have many variants; we handle the subset used in WASM TUI.
///
/// # Errors
///
/// Returns a `JsValue` error if game creation from the database fails.
///
/// # Panics
///
/// Panics if mutex locks are poisoned or internal channel operations fail.
#[wasm_bindgen]
#[allow(clippy::too_many_arguments)]
#[allow(clippy::wildcard_enum_match_arm)]
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

    // For human controller games, run until the first choice point
    // This populates the initial choice list for the player
    // For fixed controller, also run to start processing the script
    if p1_controller == WasmControllerType::Human || p1_controller == WasmControllerType::Fixed {
        state.borrow_mut().run_until_choice();
    }

    // Create the RatZilla backend, targeting our specific container element
    let backend = DomBackend::new_by_id("ratzilla-terminal")
        .map_err(|e| JsValue::from_str(&format!("Failed to create backend: {}", e)))?;
    let terminal =
        Terminal::new(backend).map_err(|e| JsValue::from_str(&format!("Failed to create terminal: {}", e)))?;

    // Set up keyboard event handling using shared event handler
    terminal.on_key_event({
        let state = Rc::clone(&state);
        move |key_event| {
            let mut state = state.borrow_mut();

            // Convert RatZilla KeyCode to our abstract KeyInput
            let key_input = match key_event.code {
                KeyCode::Char(' ') => Some(KeyInput::Space),
                KeyCode::Char('a' | 'A') => {
                    // A: toggle auto-run (WASM-specific, not shared)
                    state.auto_run = !state.auto_run;
                    state.needs_redraw = true; // UI state changed, need redraw
                    return;
                }
                KeyCode::Char('i') => {
                    // lowercase 'i': toggle card images (WASM-specific, not shared)
                    let _ = js_sys::eval("window.toggleCardImages && window.toggleCardImages()");
                    return;
                }
                KeyCode::Char('q' | 'Q') => Some(KeyInput::Pass),
                KeyCode::Esc => Some(KeyInput::Escape),
                KeyCode::Char('h' | 'H') => Some(KeyInput::FocusHand),
                KeyCode::Char('I') => Some(KeyInput::FocusInfo), // uppercase I for Info pane
                KeyCode::Char('y' | 'Y') => Some(KeyInput::FocusYourBf),
                KeyCode::Char('o' | 'O') => Some(KeyInput::FocusOpponentBf),
                KeyCode::Char('s' | 'S') => Some(KeyInput::FocusStack),
                KeyCode::Char('b' | 'B') => Some(KeyInput::ShowBattlefield),
                KeyCode::Char('c' | 'C') => {
                    // C: toggle controls panel visibility (WASM-specific)
                    let _ = js_sys::eval("document.getElementById('btn-toggle-controls')?.click()");
                    return;
                }
                KeyCode::Char('?') => Some(KeyInput::Help),
                KeyCode::Char('w' | 'W') => Some(KeyInput::ToggleWrap),
                KeyCode::Tab => Some(KeyInput::Tab),
                KeyCode::Up => Some(KeyInput::Up),
                KeyCode::Down => Some(KeyInput::Down),
                KeyCode::Left => Some(KeyInput::Left),
                KeyCode::Right => Some(KeyInput::Right),
                KeyCode::PageUp => Some(KeyInput::PageUp),
                KeyCode::PageDown => Some(KeyInput::PageDown),
                KeyCode::Home => Some(KeyInput::Home),
                KeyCode::End => Some(KeyInput::End),
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
                            // needs_redraw already set by select_previous_choice()
                            return;
                        }
                        KeyInput::Down => {
                            state.select_next_choice();
                            // needs_redraw already set by select_next_choice()
                            return;
                        }
                        KeyInput::Enter | KeyInput::Space => {
                            state.select_current_choice();
                            state.needs_redraw = true; // State changed, need redraw
                            return;
                        }
                        KeyInput::Digit(n) => {
                            // Direct number selection (1-9 for choices 0-8)
                            let idx = if n == 0 { 9 } else { (n - 1) as usize };
                            if idx < state.current_choices.len() {
                                state.selected_choice_idx = idx;
                                state.update_choice_highlights();
                                state.select_current_choice();
                                state.needs_redraw = true; // State changed, need redraw
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
                // view borrow ends here when we're done with it

                match result {
                    EventResult::Handled => {
                        // State was updated, will redraw on next frame
                        state.needs_redraw = true; // Renderer state changed, need redraw
                    }
                    EventResult::NotHandled => {
                        // For Space key (not handled by shared handler), run game
                        if matches!(key, KeyInput::Space) {
                            state.run_until_choice();
                            state.needs_redraw = true; // State changed, need redraw
                        }
                    }
                    EventResult::Pass | EventResult::Exit => {
                        // Show exit confirmation dialog
                        let _ = js_sys::eval("window.showExitConfirmation && window.showExitConfirmation()");
                    }
                    EventResult::SelectChoice(idx) => {
                        // Choice selection from hand click - set selection and confirm
                        if idx < state.current_choices.len() {
                            state.selected_choice_idx = idx;
                            state.update_choice_highlights();
                            state.select_current_choice();
                            state.needs_redraw = true; // State changed, need redraw
                        }
                    }
                    EventResult::ShowBattlefield => {
                        // Log battlefield state to console
                        // Create a fresh view since we dropped the previous one
                        let WasmFancyTuiState {
                            ref game, ref renderer, ..
                        } = *state;
                        let view = GameStateView::new(game, renderer.player_id);
                        let bf_text = crate::game::display::format_battlefield_for_log(&view);
                        log::info!("{}", bf_text);
                    }
                    EventResult::ShowHelp => {
                        // Show help dialog
                        let _ = js_sys::eval("window.showHelpDialog && window.showHelpDialog()");
                    }
                    _ => {}
                }
            }
        }
    });

    // Set up mouse event handling
    // Note: RatZilla doesn't support scroll wheel events, so only handle clicks
    terminal.on_mouse_event({
        let state = Rc::clone(&state);
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

            // Split borrows for mouse click handling
            let WasmFancyTuiState {
                ref game,
                ref mut renderer,
                ..
            } = *state;

            let view = GameStateView::new(game, renderer.player_id);
            handle_mouse_click(&mut renderer.state, cell_x, cell_y, &view);
            // State was updated, will redraw on next frame
            state.needs_redraw = true; // State changed, need redraw
        }
    });

    // Store state in global for button callbacks
    GLOBAL_TUI_STATE.with(|s| {
        *s.borrow_mut() = Some(Rc::clone(&state));
    });

    // Set up the render callback
    terminal.draw_web({
        let state = state;
        move |f| {
            let mut state = state.borrow_mut();

            // Auto-run: advance game per frame if enabled (only for AI vs AI)
            // Don't auto-run if there's a pending choice (waiting for human)
            if state.auto_run && !state.game_over && state.pending_context.is_none() {
                state.run_until_choice();
                state.needs_redraw = true; // Game state changed, need redraw
            }

            // Split borrows to avoid conflict: we need &game and &mut renderer
            let WasmFancyTuiState {
                ref game,
                ref mut renderer,
                ref current_prompt,
                ref current_choices,
                ref needs_redraw, // Borrow the dirty bit to check it
                ..
            } = *state;

            let player_id = renderer.player_id;
            let view = GameStateView::new(game, player_id);
            let prompt = current_prompt.as_deref();
            let choices: Vec<(String, bool)> = current_choices.clone();

            // IMPORTANT: We must ALWAYS draw the frame, even when state hasn't changed!
            // RatZilla/Ratatui uses immediate-mode rendering where each frame must be
            // fully populated. If we skip drawing, the frame will be empty -> black screen.
            renderer.draw_ui(f, &view, prompt, &choices);

            // Optimization: Only run expensive JavaScript callbacks when state has changed
            if *needs_redraw {
                // Clear dirty bit - borrows end when we re-destructure below
                state.needs_redraw = false;

                // Re-borrow for JavaScript callbacks
                let WasmFancyTuiState {
                    ref game, ref renderer, ..
                } = *state;

                // Update the turn info in the header
                let turn_number = game.turn.turn_number;
                let game_over = state.game_over;
                let _ = js_sys::eval(&format!(
                    "window.updateTurnInfo && window.updateTurnInfo({}, {})",
                    turn_number, game_over
                ));

                // Export card positions for image overlays
                let positions_json =
                    export_card_positions_from_renderer(&renderer.state.entity_positions, game, player_id);

                // Export selected card info for Card Details image overlay
                // Include the pane position so JavaScript can properly position the image
                let selected_card_json = if let Some(card_id) = renderer.state.selected_card_id {
                    if let Ok(card) = game.cards.get(card_id) {
                        // Escape quotes in card name for JSON
                        let escaped_name = card.name.as_str().replace('\"', "\\\"");
                        // Include pane area if available
                        if let Some(pane_area) = renderer.state.card_details_pane_area {
                            format!(
                                r#"{{"card_id": {}, "name": "{}", "pane": {{"x": {}, "y": {}, "width": {}, "height": {}}}}}"#,
                                card_id.as_u32(),
                                escaped_name,
                                pane_area.x,
                                pane_area.y,
                                pane_area.width,
                                pane_area.height
                            )
                        } else {
                            format!(r#"{{"card_id": {}, "name": "{}"}}"#, card_id.as_u32(), escaped_name)
                        }
                    } else {
                        "null".to_string()
                    }
                } else {
                    "null".to_string()
                };

                // Post-render hook: notify JavaScript that rendering is complete
                // Pass the card positions and selected card so JavaScript doesn't need to call back into WASM
                let js_code = format!(
                    "window.onRenderComplete && window.onRenderComplete({}, {})",
                    positions_json, selected_card_json
                );
                let _ = js_sys::eval(&js_code);
            }
        }
    });

    Ok(())
}

/// Launch a network game TUI
///
/// Called when the server signals game start (GameStarted message received).
/// The game state will be synchronized with the server.
///
/// Uses the GameStarted data stored in WasmNetworkClient to initialize:
/// - Starting life totals
/// - Player assignment (we may be P0 or P1)
/// - Controller types (Network for us, Remote for opponent)
#[wasm_bindgen]
#[cfg(feature = "wasm-network")]
pub fn launch_network_game(
    card_db: &WasmCardDatabase,
    deck_name: &str,
    controller_type: &str,
    controller_seed: u64,
    _canvas_width: u32,
    _canvas_height: u32,
) -> Result<(), JsValue> {
    log::info!(
        "launch_network_game: Initializing network game TUI with controller={}, seed={}",
        controller_type,
        controller_seed
    );

    // Parse the controller type from JavaScript
    // IMPORTANT: Only human and random are currently supported in WASM network mode.
    // Heuristic and zero are disabled until we achieve proper state synchronization
    // with the native client (mtg-d0jg3).
    let our_controller_type = match controller_type {
        "human" => WasmControllerType::Human,
        "random" => WasmControllerType::Random,
        "heuristic" | "zero" => {
            log::warn!(
                "launch_network_game: Controller '{}' disabled in WASM network mode. \
                 Only 'human' and 'random' are supported until state sync is fixed (mtg-d0jg3). \
                 Defaulting to Human.",
                controller_type
            );
            WasmControllerType::Human
        }
        _ => {
            log::warn!(
                "launch_network_game: Unknown controller type '{}', defaulting to Human",
                controller_type
            );
            WasmControllerType::Human
        }
    };

    // Get network client to read GameStarted data
    let client = ensure_client();
    let client_ref = client.borrow();

    // Get starting life from server (or default to 20)
    let starting_life = client_ref.starting_life();
    let our_player_id = client_ref.our_player_id();
    let opponent_name = client_ref.opponent_name().unwrap_or("Opponent").to_string();
    let our_name = deck_name.to_string(); // Use deck name as our name for now

    // CRITICAL: Get late-binding architecture data (mtg-d0jg3)
    let deck_card_ids = client_ref.deck_card_ids().cloned();
    let rng_state = client_ref.rng_state().to_vec();

    log::info!(
        "launch_network_game: starting_life={}, our_player_id={:?}, opponent={}, deck_card_ids={:?}",
        starting_life,
        our_player_id,
        opponent_name,
        deck_card_ids
            .as_ref()
            .map(|r| format!("P1:[{}..{}), P2:[{}..{})", r.p1_start, r.p1_end, r.p2_start, r.p2_end))
    );

    // Drop the borrow before creating the game
    drop(client_ref);

    // Create the game using late-binding architecture (mtg-d0jg3)
    // CRITICAL: Use init_game_reserve_only_wasm() with server's DeckCardIdRanges
    // This ensures WASM uses the SAME CardIDs as the server for behavioral identity.
    let game = if let Some(ref ranges) = deck_card_ids {
        log::info!("launch_network_game: Using late-binding CardID architecture with ranges");

        // Determine player names based on player assignment
        let (p1_name, p2_name) = match our_player_id {
            Some(pid) if pid.as_u32() == 0 => (our_name.clone(), opponent_name.clone()),
            Some(pid) if pid.as_u32() == 1 => (opponent_name.clone(), our_name.clone()),
            _ => (our_name.clone(), opponent_name.clone()),
        };

        // Create game with reserved CardID slots (same as native client)
        let mut game = init_game_reserve_only_wasm(p1_name, p2_name, starting_life, ranges);

        // Initialize RNG from server state for deterministic shuffles
        if !rng_state.is_empty() {
            use rand_chacha::ChaCha12Rng;
            match bincode::deserialize::<ChaCha12Rng>(&rng_state) {
                Ok(rng) => {
                    *game.rng.borrow_mut() = rng;
                    log::info!(
                        "launch_network_game: Initialized RNG from server state ({} bytes)",
                        rng_state.len()
                    );
                }
                Err(e) => {
                    log::error!(
                        "launch_network_game: Failed to deserialize RNG state: {} - shuffles may diverge!",
                        e
                    );
                }
            }
        } else {
            log::warn!("launch_network_game: No RNG state from server - shuffles may diverge!");
        }

        // Enable reveal logging for network games (same as native)
        game.set_skip_reveals(false);

        game
    } else {
        // Fallback: legacy mode without late-binding (will cause CardID mismatches!)
        log::warn!(
            "launch_network_game: No DeckCardIdRanges from server! Using legacy initialization. \
             This WILL cause CardID mismatches and state desync!"
        );
        let seed = crate::network::now_ms();
        create_game_from_database(card_db, deck_name, deck_name, starting_life, seed)?
    };

    // Determine controller types based on our player assignment
    // Our controller type is what we selected, opponent is always Remote
    // If we're player 0: P1=our controller, P2=Remote
    // If we're player 1: P1=Remote, P2=our controller (but we still view from P1 perspective)
    let (p1_controller_type, p2_controller_type) = match our_player_id {
        Some(pid) if pid.as_u32() == 0 => {
            log::info!("launch_network_game: We are P1 (index 0)");
            (our_controller_type, WasmControllerType::Remote)
        }
        Some(pid) if pid.as_u32() == 1 => {
            log::info!("launch_network_game: We are P2 (index 1)");
            // P1 is remote opponent, P2 is us
            (WasmControllerType::Remote, our_controller_type)
        }
        _ => {
            log::warn!("launch_network_game: Player ID not assigned, defaulting to P1");
            (our_controller_type, WasmControllerType::Remote)
        }
    };

    // Create the shared state with network mode enabled
    let state = Rc::new(RefCell::new(WasmFancyTuiState::new_with_network_mode(
        game,
        p1_controller_type,
        p2_controller_type,
        true, // is_network_mode = true
        controller_seed,
    )));

    // Run until the first choice point
    state.borrow_mut().run_until_choice();

    // Create the RatZilla backend
    let backend = DomBackend::new_by_id("ratzilla-terminal")
        .map_err(|e| JsValue::from_str(&format!("Failed to create backend: {}", e)))?;
    let terminal =
        Terminal::new(backend).map_err(|e| JsValue::from_str(&format!("Failed to create terminal: {}", e)))?;

    // Set up keyboard event handling (same as launch_fancy_tui)
    terminal.on_key_event({
        let state = state.clone();
        move |key_event| {
            let mut state = state.borrow_mut();

            // Convert RatZilla KeyCode to our abstract KeyInput
            let key_input = match key_event.code {
                KeyCode::Char(' ') => Some(KeyInput::Space),
                KeyCode::Char('a') | KeyCode::Char('A') => {
                    state.auto_run = !state.auto_run;
                    state.needs_redraw = true;
                    return;
                }
                KeyCode::Char('i') => {
                    let _ = js_sys::eval("window.toggleCardImages && window.toggleCardImages()");
                    return;
                }
                KeyCode::Char('q') | KeyCode::Char('Q') => Some(KeyInput::Pass),
                KeyCode::Esc => Some(KeyInput::Escape),
                KeyCode::Char('h') | KeyCode::Char('H') => Some(KeyInput::FocusHand),
                KeyCode::Char('I') => Some(KeyInput::FocusInfo),
                KeyCode::Char('y') | KeyCode::Char('Y') => Some(KeyInput::FocusYourBf),
                KeyCode::Char('o') | KeyCode::Char('O') => Some(KeyInput::FocusOpponentBf),
                KeyCode::Char('s') | KeyCode::Char('S') => Some(KeyInput::FocusStack),
                KeyCode::Char('b') | KeyCode::Char('B') => Some(KeyInput::ShowBattlefield),
                KeyCode::Char('c') | KeyCode::Char('C') => {
                    let _ = js_sys::eval("document.getElementById('btn-toggle-controls')?.click()");
                    return;
                }
                KeyCode::Char('?') => Some(KeyInput::Help),
                KeyCode::Char('w') | KeyCode::Char('W') => Some(KeyInput::ToggleWrap),
                KeyCode::Tab => Some(KeyInput::Tab),
                KeyCode::Up => Some(KeyInput::Up),
                KeyCode::Down => Some(KeyInput::Down),
                KeyCode::Left => Some(KeyInput::Left),
                KeyCode::Right => Some(KeyInput::Right),
                KeyCode::PageUp => Some(KeyInput::PageUp),
                KeyCode::PageDown => Some(KeyInput::PageDown),
                KeyCode::Home => Some(KeyInput::Home),
                KeyCode::End => Some(KeyInput::End),
                KeyCode::Enter => Some(KeyInput::Enter),
                KeyCode::Char(c) if c.is_ascii_digit() => Some(KeyInput::Digit(c.to_digit(10).unwrap() as u8)),
                _ => None,
            };

            if let Some(key) = key_input {
                // Handle input with the shared event handler
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
                            state.needs_redraw = true;
                            return;
                        }
                        KeyInput::Digit(d) => {
                            let idx = d as usize;
                            if idx < state.current_choices.len() {
                                state.selected_choice_idx = idx;
                                state.update_choice_highlights();
                                state.select_current_choice();
                                state.needs_redraw = true;
                            }
                            return;
                        }
                        _ => {}
                    }
                }

                let WasmFancyTuiState {
                    ref game,
                    ref mut renderer,
                    ref current_choices,
                    ..
                } = *state;

                let view = GameStateView::new(game, renderer.player_id);
                let result = handle_key_event(&mut renderer.state, key, &view, current_choices.len());

                // Process event result
                match result {
                    EventResult::Handled => {
                        state.needs_redraw = true;
                    }
                    EventResult::NotHandled => {
                        if matches!(key, KeyInput::Space) {
                            state.run_until_choice();
                            state.needs_redraw = true;
                        }
                    }
                    EventResult::Pass | EventResult::Exit => {
                        let _ = js_sys::eval("window.showExitConfirmation && window.showExitConfirmation()");
                    }
                    EventResult::SelectChoice(idx) => {
                        if idx < state.current_choices.len() {
                            state.selected_choice_idx = idx;
                            state.update_choice_highlights();
                            state.select_current_choice();
                            state.needs_redraw = true;
                        }
                    }
                    EventResult::ShowBattlefield => {
                        let WasmFancyTuiState {
                            ref game, ref renderer, ..
                        } = *state;
                        let view = GameStateView::new(game, renderer.player_id);
                        let bf_text = crate::game::display::format_battlefield_for_log(&view);
                        log::info!("{}", bf_text);
                    }
                    EventResult::ShowHelp => {
                        let _ = js_sys::eval("window.showHelpDialog && window.showHelpDialog()");
                    }
                    _ => {}
                }
            }
        }
    });

    // Store state in global
    GLOBAL_TUI_STATE.with(|s| {
        *s.borrow_mut() = Some(state.clone());
    });

    // Set up render callback (simplified version for network)
    terminal.draw_web({
        let state = state.clone();
        move |f| {
            let mut state = state.borrow_mut();

            // For network AI mode, always run the game loop (even with pending_context)
            // since "pending" just means waiting for server, not waiting for human input.
            // The game loop handles waiting states correctly via NeedInput.
            // For network HUMAN mode, stop when we have a pending_context (like local mode)
            // to avoid spinning while waiting for the human to click a choice.
            let has_human_controller = state.p1_controller_type == WasmControllerType::Human
                || state.p2_controller_type == WasmControllerType::Human;
            let should_run =
                state.auto_run && !state.game_over && (!has_human_controller || state.pending_context.is_none());
            if should_run {
                state.run_until_choice();
                state.needs_redraw = true;
            }

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

            renderer.draw_ui(f, &view, prompt, &choices);
        }
    });

    log::info!("launch_network_game: Network TUI ready");
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

    // Copy token definitions from card database into game state
    if !card_db.tokens.is_empty() {
        game.token_definitions = card_db.tokens.clone();
        log::info!("Loaded {} token definitions into game", game.token_definitions.len());
    }

    // Shuffle libraries
    game.shuffle_library(p1_id);
    game.shuffle_library(p2_id);

    // Draw opening hands (7 cards each) BEFORE the turn 1 marker
    for _ in 0..7 {
        let _ = game.draw_card(p1_id);
        let _ = game.draw_card(p2_id);
    }

    // Mark the start of turn 1 AFTER drawing opening hands.
    // This is critical for the rewind/replay pattern: when the user makes their
    // first choice on turn 1, we call rewind_to_turn_start() which looks for a
    // ChangeTurn action. The rewind stops AT this marker, preserving everything
    // before it (including opening hands). Actions AFTER this marker (during turn 1
    // gameplay) get rewound.
    let prior_log_size = game.logger.log_count();
    game.undo_log.log(
        crate::undo::GameAction::ChangeTurn {
            from_player: p1_id, // Doesn't matter for turn 1
            to_player: p1_id,
            turn_number: 1,
            rng_state: None,
        },
        prior_log_size,
    );

    Ok(game)
}

// ═══════════════════════════════════════════════════════════════════════════
// WASM NETWORK GAME INITIALIZATION (Late-Binding Architecture)
// ═══════════════════════════════════════════════════════════════════════════

// Shared with ai_harness.rs - functions live in crate::wasm::network::game_init
#[cfg(feature = "wasm-network")]
use crate::wasm::network::game_init::{init_game_reserve_only_wasm, process_card_reveal_wasm};
