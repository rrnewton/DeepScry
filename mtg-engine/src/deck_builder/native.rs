//! Native TUI implementation for deck builder
//!
//! This module contains the crossterm-based event loop for native platforms.
//! The shared state and UI rendering comes from the parent modules.

use super::state::{DeckBuilderState, FocusedPane};
use super::ui::draw_ui;
use crate::loader::{CardDefinition, DeckLoader};
use crate::Result;
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton,
        MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::collections::HashMap;
use std::io;
use std::path::Path;
use std::sync::Arc;

/// Deck builder configuration
pub struct DeckBuilderConfig {
    /// Output file path
    pub output_file: String,
    /// Input file path (if editing existing deck)
    pub input_file: Option<String>,
    /// Only include cards from sets released on or after this year
    pub start_year: Option<u16>,
    /// Only include cards from sets released on or before this year
    pub end_year: Option<u16>,
}

impl Default for DeckBuilderConfig {
    fn default() -> Self {
        Self {
            output_file: "output.dck".to_string(),
            input_file: None,
            start_year: None,
            end_year: None,
        }
    }
}

/// Run the deck builder TUI
pub async fn run_deck_builder(
    config: DeckBuilderConfig,
    card_names: Vec<String>,
    card_definitions: HashMap<String, Arc<CardDefinition>>,
    edition_index: Option<crate::loader::CardEditionIndex>,
) -> Result<()> {
    // Cards are already filtered by year in main.rs if start_year/end_year were specified
    let filtered_cards = card_names;

    println!("Loaded {} cards for deck building", filtered_cards.len());
    println!("Output will be saved to: {}", config.output_file);

    // Load existing deck if input_file is provided
    let initial_deck = if let Some(ref input_file) = config.input_file {
        let path = Path::new(input_file);
        if path.exists() {
            match DeckLoader::load_from_file(path) {
                Ok(deck_list) => {
                    println!(
                        "Loaded existing deck: {} ({} cards)",
                        input_file,
                        deck_list.total_cards()
                    );
                    Some(deck_list)
                }
                Err(e) => {
                    eprintln!("Warning: Failed to load deck '{}': {}", input_file, e);
                    None
                }
            }
        } else {
            println!("Creating new deck: {}", input_file);
            None
        }
    } else {
        None
    };

    println!("\nStarting deck builder...\n");

    // Small delay so user can see the message
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    let mut state = DeckBuilderState::new(filtered_cards, card_definitions, edition_index);

    // Pre-populate deck if we loaded one
    if let Some(deck_list) = initial_deck {
        for entry in deck_list.main_deck {
            state.deck.insert(entry.card_name, entry.count);
        }
    }

    // Setup terminal with mouse support
    enable_raw_mode().map_err(crate::MtgError::IoError)?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture).map_err(crate::MtgError::IoError)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).map_err(crate::MtgError::IoError)?;

    // Main loop
    let result = run_main_loop(&mut terminal, &mut state);

    // Restore terminal
    disable_raw_mode().map_err(crate::MtgError::IoError)?;
    execute!(terminal.backend_mut(), DisableMouseCapture, LeaveAlternateScreen).map_err(crate::MtgError::IoError)?;
    terminal.show_cursor().map_err(crate::MtgError::IoError)?;

    // Handle result
    match result {
        Ok(should_save) => {
            if should_save && !state.deck.is_empty() {
                save_deck(&state, &config.output_file)?;
                println!("\nDeck saved to: {}", config.output_file);
                println!("Total: {} cards ({} unique)", state.total_cards(), state.unique_cards());
            } else if state.deck.is_empty() {
                println!("\nNo cards added, deck not saved.");
            } else {
                println!("\nExited without saving.");
            }
            Ok(())
        }
        Err(e) => Err(e),
    }
}

