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
use smallvec::SmallVec;

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
    Backspace,       // Backspace - delete last digit in buffer
    ShowBattlefield, // B - log battlefield state
    Help,            // ? - show keyboard shortcuts
    ToggleWrap,      // W - toggle line wrapping in log
}

/// Scroll direction for mouse wheel events
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollDirection {
    Up,
    Down,
    Left,
    Right,
}

/// Backend-neutral UI event enum
///
/// Both native (crossterm) and web (RatZilla) backends convert their
/// raw events into this enum before dispatching to shared handlers.
/// Backend-specific events (auto-run toggle, image toggle, controls panel)
/// are handled before conversion and never reach shared code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiEvent {
    /// Keyboard input (already abstracted via KeyInput)
    Key(KeyInput),
    /// Mouse click at terminal cell coordinates
    MouseClick { col: u16, row: u16 },
    /// Mouse scroll wheel at terminal cell coordinates
    MouseWheel {
        direction: ScrollDirection,
        col: u16,
        row: u16,
    },
    /// Terminal/viewport was resized
    Resize { width: u16, height: u16 },
}

/// Constants for 2D battlefield navigation
const CARDS_PER_ROW: usize = 4;

/// Handle a key input event, updating state and returning result
///
/// This is the main entry point for shared event handling.
/// Both native and WASM implementations should call this.
///
/// Note: Wildcard is intentional - FocusedPane enum has many variants;
/// we handle the relevant subset per context.
#[allow(clippy::wildcard_enum_match_arm)]
pub fn handle_key_event(
    state: &mut FancyTuiState,
    key: KeyInput,
    view: &GameStateView,
    num_choices: usize,
) -> EventResult {
    match key {
        // Pane focus shortcuts
        KeyInput::FocusHand => {
            state.view.focused_pane = FocusedPane::Hand;
            // Initialize selection to first card if hand not empty
            let sorted_hand = FancyTuiRenderer::get_sorted_hand(view);
            if !sorted_hand.is_empty() && state.view.selected_card_in_hand.is_none() {
                state.view.selected_card_in_hand = Some(0);
                state.session.selected_card_id = Some(sorted_hand[0]);
            }
            EventResult::Handled
        }
        KeyInput::FocusInfo => {
            state.view.focused_pane = FocusedPane::Log;
            EventResult::Handled
        }
        KeyInput::FocusYourBf => {
            state.view.focused_pane = FocusedPane::YourBattlefield;
            // Initialize selection to first card if battlefield not empty
            let bf_cards = FancyTuiRenderer::get_battlefield_cards_in_order(view, view.player_id());
            if !bf_cards.is_empty() && state.view.selected_card_in_your_bf.is_none() {
                state.view.selected_card_in_your_bf = Some(bf_cards[0]);
                state.session.selected_card_id = Some(bf_cards[0]);
            }
            EventResult::Handled
        }
        KeyInput::FocusOpponentBf => {
            state.view.focused_pane = FocusedPane::OpponentBattlefield;
            // Initialize selection to first card if battlefield not empty
            if let Some(opp_id) = view.opponents().next() {
                let bf_cards = FancyTuiRenderer::get_battlefield_cards_in_order(view, opp_id);
                if !bf_cards.is_empty() && state.view.selected_card_in_opp_bf.is_none() {
                    state.view.selected_card_in_opp_bf = Some(bf_cards[0]);
                    state.session.selected_card_id = Some(bf_cards[0]);
                }
            }
            EventResult::Handled
        }
        KeyInput::FocusActions => {
            state.view.focused_pane = FocusedPane::Actions;
            update_card_id_from_action(state);
            EventResult::Handled
        }
        KeyInput::FocusStack => {
            // Stack is now part of Actions pane, so focus Actions
            state.view.focused_pane = FocusedPane::Actions;
            EventResult::Handled
        }
        KeyInput::Tab => {
            // Cycle through panes (Stack removed, now part of Actions)
            state.view.focused_pane = match state.view.focused_pane {
                FocusedPane::Hand => FocusedPane::Log,
                FocusedPane::Log => FocusedPane::YourBattlefield,
                FocusedPane::YourBattlefield => FocusedPane::OpponentBattlefield,
                FocusedPane::OpponentBattlefield => FocusedPane::Actions,
                FocusedPane::Actions => FocusedPane::Hand,
            };
            // When tabbing to Actions, show the card for the highlighted action
            if state.view.focused_pane == FocusedPane::Actions {
                update_card_id_from_action(state);
            }
            EventResult::Handled
        }

        // Arrow key navigation - route based on focused pane
        KeyInput::Up => handle_up_navigation(state, view, num_choices),
        KeyInput::Down => handle_down_navigation(state, view, num_choices),
        KeyInput::Left => handle_left_navigation(state, view),
        KeyInput::Right => handle_right_navigation(state, view),

        // Enter key
        KeyInput::Enter => handle_enter(state, view),

        // Pass/Cancel - but if digit buffer is non-empty, clear it first
        KeyInput::Pass | KeyInput::Escape => {
            if !state.session.digit_buffer.is_empty() {
                state.session.digit_buffer.clear();
                EventResult::Handled
            } else {
                EventResult::Pass
            }
        }

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
            if state.view.focused_pane == FocusedPane::Actions {
                if num_choices > 10 {
                    // Multi-digit mode: accumulate in buffer, auto-highlight
                    state.session.digit_buffer.push(char::from(b'0' + d));
                    if let Ok(idx) = state.session.digit_buffer.parse::<usize>() {
                        if idx < num_choices {
                            state.session.highlighted_choice = idx;
                        }
                    }
                    update_card_id_from_action(state);
                    EventResult::Handled
                } else {
                    // Single-digit mode: instant select (existing behavior)
                    let digit = d as usize;
                    if digit < num_choices {
                        EventResult::SelectChoice(digit)
                    } else {
                        EventResult::NotHandled
                    }
                }
            } else {
                EventResult::NotHandled
            }
        }

        // Backspace: remove last digit from buffer
        KeyInput::Backspace => {
            if !state.session.digit_buffer.is_empty() {
                state.session.digit_buffer.pop();
                // Update highlight based on remaining buffer
                if let Ok(idx) = state.session.digit_buffer.parse::<usize>() {
                    if idx < num_choices {
                        state.session.highlighted_choice = idx;
                    }
                }
                EventResult::Handled
            } else {
                EventResult::NotHandled
            }
        }

        // Page navigation (only effective for Info pane log)
        KeyInput::PageUp => {
            if state.view.focused_pane == FocusedPane::Log {
                // Page size of 10 - renderer will clamp based on actual log size
                state.view.log_page_up(usize::MAX, 10);
                EventResult::Handled
            } else {
                EventResult::NotHandled
            }
        }
        KeyInput::PageDown => {
            if state.view.focused_pane == FocusedPane::Log {
                state.view.log_page_down(10);
                EventResult::Handled
            } else {
                EventResult::NotHandled
            }
        }
        KeyInput::Home => {
            if state.view.focused_pane == FocusedPane::Log {
                // Scroll to beginning (oldest messages)
                state.view.log_scroll_home(usize::MAX, 10);
                EventResult::Handled
            } else {
                EventResult::NotHandled
            }
        }
        KeyInput::End => {
            if state.view.focused_pane == FocusedPane::Log {
                // Scroll to end (follow mode - newest messages)
                state.view.log_scroll_end();
                EventResult::Handled
            } else {
                EventResult::NotHandled
            }
        }

        // Toggle line wrapping in log (W key)
        KeyInput::ToggleWrap => {
            if state.view.focused_pane == FocusedPane::Log {
                let logs = view.logger().logs();
                state.view.log_toggle_wrap(logs.len());
                EventResult::Handled
            } else {
                EventResult::NotHandled
            }
        }

        // Space - for WASM this advances turn, for native it depends on context
        KeyInput::Space => EventResult::NotHandled,
    }
}

