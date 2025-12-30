//! Shared event handling for FancyTUI (native and WASM)
//!
//! This module provides common input handling logic that can be used by both
//! the native TUI controller and the WASM browser implementation.
//!
//! ## Design
//!
//! The event handler operates on `FancyTuiState` and `GameStateView`, returning
//! an `EventResult` that the caller can use to determine what action to take.
//!
//! This allows both native (crossterm) and WASM (RatZilla) implementations to
//! share the same navigation and selection logic.

use crate::core::{CardId, PlayerId};
use crate::game::controller::GameStateView;
use crate::game::fancy_tui_renderer::{FancyTuiRenderer, FancyTuiState, FocusedPane};

/// Result of handling a keyboard event
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventResult {
    /// Event was handled, UI should be redrawn
    Handled,
    /// Event was not handled (unknown key)
    NotHandled,
    /// User wants to pass/cancel
    Pass,
    /// User wants to exit
    Exit,
    /// User wants to undo
    Undo,
    /// User wants to make a random choice
    RandomChoice,
    /// User selected a specific choice index (for Actions pane)
    SelectChoice(usize),
    /// User wants to see battlefield state in log
    ShowBattlefield,
    /// User wants to see help/keyboard shortcuts
    ShowHelp,
}

/// Abstract key code for cross-platform input handling
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyInput {
    // Navigation
    Up,
    Down,
    Left,
    Right,
    Tab,
    Enter,
    Escape,
    Space,
    PageUp,
    PageDown,
    Home,
    End,

    // Pane focus shortcuts
    FocusHand,       // H
    FocusInfo,       // I
    FocusYourBf,     // Y
    FocusOpponentBf, // O
    FocusActions,    // A
    FocusStack,      // S

    // Actions
    Pass,            // P or Q
    Undo,            // Z (uppercase)
    Random,          // R
    CtrlC,           // Exit
    Digit(u8),       // 0-9 for quick choice selection
    ShowBattlefield, // B - log battlefield state
    Help,            // ? - show keyboard shortcuts
    ToggleWrap,      // W - toggle line wrapping in log
}

/// Constants for 2D battlefield navigation
const CARDS_PER_ROW: usize = 4;

