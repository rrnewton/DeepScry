//! Native TUI implementation for deck builder
//!
//! This module contains the crossterm-based event loop for native platforms.
//! Event handling is delegated to shared handlers in `events.rs`.

use super::events::{
    handle_deck_builder_click, handle_deck_builder_key, handle_exit_dialog_key, DeckBuilderAction, DeckBuilderKey,
};
use super::state::DeckBuilderState;
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
use std::collections::HashSet;
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
///
/// # Errors
///
/// Returns an error if terminal setup, file I/O, or deck operations fail.
pub async fn run_deck_builder(
    config: DeckBuilderConfig,
    card_names: Vec<String>,
    card_definitions: HashMap<String, Arc<CardDefinition>>,
    edition_index: Option<crate::loader::CardEditionIndex>,
) -> Result<()> {
    let filtered_cards = card_names;

    println!("Loaded {} cards for deck building", filtered_cards.len());
    println!("Output will be saved to: {}", config.output_file);

    let known_cards: HashSet<String> = filtered_cards.iter().cloned().collect();

    let (initial_deck, import_problems) = if let Some(ref input_file) = config.input_file {
        let path = Path::new(input_file);
        if path.exists() {
            match DeckLoader::load_from_file_with_problems(path) {
                Ok(parse_result) => {
                    let total = parse_result.deck_list.total_cards();
                    let mut problems = parse_result.problems;

                    let known_refs: HashSet<&str> = known_cards.iter().map(|s| s.as_str()).collect();
                    let missing_card_problems = DeckLoader::validate_cards(&parse_result.deck_list, &known_refs);
                    problems.extend(missing_card_problems);

                    if problems.is_empty() {
                        println!("Loaded existing deck: {} ({} cards)", input_file, total);
                    } else {
                        println!(
                            "Loaded deck: {} ({} cards, {} problems to fix)",
                            input_file,
                            total,
                            problems.len()
                        );
                    }

                    (Some(parse_result.deck_list), problems)
                }
                Err(e) => {
                    eprintln!("Warning: Failed to load deck '{}': {}", input_file, e);
                    (None, Vec::new())
                }
            }
        } else {
            println!("Creating new deck: {}", input_file);
            (None, Vec::new())
        }
    } else {
        (None, Vec::new())
    };

    println!("\nStarting deck builder...\n");
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    let mut state = DeckBuilderState::new(filtered_cards, card_definitions, edition_index);

    if let Some(deck_list) = initial_deck {
        for entry in deck_list.main_deck {
            if known_cards.contains(&entry.card_name) {
                state.deck.insert(entry.card_name, entry.count);
            }
        }
    }

    if !import_problems.is_empty() {
        state.set_import_problems(import_problems);
    }

    enable_raw_mode().map_err(crate::MtgError::IoError)?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture).map_err(crate::MtgError::IoError)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).map_err(crate::MtgError::IoError)?;

    let result = run_main_loop(&mut terminal, &mut state);

    disable_raw_mode().map_err(crate::MtgError::IoError)?;
    execute!(terminal.backend_mut(), DisableMouseCapture, LeaveAlternateScreen).map_err(crate::MtgError::IoError)?;
    terminal.show_cursor().map_err(crate::MtgError::IoError)?;

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

/// Main event loop — thin adapter using shared handlers
#[allow(clippy::wildcard_enum_match_arm)]
fn run_main_loop(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, state: &mut DeckBuilderState) -> Result<bool> {
    loop {
        terminal.draw(|f| draw_ui(f, state)).map_err(crate::MtgError::IoError)?;

        if event::poll(std::time::Duration::from_millis(100)).map_err(crate::MtgError::IoError)? {
            match event::read().map_err(crate::MtgError::IoError)? {
                Event::Key(key) => {
                    if key.kind != KeyEventKind::Press {
                        continue;
                    }

                    let db_key = match crossterm_to_deck_builder_key(key.code, key.modifiers) {
                        Some(k) => k,
                        None => continue,
                    };

                    if let Some(action) = handle_exit_dialog_key(state, db_key) {
                        match action {
                            DeckBuilderAction::SaveAndExit => return Ok(true),
                            DeckBuilderAction::ExitWithoutSaving => return Ok(false),
                            DeckBuilderAction::Handled | DeckBuilderAction::NotHandled => continue,
                        }
                    }

                    match handle_deck_builder_key(state, db_key) {
                        DeckBuilderAction::SaveAndExit => return Ok(true),
                        DeckBuilderAction::ExitWithoutSaving => return Ok(false),
                        DeckBuilderAction::Handled | DeckBuilderAction::NotHandled => {}
                    }
                }
                Event::Mouse(mouse) => {
                    if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
                        handle_deck_builder_click(state, mouse.column, mouse.row);
                    }
                }
                _ => {}
            }
        }
    }
}

/// Convert crossterm key to backend-neutral `DeckBuilderKey`.
#[allow(clippy::wildcard_enum_match_arm)]
fn crossterm_to_deck_builder_key(code: KeyCode, modifiers: KeyModifiers) -> Option<DeckBuilderKey> {
    match code {
        KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => Some(DeckBuilderKey::CtrlC),
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

/// Save deck to file in .dck format
fn save_deck(state: &DeckBuilderState, output_file: &str) -> Result<()> {
    let deck_list = state.to_deck_list();
    deck_list.save_to_file(std::path::Path::new(output_file), None)
}