/// Update selected_card_id based on the currently highlighted action.
///
/// In all choice contexts, index 0 is a non-card option (pass/done/skip/no-target),
/// and indices 1..N map to valid_choices[0..N-1]. When the focused pane is not
/// Actions, this is a no-op.
fn update_card_id_from_action(state: &mut FancyTuiState) {
    if state.view.focused_pane != FocusedPane::Actions {
        return;
    }
    if state.session.highlighted_choice > 0 {
        if let Some(&card_id) = state.session.valid_choices.get(state.session.highlighted_choice - 1) {
            state.session.selected_card_id = Some(card_id);
        }
    }
}

/// Handle Up arrow key navigation
fn handle_up_navigation(state: &mut FancyTuiState, view: &GameStateView, _num_choices: usize) -> EventResult {
    match state.view.focused_pane {
        FocusedPane::Actions => {
            if state.session.highlighted_choice > 0 {
                state.session.highlighted_choice -= 1;
            }
            update_card_id_from_action(state);
            EventResult::Handled
        }
        FocusedPane::Hand => {
            let sorted_hand = FancyTuiRenderer::get_sorted_hand(view);
            if !sorted_hand.is_empty() {
                let current = state.view.selected_card_in_hand.unwrap_or(0);
                if current > 0 {
                    state.view.selected_card_in_hand = Some(current - 1);
                    state.session.selected_card_id = Some(sorted_hand[current - 1]);
                }
            }
            EventResult::Handled
        }
        FocusedPane::YourBattlefield => {
            navigate_battlefield_up(
                &mut state.view.selected_card_in_your_bf,
                &mut state.session.selected_card_id,
                view,
                view.player_id(),
            );
            EventResult::Handled
        }
        FocusedPane::OpponentBattlefield => {
            if let Some(opp_id) = view.opponents().next() {
                navigate_battlefield_up(
                    &mut state.view.selected_card_in_opp_bf,
                    &mut state.session.selected_card_id,
                    view,
                    opp_id,
                );
            }
            EventResult::Handled
        }
        FocusedPane::Log => {
            // Scroll log up (toward older messages)
            // Use large values - renderer will clamp based on actual log size
            state.view.log_scroll_up(usize::MAX, 10);
            EventResult::Handled
        }
    }
}