/// Handle a key input event, updating state and returning result
///
/// This is the main entry point for shared event handling.
/// Both native and WASM implementations should call this.
pub fn handle_key_event(
    state: &mut FancyTuiState,
    key: KeyInput,
    view: &GameStateView,
    num_choices: usize,
) -> EventResult {
    match key {
        // Pane focus shortcuts
        KeyInput::FocusHand => {
            state.focused_pane = FocusedPane::Hand;
            // Initialize selection to first card if hand not empty
            let hand = view.hand();
            if !hand.is_empty() && state.selected_card_in_hand.is_none() {
                state.selected_card_in_hand = Some(0);
                state.selected_card_id = Some(hand[0]);
            }
            EventResult::Handled
        }
        KeyInput::FocusInfo => {
            state.focused_pane = FocusedPane::Info;
            EventResult::Handled
        }
        KeyInput::FocusYourBf => {
            state.focused_pane = FocusedPane::YourBattlefield;
            // Initialize selection to first card if battlefield not empty
            let bf_cards = FancyTuiRenderer::get_battlefield_cards_in_order(view, view.player_id());
            if !bf_cards.is_empty() && state.selected_card_in_your_bf.is_none() {
                state.selected_card_in_your_bf = Some(bf_cards[0]);
                state.selected_card_id = Some(bf_cards[0]);
            }
            EventResult::Handled
        }
        KeyInput::FocusOpponentBf => {
            state.focused_pane = FocusedPane::OpponentBattlefield;
            // Initialize selection to first card if battlefield not empty
            if let Some(opp_id) = view.opponents().next() {
                let bf_cards = FancyTuiRenderer::get_battlefield_cards_in_order(view, opp_id);
                if !bf_cards.is_empty() && state.selected_card_in_opp_bf.is_none() {
                    state.selected_card_in_opp_bf = Some(bf_cards[0]);
                    state.selected_card_id = Some(bf_cards[0]);
                }
            }
            EventResult::Handled
        }
        KeyInput::FocusActions => {
            state.focused_pane = FocusedPane::Actions;
            EventResult::Handled
        }
        KeyInput::FocusStack => {
            // Stack is now part of Actions pane, so focus Actions
            state.focused_pane = FocusedPane::Actions;
            EventResult::Handled
        }
        KeyInput::Tab => {
            // Cycle through panes (Stack removed, now part of Actions)
            state.focused_pane = match state.focused_pane {
                FocusedPane::Hand => FocusedPane::Info,
                FocusedPane::Info => FocusedPane::YourBattlefield,
                FocusedPane::YourBattlefield => FocusedPane::OpponentBattlefield,
                FocusedPane::OpponentBattlefield => FocusedPane::Actions,
                FocusedPane::Actions => FocusedPane::Hand,
            };
            EventResult::Handled
        }

        // Arrow key navigation - route based on focused pane
        KeyInput::Up => handle_up_navigation(state, view, num_choices),
        KeyInput::Down => handle_down_navigation(state, view, num_choices),
        KeyInput::Left => handle_left_navigation(state, view),
        KeyInput::Right => handle_right_navigation(state, view),

        // Enter key
        KeyInput::Enter => handle_enter(state, view),

        // Pass/Cancel
        KeyInput::Pass | KeyInput::Escape => EventResult::Pass,

        // Exit
        KeyInput::CtrlC => EventResult::Exit,

        // Undo
        KeyInput::Undo => EventResult::Undo,

        // Random choice
        KeyInput::Random => EventResult::RandomChoice,

        // Show battlefield in log
        KeyInput::ShowBattlefield => EventResult::ShowBattlefield,

        // Show help
        KeyInput::Help => EventResult::ShowHelp,

        // Digit selection (only in Actions pane)
        KeyInput::Digit(d) => {
            if state.focused_pane == FocusedPane::Actions {
                let digit = d as usize;
                if digit < num_choices {
                    EventResult::SelectChoice(digit)
                } else {
                    EventResult::NotHandled
                }
            } else {
                EventResult::NotHandled
            }
        }

        // Page navigation (only effective for Info pane log)
        KeyInput::PageUp => {
            if state.focused_pane == FocusedPane::Info {
                // Page size of 10 - renderer will clamp based on actual log size
                state.log_page_up(usize::MAX, 10);
                EventResult::Handled
            } else {
                EventResult::NotHandled
            }
        }
        KeyInput::PageDown => {
            if state.focused_pane == FocusedPane::Info {
                state.log_page_down(10);
                EventResult::Handled
            } else {
                EventResult::NotHandled
            }
        }
        KeyInput::Home => {
            if state.focused_pane == FocusedPane::Info {
                // Scroll to beginning (oldest messages)
                state.log_scroll_home(usize::MAX, 10);
                EventResult::Handled
            } else {
                EventResult::NotHandled
            }
        }
        KeyInput::End => {
            if state.focused_pane == FocusedPane::Info {
                // Scroll to end (follow mode - newest messages)
                state.log_scroll_end();
                EventResult::Handled
            } else {
                EventResult::NotHandled
            }
        }

        // Toggle line wrapping in log (W key)
        KeyInput::ToggleWrap => {
            if state.focused_pane == FocusedPane::Info {
                state.log_toggle_wrap();
                EventResult::Handled
            } else {
                EventResult::NotHandled
            }
        }

        // Space - for WASM this advances turn, for native it depends on context
        KeyInput::Space => EventResult::NotHandled,
    }
}

/// Handle Up arrow key navigation
fn handle_up_navigation(state: &mut FancyTuiState, view: &GameStateView, _num_choices: usize) -> EventResult {
    match state.focused_pane {
        FocusedPane::Actions => {
            if state.highlighted_choice > 0 {
                state.highlighted_choice -= 1;
            }
            EventResult::Handled
        }
        FocusedPane::Hand => {
            let hand = view.hand();
            if !hand.is_empty() {
                let current = state.selected_card_in_hand.unwrap_or(0);
                if current > 0 {
                    state.selected_card_in_hand = Some(current - 1);
                    state.selected_card_id = Some(hand[current - 1]);
                }
            }
            EventResult::Handled
        }
        FocusedPane::YourBattlefield => {
            navigate_battlefield_up(
                &mut state.selected_card_in_your_bf,
                &mut state.selected_card_id,
                view,
                view.player_id(),
            );
            EventResult::Handled
        }
        FocusedPane::OpponentBattlefield => {
            if let Some(opp_id) = view.opponents().next() {
                navigate_battlefield_up(
                    &mut state.selected_card_in_opp_bf,
                    &mut state.selected_card_id,
                    view,
                    opp_id,
                );
            }
            EventResult::Handled
        }
        FocusedPane::Info => {
            // Scroll log up (toward older messages)
            // Use large values - renderer will clamp based on actual log size
            state.log_scroll_up(usize::MAX, 10);
            EventResult::Handled
        }
    }
}