/// Main event loop
fn run_main_loop(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, state: &mut DeckBuilderState) -> Result<bool> {
    loop {
        // Draw UI
        terminal.draw(|f| draw_ui(f, state)).map_err(crate::MtgError::IoError)?;

        // Handle input
        if event::poll(std::time::Duration::from_millis(100)).map_err(crate::MtgError::IoError)? {
            match event::read().map_err(crate::MtgError::IoError)? {
                Event::Key(key) => {
                    // Only process key press events, not release
                    if key.kind != KeyEventKind::Press {
                        continue;
                    }

                    // Handle exit dialog
                    if state.show_exit_dialog {
                        match key.code {
                            KeyCode::Char('y') | KeyCode::Char('Y') => {
                                return Ok(true); // Save and exit
                            }
                            KeyCode::Char('q') | KeyCode::Char('Q') => {
                                return Ok(false); // Exit without saving
                            }
                            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                                state.show_exit_dialog = false; // Cancel, go back
                            }
                            _ => {}
                        }
                        continue;
                    }

                    // Clear status message on any key
                    state.status_message = None;

                    match key.code {
                        KeyCode::Tab => {
                            state.toggle_focus();
                        }
                        KeyCode::Esc => {
                            // First ESC clears search, second ESC shows exit dialog
                            if !state.search_query.is_empty() {
                                state.search_query.clear();
                                state.update_search();
                            } else {
                                state.show_exit_dialog = true;
                            }
                        }
                        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            return Ok(false); // Exit without saving
                        }
                        KeyCode::Up => match state.focused_pane {
                            FocusedPane::Search => state.select_previous(),
                            FocusedPane::DeckSummary => {
                                let num_cols = state.deck_num_columns;
                                state.deck_select_previous(num_cols);
                            }
                        },
                        KeyCode::Down => match state.focused_pane {
                            FocusedPane::Search => state.select_next(),
                            FocusedPane::DeckSummary => {
                                let num_cols = state.deck_num_columns;
                                state.deck_select_next(num_cols);
                            }
                        },
                        KeyCode::Left => {
                            if state.focused_pane == FocusedPane::DeckSummary {
                                let num_cols = state.deck_num_columns;
                                state.deck_select_left(num_cols);
                            }
                        }
                        KeyCode::Right => {
                            if state.focused_pane == FocusedPane::DeckSummary {
                                let num_cols = state.deck_num_columns;
                                state.deck_select_right(num_cols);
                            }
                        }
                        KeyCode::PageUp => {
                            match state.focused_pane {
                                FocusedPane::Search => {
                                    // Page up by one screen
                                    let page_size = state.max_results;
                                    state.selected_index = state.selected_index.saturating_sub(page_size);
                                    state.scroll_offset = state.scroll_offset.saturating_sub(page_size);
                                }
                                FocusedPane::DeckSummary => {
                                    state.deck_selected_index = state.deck_selected_index.saturating_sub(10);
                                }
                            }
                        }
                        KeyCode::PageDown => match state.focused_pane {
                            FocusedPane::Search => {
                                let page_size = state.max_results;
                                let max_idx = state.search_results.len().saturating_sub(1);
                                state.selected_index = (state.selected_index + page_size).min(max_idx);
                                // Ensure selected_index is visible: scroll so it's at the bottom of the view
                                if state.selected_index >= state.scroll_offset + page_size {
                                    state.scroll_offset = state.selected_index.saturating_sub(page_size - 1);
                                }
                            }
                            FocusedPane::DeckSummary => {
                                let max_idx = state.deck.len().saturating_sub(1);
                                state.deck_selected_index = (state.deck_selected_index + 10).min(max_idx);
                            }
                        },
                        KeyCode::Home => match state.focused_pane {
                            FocusedPane::Search => {
                                state.selected_index = 0;
                                state.scroll_offset = 0;
                            }
                            FocusedPane::DeckSummary => {
                                state.deck_selected_index = 0;
                            }
                        },
                        KeyCode::End => match state.focused_pane {
                            FocusedPane::Search => {
                                let max_idx = state.search_results.len().saturating_sub(1);
                                state.selected_index = max_idx;
                                // Scroll so the last item is at the bottom of the view
                                state.scroll_offset = max_idx.saturating_sub(state.max_results - 1);
                            }
                            FocusedPane::DeckSummary => {
                                state.deck_selected_index = state.deck.len().saturating_sub(1);
                            }
                        },
                        KeyCode::Enter => {
                            state.add_selected(1);
                        }
                        KeyCode::Delete => {
                            state.remove_selected();
                        }
                        KeyCode::Char(c) if c.is_ascii_digit() && c != '0' => {
                            let count = c.to_digit(10).unwrap() as u8;
                            state.add_selected(count);
                        }
                        KeyCode::Char(c) => {
                            // Only allow typing in search mode
                            if state.focused_pane == FocusedPane::Search {
                                state.search_query.push(c);
                                state.update_search();
                            }
                        }
                        KeyCode::Backspace => {
                            if state.focused_pane == FocusedPane::Search {
                                state.search_query.pop();
                                state.update_search();
                            }
                        }
                        _ => {}
                    }
                }
                Event::Mouse(mouse) => {
                    if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
                        let x = mouse.column;
                        let y = mouse.row;

                        // Check if click is within deck summary area
                        if let Some(area) = state.deck_summary_area {
                            if x >= area.x && x < area.x + area.width && y >= area.y && y < area.y + area.height {
                                state.focused_pane = FocusedPane::DeckSummary;
                                state.status_message = None;
                            }
                        }

                        // Check if click is within search results area
                        if let Some(area) = state.search_results_area {
                            if x >= area.x && x < area.x + area.width && y >= area.y && y < area.y + area.height {
                                state.focused_pane = FocusedPane::Search;
                                state.status_message = None;
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

/// Save deck to file in .dck format (uses shared DeckList::save_to_file)
fn save_deck(state: &DeckBuilderState, output_file: &str) -> Result<()> {
    let deck_list = state.to_deck_list();
    deck_list.save_to_file(std::path::Path::new(output_file), None)
}