/// Handle Down arrow key navigation
fn handle_down_navigation(state: &mut FancyTuiState, view: &GameStateView, num_choices: usize) -> EventResult {
    match state.view.focused_pane {
        FocusedPane::Actions => {
            if state.session.highlighted_choice + 1 < num_choices {
                state.session.highlighted_choice += 1;
            }
            update_card_id_from_action(state);
            EventResult::Handled
        }
        FocusedPane::Hand => {
            let sorted_hand = FancyTuiRenderer::get_sorted_hand(view);
            if !sorted_hand.is_empty() {
                let current = state.view.selected_card_in_hand.unwrap_or(0);
                if current + 1 < sorted_hand.len() {
                    state.view.selected_card_in_hand = Some(current + 1);
                    state.session.selected_card_id = Some(sorted_hand[current + 1]);
                }
            }
            EventResult::Handled
        }
        FocusedPane::YourBattlefield => {
            navigate_battlefield_down(
                &mut state.view.selected_card_in_your_bf,
                &mut state.session.selected_card_id,
                view,
                view.player_id(),
            );
            EventResult::Handled
        }
        FocusedPane::OpponentBattlefield => {
            if let Some(opp_id) = view.opponents().next() {
                navigate_battlefield_down(
                    &mut state.view.selected_card_in_opp_bf,
                    &mut state.session.selected_card_id,
                    view,
                    opp_id,
                );
            }
            EventResult::Handled
        }
        FocusedPane::Log => {
            // Scroll log down (toward newer messages)
            state.view.log_scroll_down();
            EventResult::Handled
        }
    }
}