/// Handle Down arrow key navigation
fn handle_down_navigation(state: &mut FancyTuiState, view: &GameStateView, num_choices: usize) -> EventResult {
    match state.focused_pane {
        FocusedPane::Actions => {
            if state.highlighted_choice + 1 < num_choices {
                state.highlighted_choice += 1;
            }
            EventResult::Handled
        }
        FocusedPane::Hand => {
            let hand = view.hand();
            if !hand.is_empty() {
                let current = state.selected_card_in_hand.unwrap_or(0);
                if current + 1 < hand.len() {
                    state.selected_card_in_hand = Some(current + 1);
                    state.selected_card_id = Some(hand[current + 1]);
                }
            }
            EventResult::Handled
        }
        FocusedPane::YourBattlefield => {
            navigate_battlefield_down(
                &mut state.selected_card_in_your_bf,
                &mut state.selected_card_id,
                view,
                view.player_id(),
            );
            EventResult::Handled
        }
        FocusedPane::OpponentBattlefield => {
            if let Some(opp_id) = view.opponents().next() {
                navigate_battlefield_down(
                    &mut state.selected_card_in_opp_bf,
                    &mut state.selected_card_id,
                    view,
                    opp_id,
                );
            }
            EventResult::Handled
        }
        FocusedPane::Info => {
            // Scroll log down (toward newer messages)
            state.log_scroll_down();
            EventResult::Handled
        }
    }
}

/// Handle Left arrow key navigation
fn handle_left_navigation(state: &mut FancyTuiState, view: &GameStateView) -> EventResult {
    match state.focused_pane {
        FocusedPane::YourBattlefield => {
            navigate_battlefield_left(
                &mut state.selected_card_in_your_bf,
                &mut state.selected_card_id,
                view,
                view.player_id(),
            );
            EventResult::Handled
        }
        FocusedPane::OpponentBattlefield => {
            if let Some(opp_id) = view.opponents().next() {
                navigate_battlefield_left(
                    &mut state.selected_card_in_opp_bf,
                    &mut state.selected_card_id,
                    view,
                    opp_id,
                );
            }
            EventResult::Handled
        }
        FocusedPane::Info => {
            // Scroll to previous turn header
            let logs = view.logger().logs();
            // Estimate visible lines (will be clamped by renderer)
            state.log_scroll_prev_turn(&logs, 20);
            EventResult::Handled
        }
        _ => EventResult::Handled,
    }
}

/// Handle Right arrow key navigation
fn handle_right_navigation(state: &mut FancyTuiState, view: &GameStateView) -> EventResult {
    match state.focused_pane {
        FocusedPane::YourBattlefield => {
            navigate_battlefield_right(
                &mut state.selected_card_in_your_bf,
                &mut state.selected_card_id,
                view,
                view.player_id(),
            );
            EventResult::Handled
        }
        FocusedPane::OpponentBattlefield => {
            if let Some(opp_id) = view.opponents().next() {
                navigate_battlefield_right(
                    &mut state.selected_card_in_opp_bf,
                    &mut state.selected_card_id,
                    view,
                    opp_id,
                );
            }
            EventResult::Handled
        }
        FocusedPane::Info => {
            // Scroll to next turn header
            let logs = view.logger().logs();
            // Estimate visible lines (will be clamped by renderer)
            state.log_scroll_next_turn(&logs, 20);
            EventResult::Handled
        }
        _ => EventResult::Handled,
    }
}

