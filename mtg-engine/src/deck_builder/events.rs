//! Shared event handling for Deck Builder (native and WASM)
//!
//! This module provides common input handling logic that can be used by both
//! the native deck builder and the WASM browser implementation, eliminating
//! duplicated event routing between the two backends.

use super::state::{DeckBuilderState, FocusedPane};

/// Result of handling a deck builder event
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeckBuilderAction {
    /// Event was handled, UI should be redrawn
    Handled,
    /// Event was not handled (unknown key)
    NotHandled,
    /// Save the deck and exit
    SaveAndExit,
    /// Exit without saving
    ExitWithoutSaving,
}

/// Handle a key event in the exit dialog.
///
/// Returns `Some(action)` if the dialog handled the key, `None` if the dialog
/// is not showing (caller should process the key normally).
pub fn handle_exit_dialog_key(state: &mut DeckBuilderState, key: DeckBuilderKey) -> Option<DeckBuilderAction> {
    if !state.show_exit_dialog {
        return None;
    }

    match key {
        DeckBuilderKey::Char('y' | 'Y') => Some(DeckBuilderAction::SaveAndExit),
        DeckBuilderKey::Char('q' | 'Q') => Some(DeckBuilderAction::ExitWithoutSaving),
        DeckBuilderKey::Char('n' | 'N') | DeckBuilderKey::Escape => {
            state.show_exit_dialog = false;
            Some(DeckBuilderAction::Handled)
        }
        DeckBuilderKey::Up
        | DeckBuilderKey::Down
        | DeckBuilderKey::Left
        | DeckBuilderKey::Right
        | DeckBuilderKey::Tab
        | DeckBuilderKey::Enter
        | DeckBuilderKey::PageUp
        | DeckBuilderKey::PageDown
        | DeckBuilderKey::Home
        | DeckBuilderKey::End
        | DeckBuilderKey::Delete
        | DeckBuilderKey::Backspace
        | DeckBuilderKey::CtrlC
        | DeckBuilderKey::Char(_) => Some(DeckBuilderAction::NotHandled),
    }
}

/// Abstract key input for deck builder (subset of keys we handle)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeckBuilderKey {
    Up,
    Down,
    Left,
    Right,
    Tab,
    Enter,
    Escape,
    PageUp,
    PageDown,
    Home,
    End,
    Delete,
    Backspace,
    CtrlC,
    Char(char),
}