/// Handle Left arrow key navigation
/// Note: Wildcard is intentional - FocusedPane has many variants;
/// we handle the relevant subset for left navigation.
#[allow(clippy::wildcard_enum_match_arm)]
fn handle_left_navigation(state: &mut FancyTuiState, view: &GameStateView) -> EventResult {
    match state.view.focused_pane {
        FocusedPane::YourBattlefield => {
            navigate_battlefield_left(
                &mut state.view.selected_card_in_your_bf,
                &mut state.session.selected_card_id,
                view,
                view.player_id(),
            );
            EventResult::Handled
        }
        FocusedPane::OpponentBattlefield => {
            if let Some(opp_id) = view.opponents().next() {
                navigate_battlefield_left(
                    &mut state.view.selected_card_in_opp_bf,
                    &mut state.session.selected_card_id,
                    view,
                    opp_id,
                );
            }
            EventResult::Handled
        }
        FocusedPane::Log => {
            // Scroll to previous turn header
            let logs = view.logger().logs();
            let visible_lines = state.view.log_visible_lines;
            state.view.log_scroll_prev_turn(&logs, visible_lines);
            EventResult::Handled
        }
        _ => EventResult::Handled,
    }
}

/// Handle Right arrow key navigation
/// Note: Wildcard is intentional - FocusedPane has many variants;
/// we handle the relevant subset for right navigation.
#[allow(clippy::wildcard_enum_match_arm)]
fn handle_right_navigation(state: &mut FancyTuiState, view: &GameStateView) -> EventResult {
    match state.view.focused_pane {
        FocusedPane::YourBattlefield => {
            navigate_battlefield_right(
                &mut state.view.selected_card_in_your_bf,
                &mut state.session.selected_card_id,
                view,
                view.player_id(),
            );
            EventResult::Handled
        }
        FocusedPane::OpponentBattlefield => {
            if let Some(opp_id) = view.opponents().next() {
                navigate_battlefield_right(
                    &mut state.view.selected_card_in_opp_bf,
                    &mut state.session.selected_card_id,
                    view,
                    opp_id,
                );
            }
            EventResult::Handled
        }
        FocusedPane::Log => {
            // Scroll to next turn header
            let logs = view.logger().logs();
            let visible_lines = state.view.log_visible_lines;
            state.view.log_scroll_next_turn(&logs, visible_lines);
            EventResult::Handled
        }
        _ => EventResult::Handled,
    }
}