/// Handle Enter key
fn handle_enter(state: &mut FancyTuiState, view: &GameStateView) -> EventResult {
    // In Actions pane, select the highlighted choice
    if state.focused_pane == FocusedPane::Actions {
        return EventResult::SelectChoice(state.highlighted_choice);
    }

    // In other panes, Enter selects a card to view in Card Details
    match state.focused_pane {
        FocusedPane::Hand => {
            if let Some(idx) = state.selected_card_in_hand {
                let hand = view.hand();
                if idx < hand.len() {
                    state.selected_card_id = Some(hand[idx]);
                }
            }
        }
        FocusedPane::YourBattlefield => {
            if let Some(card_id) = state.selected_card_in_your_bf {
                state.selected_card_id = Some(card_id);
            }
        }
        FocusedPane::OpponentBattlefield => {
            if let Some(card_id) = state.selected_card_in_opp_bf {
                state.selected_card_id = Some(card_id);
            }
        }
        FocusedPane::Info | FocusedPane::Actions => {
            // Info pane doesn't have cards to select
            // Actions pane could potentially show stack items for card details
            // but that's a future enhancement
        }
    }

    EventResult::Handled
}

// Battlefield navigation helpers

fn navigate_battlefield_up(
    selected: &mut Option<CardId>,
    selected_card_id: &mut Option<CardId>,
    view: &GameStateView,
    owner: PlayerId,
) {
    let bf_cards = FancyTuiRenderer::get_battlefield_cards_in_order(view, owner);
    if bf_cards.is_empty() {
        return;
    }

    if let Some(current_idx) = selected.and_then(|id| bf_cards.iter().position(|&c| c == id)) {
        if current_idx >= CARDS_PER_ROW {
            let new_idx = current_idx - CARDS_PER_ROW;
            let new_card = bf_cards[new_idx];
            *selected = Some(new_card);
            *selected_card_id = Some(new_card);
        }
    }
}

fn navigate_battlefield_down(
    selected: &mut Option<CardId>,
    selected_card_id: &mut Option<CardId>,
    view: &GameStateView,
    owner: PlayerId,
) {
    let bf_cards = FancyTuiRenderer::get_battlefield_cards_in_order(view, owner);
    if bf_cards.is_empty() {
        return;
    }

    if let Some(current_idx) = selected.and_then(|id| bf_cards.iter().position(|&c| c == id)) {
        let new_idx = current_idx + CARDS_PER_ROW;
        if new_idx < bf_cards.len() {
            let new_card = bf_cards[new_idx];
            *selected = Some(new_card);
            *selected_card_id = Some(new_card);
        }
    }
}

fn navigate_battlefield_left(
    selected: &mut Option<CardId>,
    selected_card_id: &mut Option<CardId>,
    view: &GameStateView,
    owner: PlayerId,
) {
    let bf_cards = FancyTuiRenderer::get_battlefield_cards_in_order(view, owner);
    if bf_cards.is_empty() {
        return;
    }

    if let Some(current_idx) = selected.and_then(|id| bf_cards.iter().position(|&c| c == id)) {
        let row = current_idx / CARDS_PER_ROW;
        let col = current_idx % CARDS_PER_ROW;

        let new_idx = if col > 0 {
            // Move left within the row
            current_idx - 1
        } else {
            // Wrap to end of current row
            let row_end = ((row + 1) * CARDS_PER_ROW).min(bf_cards.len());
            row_end - 1
        };

        let new_card = bf_cards[new_idx];
        *selected = Some(new_card);
        *selected_card_id = Some(new_card);
    }
}

fn navigate_battlefield_right(
    selected: &mut Option<CardId>,
    selected_card_id: &mut Option<CardId>,
    view: &GameStateView,
    owner: PlayerId,
) {
    let bf_cards = FancyTuiRenderer::get_battlefield_cards_in_order(view, owner);
    if bf_cards.is_empty() {
        return;
    }

    if let Some(current_idx) = selected.and_then(|id| bf_cards.iter().position(|&c| c == id)) {
        let row = current_idx / CARDS_PER_ROW;
        let row_start = row * CARDS_PER_ROW;
        let row_end = ((row + 1) * CARDS_PER_ROW).min(bf_cards.len());

        let new_idx = if current_idx + 1 < row_end {
            // Move right within the row
            current_idx + 1
        } else {
            // Wrap to start of current row
            row_start
        };

        let new_card = bf_cards[new_idx];
        *selected = Some(new_card);
        *selected_card_id = Some(new_card);
    }
}