/// Handle a key event in normal mode (not exit dialog).
///
/// Mutates `DeckBuilderState` and returns the appropriate action.
pub fn handle_deck_builder_key(state: &mut DeckBuilderState, key: DeckBuilderKey) -> DeckBuilderAction {
    // Clear status message on any key
    state.status_message = None;

    match key {
        DeckBuilderKey::Tab => {
            state.toggle_focus();
            DeckBuilderAction::Handled
        }
        DeckBuilderKey::Escape => {
            if !state.search_query.is_empty() {
                state.search_query.clear();
                state.update_search();
            } else {
                state.show_exit_dialog = true;
            }
            DeckBuilderAction::Handled
        }
        DeckBuilderKey::CtrlC => DeckBuilderAction::ExitWithoutSaving,
        DeckBuilderKey::Up => {
            match state.focused_pane {
                FocusedPane::Search => state.select_previous(),
                FocusedPane::DeckSummary => {
                    let num_cols = state.deck_num_columns;
                    state.deck_select_previous(num_cols);
                }
                FocusedPane::Problems => state.problems_select_previous(),
            }
            DeckBuilderAction::Handled
        }
        DeckBuilderKey::Down => {
            match state.focused_pane {
                FocusedPane::Search => state.select_next(),
                FocusedPane::DeckSummary => {
                    let num_cols = state.deck_num_columns;
                    state.deck_select_next(num_cols);
                }
                FocusedPane::Problems => state.problems_select_next(),
            }
            DeckBuilderAction::Handled
        }
        DeckBuilderKey::Left => {
            if state.focused_pane == FocusedPane::DeckSummary {
                let num_cols = state.deck_num_columns;
                state.deck_select_left(num_cols);
            }
            DeckBuilderAction::Handled
        }
        DeckBuilderKey::Right => {
            if state.focused_pane == FocusedPane::DeckSummary {
                let num_cols = state.deck_num_columns;
                state.deck_select_right(num_cols);
            }
            DeckBuilderAction::Handled
        }
        DeckBuilderKey::PageUp => {
            match state.focused_pane {
                FocusedPane::Search => {
                    let page_size = state.max_results;
                    state.selected_index = state.selected_index.saturating_sub(page_size);
                    state.scroll_offset = state.scroll_offset.saturating_sub(page_size);
                }
                FocusedPane::DeckSummary => {
                    state.deck_selected_index = state.deck_selected_index.saturating_sub(10);
                }
                FocusedPane::Problems => {
                    let page_size = state.max_results;
                    state.problems_selected_index = state.problems_selected_index.saturating_sub(page_size);
                    state.problems_scroll_offset = state.problems_scroll_offset.saturating_sub(page_size);
                }
            }
            DeckBuilderAction::Handled
        }
        DeckBuilderKey::PageDown => {
            match state.focused_pane {
                FocusedPane::Search => {
                    let page_size = state.max_results;
                    let max_idx = state.search_results.len().saturating_sub(1);
                    state.selected_index = (state.selected_index + page_size).min(max_idx);
                    if state.selected_index >= state.scroll_offset + page_size {
                        state.scroll_offset = state.selected_index.saturating_sub(page_size - 1);
                    }
                }
                FocusedPane::DeckSummary => {
                    let max_idx = state.deck.len().saturating_sub(1);
                    state.deck_selected_index = (state.deck_selected_index + 10).min(max_idx);
                }
                FocusedPane::Problems => {
                    let page_size = state.max_results;
                    let max_idx = state.import_problems.len().saturating_sub(1);
                    state.problems_selected_index = (state.problems_selected_index + page_size).min(max_idx);
                    if state.problems_selected_index >= state.problems_scroll_offset + page_size {
                        state.problems_scroll_offset = state.problems_selected_index.saturating_sub(page_size - 1);
                    }
                }
            }
            DeckBuilderAction::Handled
        }
        DeckBuilderKey::Home => {
            match state.focused_pane {
                FocusedPane::Search => {
                    state.selected_index = 0;
                    state.scroll_offset = 0;
                }
                FocusedPane::DeckSummary => {
                    state.deck_selected_index = 0;
                }
                FocusedPane::Problems => {
                    state.problems_selected_index = 0;
                    state.problems_scroll_offset = 0;
                }
            }
            DeckBuilderAction::Handled
        }
        DeckBuilderKey::End => {
            match state.focused_pane {
                FocusedPane::Search => {
                    let max_idx = state.search_results.len().saturating_sub(1);
                    state.selected_index = max_idx;
                    state.scroll_offset = max_idx.saturating_sub(state.max_results - 1);
                }
                FocusedPane::DeckSummary => {
                    state.deck_selected_index = state.deck.len().saturating_sub(1);
                }
                FocusedPane::Problems => {
                    let max_idx = state.import_problems.len().saturating_sub(1);
                    state.problems_selected_index = max_idx;
                    state.problems_scroll_offset = max_idx.saturating_sub(state.max_results - 1);
                }
            }
            DeckBuilderAction::Handled
        }
        DeckBuilderKey::Enter => {
            match state.focused_pane {
                FocusedPane::Search => state.add_selected(1),
                FocusedPane::DeckSummary => state.increment_deck_selected(),
                FocusedPane::Problems => {}
            }
            DeckBuilderAction::Handled
        }
        DeckBuilderKey::Delete => {
            if state.focused_pane == FocusedPane::Problems {
                state.remove_selected_problem();
            } else {
                state.remove_selected();
            }
            DeckBuilderAction::Handled
        }
        DeckBuilderKey::Backspace => {
            match state.focused_pane {
                FocusedPane::Problems => state.remove_selected_problem(),
                FocusedPane::DeckSummary => state.remove_selected(),
                FocusedPane::Search => {
                    state.search_query.pop();
                    state.update_search();
                }
            }
            DeckBuilderAction::Handled
        }
        DeckBuilderKey::Char(c) => {
            if c.is_ascii_digit() && c != '0' {
                let count = c.to_digit(10).unwrap() as u8;
                match state.focused_pane {
                    FocusedPane::Search => state.set_selected(count),
                    FocusedPane::DeckSummary => state.set_deck_selected(count),
                    FocusedPane::Problems => {}
                }
            } else if state.focused_pane == FocusedPane::Search {
                state.search_query.push(c);
                state.update_search();
            }
            DeckBuilderAction::Handled
        }
    }
}

/// Handle a mouse click at terminal cell coordinates.
///
/// Checks which pane area was clicked and updates focus. Returns true
/// if the click was handled and the UI should be redrawn.
pub fn handle_deck_builder_click(state: &mut DeckBuilderState, col: u16, row: u16) -> bool {
    let mut handled = false;

    // Check if click is within deck summary area
    if let Some(area) = state.deck_summary_area {
        if col >= area.x && col < area.x + area.width && row >= area.y && row < area.y + area.height {
            state.focused_pane = FocusedPane::DeckSummary;
            state.status_message = None;
            handled = true;
        }
    }

    // Check if click is within search input area
    if let Some(area) = state.search_input_area {
        if col >= area.x && col < area.x + area.width && row >= area.y && row < area.y + area.height {
            state.focused_pane = FocusedPane::Search;
            state.status_message = None;
            handled = true;
        }
    }

    // Check if click is within search results area
    if let Some(area) = state.search_results_area {
        if col >= area.x && col < area.x + area.width && row >= area.y && row < area.y + area.height {
            state.focused_pane = FocusedPane::Search;
            state.status_message = None;
            handled = true;
        }
    }

    // Check if click is within problems area
    if let Some(area) = state.problems_area {
        if col >= area.x && col < area.x + area.width && row >= area.y && row < area.y + area.height {
            state.focused_pane = FocusedPane::Problems;
            state.status_message = None;
            handled = true;
        }
    }

    handled
}