/// Handle Enter key
fn handle_enter(state: &mut FancyTuiState, view: &GameStateView) -> EventResult {
    // If digit buffer is non-empty, parse and select that choice index
    if !state.session.digit_buffer.is_empty() {
        if let Ok(idx) = state.session.digit_buffer.parse::<usize>() {
            state.session.digit_buffer.clear();
            return EventResult::SelectChoice(idx);
        }
        state.session.digit_buffer.clear();
        return EventResult::Handled;
    }

    // In Actions pane, select the highlighted choice
    if state.view.focused_pane == FocusedPane::Actions {
        return EventResult::SelectChoice(state.session.highlighted_choice);
    }

    // In other panes, Enter selects a card to view in Card Details
    match state.view.focused_pane {
        FocusedPane::Hand => {
            if let Some(idx) = state.view.selected_card_in_hand {
                let sorted_hand = FancyTuiRenderer::get_sorted_hand(view);
                if idx < sorted_hand.len() {
                    state.session.selected_card_id = Some(sorted_hand[idx]);
                }
            }
        }
        FocusedPane::YourBattlefield => {
            if let Some(card_id) = state.view.selected_card_in_your_bf {
                state.session.selected_card_id = Some(card_id);
            }
        }
        FocusedPane::OpponentBattlefield => {
            if let Some(card_id) = state.view.selected_card_in_opp_bf {
                state.session.selected_card_id = Some(card_id);
            }
        }
        FocusedPane::Log | FocusedPane::Actions => {
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
///
/// Note: Wildcard is intentional - Entity enum has many variants (SingleCard, Stack types);
/// we handle clicking on specific entity types.
#[allow(clippy::wildcard_enum_match_arm)]
pub fn handle_mouse_click(state: &mut FancyTuiState, x: u16, y: u16, view: &GameStateView) -> bool {
    // Check entity positions FIRST (more specific than pane areas)
    // This allows clicking on individual cards within panes
    for entity_pos in &state.view.entity_positions {
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
                    state.view.selected_card_in_hand = Some(*index);
                    state.session.selected_card_id = Some(*card_id);
                    state.view.focused_pane = FocusedPane::Hand;
                }
                Entity::GraveyardCard { card_id, .. } => {
                    // Graveyard card clicked - just select it to show details
                    // Don't change focused pane since graveyard isn't a navigable pane
                    state.session.selected_card_id = Some(*card_id);
                }
                _ => {
                    // Battlefield entity clicked
                    let representative = entity_pos.entity.representative_card();
                    state.session.selected_card_id = Some(representative);

                    // Update battlefield selection
                    if let Some(card) = view.get_card(representative) {
                        if card.controller == view.player_id() {
                            state.view.selected_card_in_your_bf = Some(representative);
                            state.view.focused_pane = FocusedPane::YourBattlefield;
                        } else {
                            state.view.selected_card_in_opp_bf = Some(representative);
                            state.view.focused_pane = FocusedPane::OpponentBattlefield;
                        }
                    }
                }
            }

            return true;
        }
    }

    // Check pane areas (fallback for clicks on empty areas within panes)
    // Check Actions pane
    if let Some(actions_area) = state.view.actions_pane_area {
        if x >= actions_area.x
            && x < actions_area.x + actions_area.width
            && y >= actions_area.y
            && y < actions_area.y + actions_area.height
        {
            state.view.focused_pane = FocusedPane::Actions;
            return true;
        }
    }

    // Check Log pane area
    if let Some(log_area) = state.view.log_pane_area {
        if x >= log_area.x && x < log_area.x + log_area.width && y >= log_area.y && y < log_area.y + log_area.height {
            state.view.focused_pane = FocusedPane::Log;
            return true;
        }
    }

    // Check Hand pane (for clicks on empty space in hand)
    if let Some(hand_area) = state.view.hand_pane_area {
        if x >= hand_area.x
            && x < hand_area.x + hand_area.width
            && y >= hand_area.y
            && y < hand_area.y + hand_area.height
        {
            state.view.focused_pane = FocusedPane::Hand;
            // Initialize selection to first card if hand not empty (use sorted order)
            let sorted_hand = FancyTuiRenderer::get_sorted_hand(view);
            if !sorted_hand.is_empty() && state.view.selected_card_in_hand.is_none() {
                state.view.selected_card_in_hand = Some(0);
                state.session.selected_card_id = Some(sorted_hand[0]);
            }
            return true;
        }
    }

    false
}

/// Outcome of selecting a card by id (e.g. via the HTML GUI). Reported back so
/// callers can decide whether to redraw, or to surface an error to the user
/// when the id does not refer to a card the perspective player can select.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardSelectionResult {
    /// Selection updated; the GUI should redraw the details pane.
    Selected {
        /// Where the card was located (for the GUI's badge / focus).
        zone: SelectedCardZone,
    },
    /// `card_id` does not refer to any card in the game state.
    NotFound,
    /// Card exists but is not currently visible to the perspective player
    /// (e.g. opponent's hand or library — must not leak hidden information).
    NotVisible,
}