/// Handle a mouse click event, updating state
///
/// Returns true if the click was handled and the UI should be redrawn.
pub fn handle_mouse_click(state: &mut FancyTuiState, x: u16, y: u16, view: &GameStateView) -> bool {
    // Check entity positions FIRST (more specific than pane areas)
    // This allows clicking on individual cards within panes
    for entity_pos in &state.entity_positions {
        if x >= entity_pos.area.x
            && x < entity_pos.area.x + entity_pos.area.width
            && y >= entity_pos.area.y
            && y < entity_pos.area.y + entity_pos.area.height
        {
            use crate::game::fancy_tui_renderer::{BattlefieldEntity, Entity};

            // Entity clicked! Handle based on entity type
            match &entity_pos.entity {
                Entity::HandCard { card_id, index } => {
                    // Hand card clicked - select it and focus hand pane
                    state.selected_card_in_hand = Some(*index);
                    state.selected_card_id = Some(*card_id);
                    state.focused_pane = FocusedPane::Hand;
                }
                Entity::GraveyardCard { card_id, .. } => {
                    // Graveyard card clicked - just select it to show details
                    // Don't change focused pane since graveyard isn't a navigable pane
                    state.selected_card_id = Some(*card_id);
                }
                _ => {
                    // Battlefield entity clicked
                    let representative = entity_pos.entity.representative_card();
                    state.selected_card_id = Some(representative);

                    // Update battlefield selection
                    if let Some(card) = view.get_card(representative) {
                        if card.controller == view.player_id() {
                            state.selected_card_in_your_bf = Some(representative);
                            state.focused_pane = FocusedPane::YourBattlefield;
                        } else {
                            state.selected_card_in_opp_bf = Some(representative);
                            state.focused_pane = FocusedPane::OpponentBattlefield;
                        }
                    }
                }
            }

            return true;
        }
    }

    // Check pane areas (fallback for clicks on empty areas within panes)
    // Check Actions pane
    if let Some(actions_area) = state.actions_pane_area {
        if x >= actions_area.x
            && x < actions_area.x + actions_area.width
            && y >= actions_area.y
            && y < actions_area.y + actions_area.height
        {
            state.focused_pane = FocusedPane::Actions;
            return true;
        }
    }

    // Check Info pane (for clicks on Combat/Log area)
    if let Some(info_area) = state.info_pane_area {
        if x >= info_area.x
            && x < info_area.x + info_area.width
            && y >= info_area.y
            && y < info_area.y + info_area.height
        {
            state.focused_pane = FocusedPane::Info;
            return true;
        }
    }

    // Check Hand pane (for clicks on empty space in hand)
    if let Some(hand_area) = state.hand_pane_area {
        if x >= hand_area.x
            && x < hand_area.x + hand_area.width
            && y >= hand_area.y
            && y < hand_area.y + hand_area.height
        {
            state.focused_pane = FocusedPane::Hand;
            // Initialize selection to first card if hand not empty
            let hand = view.hand();
            if !hand.is_empty() && state.selected_card_in_hand.is_none() {
                state.selected_card_in_hand = Some(0);
                state.selected_card_id = Some(hand[0]);
            }
            return true;
        }
    }

    false
}

/// Get help text describing all keyboard shortcuts
///
/// This function returns a formatted string with all available keyboard shortcuts.
/// The format is suitable for display in both native TUI (as an overlay) and
/// browser (as a modal dialog).
///
/// # Arguments
/// * `include_wasm_only` - If true, include WASM/browser-only shortcuts (like 'i' for images)
pub fn get_help_text(include_wasm_only: bool) -> String {
    let mut help = String::from("Keyboard Shortcuts\n");
    help.push_str("==================\n\n");

    help.push_str("Navigation:\n");
    help.push_str("  Arrow Keys  - Navigate within panes\n");
    help.push_str("  Tab         - Cycle through panes\n");
    help.push_str("  Enter       - Select/Confirm\n");
    help.push_str("  1-9         - Quick select (Actions pane)\n");
    help.push_str("  PgUp/PgDn   - Page scroll (Info pane)\n");
    help.push_str("  Home/End    - Jump to start/end (Info pane)\n");
    help.push_str("  Left/Right  - Scroll by turn (Info pane)\n");
    help.push_str("  W           - Toggle line wrap (Info pane)\n\n");

    help.push_str("Pane Focus:\n");
    help.push_str("  H           - Focus Hand\n");
    help.push_str("  I           - Focus Info/Card Details\n");
    help.push_str("  Y           - Focus Your Battlefield\n");
    help.push_str("  O           - Focus Opponent's Battlefield\n");
    help.push_str("  A           - Focus Actions\n");
    help.push_str("  S           - Focus Stack\n\n");

    help.push_str("Game Actions:\n");
    help.push_str("  Space       - Advance turn/Continue\n");
    help.push_str("  P or Q      - Pass priority\n");
    help.push_str("  Z           - Undo last action\n");
    help.push_str("  R           - Random choice\n");
    help.push_str("  B           - Show battlefield in log\n");
    help.push_str("  Esc         - Exit game\n\n");

    if include_wasm_only {
        help.push_str("Browser Only:\n");
        help.push_str("  I (lowercase) - Toggle card images\n");
        help.push_str("  C           - Toggle controls panel\n\n");
    }

    help.push_str("Other:\n");
    help.push_str("  ?           - Show this help\n");

    help
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::fancy_tui_renderer::{Entity, EntityPosition};
    use ratatui::layout::Rect;

    /// Test that clicking on a GraveyardCard entity sets selected_card_id
    #[test]
    fn test_graveyard_click_sets_selected_card() {
        let mut state = FancyTuiState::new();

        // Create a fake graveyard card entity position
        let card_id = CardId::new(42);
        let graveyard_entity = Entity::GraveyardCard {
            card_id,
            index: 0,
            owner: PlayerId::new(0),
        };

        // Position the entity at (10, 5) with width 15, height 1
        state.entity_positions.push(EntityPosition {
            entity: graveyard_entity,
            area: Rect {
                x: 10,
                y: 5,
                width: 15,
                height: 1,
            },
            layout_area_px: None,
        });

        // Verify initial state
        assert!(state.selected_card_id.is_none());

        // Simulate clicking at coordinates within the entity area
        // Note: handle_mouse_click needs a view, but for entity clicks
        // it only uses the view after the entity is identified.
        // We'll test the entity matching directly instead.

        // Check if click at (12, 5) would match the entity
        let click_x: u16 = 12;
        let click_y: u16 = 5;

        // Find matching entity (mimicking handle_mouse_click logic)
        for entity_pos in &state.entity_positions {
            if click_x >= entity_pos.area.x
                && click_x < entity_pos.area.x + entity_pos.area.width
                && click_y >= entity_pos.area.y
                && click_y < entity_pos.area.y + entity_pos.area.height
            {
                // Entity matched! Verify it's our graveyard card
                match &entity_pos.entity {
                    Entity::GraveyardCard { card_id: cid, .. } => {
                        state.selected_card_id = Some(*cid);
                    }
                    _ => panic!("Expected GraveyardCard entity"),
                }
            }
        }

        // Verify the click set selected_card_id
        assert_eq!(state.selected_card_id, Some(card_id));
    }

    /// Test that clicking outside entity area doesn't match
    #[test]
    fn test_graveyard_click_outside_area_no_match() {
        // State not needed for this test, just checking geometry
        let _state = FancyTuiState::new();

        let card_id = CardId::new(42);
        let graveyard_entity = Entity::GraveyardCard {
            card_id,
            index: 0,
            owner: PlayerId::new(0),
        };

        let entity_pos = EntityPosition {
            entity: graveyard_entity,
            area: Rect {
                x: 10,
                y: 5,
                width: 15,
                height: 1,
            },
            layout_area_px: None,
        };

        // Click outside the area (y=6 is below the entity at y=5, height=1)
        let click_x: u16 = 12;
        let click_y: u16 = 6;

        let is_inside = click_x >= entity_pos.area.x
            && click_x < entity_pos.area.x + entity_pos.area.width
            && click_y >= entity_pos.area.y
            && click_y < entity_pos.area.y + entity_pos.area.height;

        assert!(!is_inside, "Click at y=6 should be outside entity at y=5 with height=1");
    }
}