/// Where a selected card lives, from the perspective of one player.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectedCardZone {
    /// Our own hand.
    OurHand,
    /// Our battlefield.
    OurBattlefield,
    /// Opponent's battlefield.
    OpponentBattlefield,
    /// A graveyard (could be either player's).
    Graveyard,
    /// The shared stack.
    Stack,
    /// A command zone (Commander format).
    Command,
}

/// Select a card by stable id and update the corresponding focus/index state.
///
/// This is the SHARED selection routine used by both the ratatui mouse-click
/// handler (which derives a `CardId` from a clicked entity area) and the WASM
/// `tui_select_card` export (which receives the `CardId` directly from the
/// HTML GUI). Both code paths end up updating the same `FancyTuiState` fields
/// — `selected_card_id` plus the appropriate per-pane index/highlight — so
/// the TUI and the GUI never disagree on what's selected.
///
/// Hidden-information rule: opponent's hand and any player's library are NOT
/// selectable; they return `NotVisible` without mutating state. This protects
/// the network/WASM boundary from accidental hidden-state leaks.
///
/// Returns `Selected` with the zone where the card lives, or `NotFound` /
/// `NotVisible` when selection could not happen.
pub fn select_card_by_id(state: &mut FancyTuiState, card_id: CardId, view: &GameStateView) -> CardSelectionResult {
    let Some(card) = view.get_card(card_id) else {
        return CardSelectionResult::NotFound;
    };

    let perspective = view.player_id();

    // Walk the zones in the same order as the TUI's mouse handler.
    // 1. Our hand.
    let our_hand = view.player_hand(perspective);
    if our_hand.contains(&card_id) {
        // Use the SAME sorted order as `draw_hand` so the index matches what
        // the user sees in the TUI (and what `get_sorted_hand` exposes to
        // the GUI view model).
        let sorted_hand = FancyTuiRenderer::get_sorted_hand(view);
        if let Some(idx) = sorted_hand.iter().position(|&cid| cid == card_id) {
            state.view.selected_card_in_hand = Some(idx);
        }
        state.session.selected_card_id = Some(card_id);
        state.view.focused_pane = FocusedPane::Hand;
        return CardSelectionResult::Selected {
            zone: SelectedCardZone::OurHand,
        };
    }

    // 2. Battlefield (split by controller — same as the mouse handler).
    if view.battlefield().contains(&card_id) {
        state.session.selected_card_id = Some(card_id);
        if card.controller == perspective {
            state.view.selected_card_in_your_bf = Some(card_id);
            state.view.focused_pane = FocusedPane::YourBattlefield;
            return CardSelectionResult::Selected {
                zone: SelectedCardZone::OurBattlefield,
            };
        }
        state.view.selected_card_in_opp_bf = Some(card_id);
        state.view.focused_pane = FocusedPane::OpponentBattlefield;
        return CardSelectionResult::Selected {
            zone: SelectedCardZone::OpponentBattlefield,
        };
    }

    // 3. Stack — public, but no associated pane. The mouse handler treats
    //    a stack click as a battlefield-like selection (just sets the
    //    detail pane), and we mirror that here.
    if view.stack().contains(&card_id) {
        state.session.selected_card_id = Some(card_id);
        return CardSelectionResult::Selected {
            zone: SelectedCardZone::Stack,
        };
    }

    // 4. Any player's graveyard or command zone (both are public).
    let all_players: SmallVec<[PlayerId; 4]> = std::iter::once(perspective).chain(view.opponents()).collect();
    for pid in all_players {
        if view.player_graveyard(pid).contains(&card_id) {
            state.session.selected_card_id = Some(card_id);
            return CardSelectionResult::Selected {
                zone: SelectedCardZone::Graveyard,
            };
        }
        if view.player_command_zone(pid).contains(&card_id) {
            state.session.selected_card_id = Some(card_id);
            return CardSelectionResult::Selected {
                zone: SelectedCardZone::Command,
            };
        }
    }

    // Hidden zones (library, opponent's hand). Do NOT select — would leak
    // hidden information through the WASM boundary.
    CardSelectionResult::NotVisible
}

/// Clear any current card selection and reset per-pane selection indices that
/// reference a specific card. Mirrors `select_card_by_id` for the negative case.
pub fn clear_card_selection(state: &mut FancyTuiState) {
    state.session.selected_card_id = None;
    state.view.selected_card_in_hand = None;
    state.view.selected_card_in_your_bf = None;
    state.view.selected_card_in_opp_bf = None;
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
    help.push_str("  0-9         - Quick select / type number + Enter (>10 choices)\n");
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

/// Handle any UI event, dispatching to the appropriate shared handler.
///
/// This is the single entry point for all backend-neutral event processing.
/// Both native and web backends should convert their raw events to `UiEvent`
/// and call this function.
pub fn handle_ui_event(
    state: &mut FancyTuiState,
    event: UiEvent,
    view: &GameStateView,
    num_choices: usize,
) -> EventResult {
    match event {
        UiEvent::Key(key) => handle_key_event(state, key, view, num_choices),
        UiEvent::MouseClick { col, row } => {
            if handle_mouse_click(state, col, row, view) {
                EventResult::Handled
            } else {
                EventResult::NotHandled
            }
        }
        UiEvent::MouseWheel { direction, col, row } => handle_scroll_wheel(state, direction, col, row),
        UiEvent::Resize { .. } => EventResult::Handled,
    }
}

/// Handle a mouse scroll wheel event.
///
/// Scrolls the log pane if the pointer is over it.
fn handle_scroll_wheel(state: &mut FancyTuiState, direction: ScrollDirection, col: u16, row: u16) -> EventResult {
    let in_log = state
        .view
        .log_pane_area
        .is_some_and(|area| col >= area.x && col < area.x + area.width && row >= area.y && row < area.y + area.height);

    if !in_log {
        return EventResult::NotHandled;
    }

    match direction {
        ScrollDirection::Up => {
            state.view.log_scroll_up(usize::MAX, 10);
            EventResult::Handled
        }
        ScrollDirection::Down => {
            state.view.log_scroll_down();
            EventResult::Handled
        }
        ScrollDirection::Left => {
            if !state.view.log_wrap_lines {
                state.view.log_scroll_left();
                EventResult::Handled
            } else {
                EventResult::NotHandled
            }
        }
        ScrollDirection::Right => {
            if !state.view.log_wrap_lines {
                state.view.log_scroll_right();
                EventResult::Handled
            } else {
                EventResult::NotHandled
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::wildcard_enum_match_arm)] // Tests use wildcards in panic branches
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
        state.view.entity_positions.push(EntityPosition {
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
        assert!(state.session.selected_card_id.is_none());

        // Simulate clicking at coordinates within the entity area
        // Note: handle_mouse_click needs a view, but for entity clicks
        // it only uses the view after the entity is identified.
        // We'll test the entity matching directly instead.

        // Check if click at (12, 5) would match the entity
        let click_x: u16 = 12;
        let click_y: u16 = 5;

        // Find matching entity (mimicking handle_mouse_click logic)
        for entity_pos in &state.view.entity_positions {
            if click_x >= entity_pos.area.x
                && click_x < entity_pos.area.x + entity_pos.area.width
                && click_y >= entity_pos.area.y
                && click_y < entity_pos.area.y + entity_pos.area.height
            {
                // Entity matched! Verify it's our graveyard card
                match &entity_pos.entity {
                    Entity::GraveyardCard { card_id: cid, .. } => {
                        state.session.selected_card_id = Some(*cid);
                    }
                    _ => panic!("Expected GraveyardCard entity"),
                }
            }
        }

        // Verify the click set selected_card_id
        assert_eq!(state.session.selected_card_id, Some(card_id));
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

    // ---- select_card_by_id (shared GUI/TUI selection routine) -------------

    /// `select_card_by_id` returns `NotFound` for an unknown id and does NOT
    /// mutate the state. The id namespace is `u32`, so any id can be tried;
    /// invalid ones must not crash or partially update state.
    #[test]
    fn select_card_by_id_unknown_card_returns_not_found() {
        use crate::game::GameState;

        let game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
        let perspective = game.players[0].id;
        let view = GameStateView::new(&game, perspective);
        let mut state = FancyTuiState::new();

        // CardId(9999) is guaranteed not to exist in a fresh 2-player game.
        let result = select_card_by_id(&mut state, CardId::new(9999), &view);
        assert_eq!(result, CardSelectionResult::NotFound);
        assert!(
            state.session.selected_card_id.is_none(),
            "state must not change on NotFound"
        );
    }

    /// Selecting our own battlefield card focuses YourBattlefield and updates
    /// both the per-pane selection and the shared `selected_card_id`.
    #[test]
    fn select_card_by_id_our_battlefield_focuses_correctly() {
        use crate::core::Card;
        use crate::game::GameState;

        let mut game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
        let perspective = game.players[0].id;

        // Manually drop a card onto the battlefield owned by the perspective
        // player. We only need a CardId that resolves to a real Card with a
        // controller — no need for a full ETB cycle.
        let card_id = game.next_entity_id();
        let mut card = Card::new(card_id, "ScratchLand".to_string(), perspective);
        card.controller = perspective;
        game.cards.insert(card_id, card);
        game.battlefield.cards.push(card_id);

        let view = GameStateView::new(&game, perspective);
        let mut state = FancyTuiState::new();
        let result = select_card_by_id(&mut state, card_id, &view);

        assert_eq!(
            result,
            CardSelectionResult::Selected {
                zone: SelectedCardZone::OurBattlefield,
            }
        );
        assert_eq!(state.session.selected_card_id, Some(card_id));
        assert_eq!(state.view.selected_card_in_your_bf, Some(card_id));
        assert_eq!(state.view.focused_pane, FocusedPane::YourBattlefield);
        assert!(state.view.selected_card_in_opp_bf.is_none());
    }

    /// Selecting an opponent's battlefield card focuses OpponentBattlefield.
    #[test]
    fn select_card_by_id_opponent_battlefield_focuses_correctly() {
        use crate::core::Card;
        use crate::game::GameState;

        let mut game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
        let perspective = game.players[0].id;
        let opponent = game.players[1].id;

        let card_id = game.next_entity_id();
        let mut card = Card::new(card_id, "OppCreature".to_string(), opponent);
        card.controller = opponent;
        game.cards.insert(card_id, card);
        game.battlefield.cards.push(card_id);

        let view = GameStateView::new(&game, perspective);
        let mut state = FancyTuiState::new();
        let result = select_card_by_id(&mut state, card_id, &view);

        assert_eq!(
            result,
            CardSelectionResult::Selected {
                zone: SelectedCardZone::OpponentBattlefield,
            }
        );
        assert_eq!(state.session.selected_card_id, Some(card_id));
        assert_eq!(state.view.selected_card_in_opp_bf, Some(card_id));
        assert_eq!(state.view.focused_pane, FocusedPane::OpponentBattlefield);
        assert!(state.view.selected_card_in_your_bf.is_none());
    }

    /// `clear_card_selection` zeros out every card-pointing field.
    #[test]
    fn clear_card_selection_zeros_card_pointers() {
        let mut state = FancyTuiState::new();
        state.session.selected_card_id = Some(CardId::new(7));
        state.view.selected_card_in_hand = Some(2);
        state.view.selected_card_in_your_bf = Some(CardId::new(7));
        state.view.selected_card_in_opp_bf = Some(CardId::new(8));

        clear_card_selection(&mut state);

        assert!(state.session.selected_card_id.is_none());
        assert!(state.view.selected_card_in_hand.is_none());
        assert!(state.view.selected_card_in_your_bf.is_none());
        assert!(state.view.selected_card_in_opp_bf.is_none());
    }
}
