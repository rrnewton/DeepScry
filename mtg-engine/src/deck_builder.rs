//! Fast Deck Entry Mode - Interactive TUI for rapid deck building
//!
//! Provides a streamlined interface for entering paper decks with minimal keystrokes:
//! - Fuzzy search with real-time preview (prefix matches prioritized)
//! - Number keys (1-9) to add multiple copies
//! - Enter to add single copy
//! - Arrow keys to navigate results
//! - First ESC clears search, second ESC saves and exits

use crate::core::{CardType, Color as MtgColor};
use crate::loader::{CardDefinition, DeckEntry, DeckList, DeckLoader};
use crate::Result;
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton,
        MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
    Frame, Terminal,
};
use std::collections::HashMap;
use std::io;
use std::path::Path;
use std::sync::Arc;

/// Type alias for grouped card entries: (card_name, count, optional_definition)
type CardEntryGroup<'a> = Vec<(&'a String, &'a u8, Option<&'a CardDefinition>)>;

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

/// Which pane currently has keyboard focus
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FocusedPane {
    Search,
    DeckSummary,
}

/// State for the deck builder TUI
struct DeckBuilderState {
    /// All available card names (sorted)
    all_cards: Vec<String>,
    /// Card definitions cache (loaded on demand for details display)
    card_definitions: HashMap<String, Arc<CardDefinition>>,
    /// Current search query
    search_query: String,
    /// Filtered search results (indices into all_cards)
    search_results: Vec<usize>,
    /// Currently selected result index in search results
    selected_index: usize,
    /// First visible result index (for pagination)
    scroll_offset: usize,
    /// Current deck: card name -> count
    deck: HashMap<String, u8>,
    /// Whether to show exit confirmation dialog
    show_exit_dialog: bool,
    /// Message to show at bottom (e.g., "Added 4x Lightning Bolt")
    status_message: Option<String>,
    /// Maximum number of results to show (dynamic based on pane height)
    max_results: usize,
    /// Which pane has keyboard focus
    focused_pane: FocusedPane,
    /// Rect areas for click detection (set during draw)
    deck_summary_area: Option<Rect>,
    search_results_area: Option<Rect>,
    /// Selected card index in deck summary (flattened across all categories)
    deck_selected_index: usize,
    /// Edition index for showing card release info (optional)
    edition_index: Option<crate::loader::CardEditionIndex>,
    /// Number of columns in deck summary (updated during draw)
    deck_num_columns: usize,
}

impl DeckBuilderState {
    fn new(
        all_cards: Vec<String>,
        card_definitions: HashMap<String, Arc<CardDefinition>>,
        edition_index: Option<crate::loader::CardEditionIndex>,
    ) -> Self {
        Self {
            all_cards,
            card_definitions,
            search_query: String::new(),
            search_results: Vec::new(),
            selected_index: 0,
            scroll_offset: 0,
            deck: HashMap::new(),
            show_exit_dialog: false,
            status_message: None,
            max_results: 10, // Will be updated based on pane height
            focused_pane: FocusedPane::Search,
            deck_summary_area: None,
            search_results_area: None,
            deck_selected_index: 0,
            edition_index,
            deck_num_columns: 1, // Will be updated during draw
        }
    }

    /// Toggle focus between panes
    fn toggle_focus(&mut self) {
        self.focused_pane = match self.focused_pane {
            FocusedPane::Search => FocusedPane::DeckSummary,
            FocusedPane::DeckSummary => FocusedPane::Search,
        };
    }

    /// Update search results based on current query
    fn update_search(&mut self) {
        if self.search_query.is_empty() {
            self.search_results.clear();
            self.selected_index = 0;
            self.scroll_offset = 0;
            return;
        }

        let query_lower = self.search_query.to_lowercase();

        // Remember what was selected before update (if anything)
        let previously_selected = if !self.search_results.is_empty() && self.selected_index < self.search_results.len()
        {
            Some(self.search_results[self.selected_index])
        } else {
            None
        };

        // Collect matches with scores (lower score = better match)
        let mut scored_results: Vec<(usize, u8)> = self
            .all_cards
            .iter()
            .enumerate()
            .filter_map(|(idx, name)| {
                let name_lower = name.to_lowercase();
                match_score(&name_lower, &query_lower).map(|score| (idx, score))
            })
            .collect();

        // Sort by score (prefix matches first), then alphabetically
        scored_results.sort_by(|(idx_a, score_a), (idx_b, score_b)| {
            score_a
                .cmp(score_b)
                .then_with(|| self.all_cards[*idx_a].cmp(&self.all_cards[*idx_b]))
        });

        // Keep all results for pagination (removed the take(max_results) limit)
        self.search_results = scored_results.into_iter().map(|(idx, _)| idx).collect();

        // Try to follow the previously selected card
        if let Some(prev_idx) = previously_selected {
            if let Some(new_pos) = self.search_results.iter().position(|&idx| idx == prev_idx) {
                self.selected_index = new_pos;
                // Adjust scroll_offset to keep selection visible
                if self.selected_index < self.scroll_offset {
                    self.scroll_offset = self.selected_index;
                } else if self.selected_index >= self.scroll_offset + self.max_results {
                    self.scroll_offset = self.selected_index.saturating_sub(self.max_results - 1);
                }
            } else {
                self.selected_index = 0;
                self.scroll_offset = 0;
            }
        } else {
            self.selected_index = 0;
            self.scroll_offset = 0;
        }
    }

    /// Get the currently selected card name (if any)
    fn selected_card(&self) -> Option<&str> {
        if self.search_results.is_empty() {
            return None;
        }
        let idx = self.search_results.get(self.selected_index)?;
        Some(&self.all_cards[*idx])
    }

    /// Add copies of the selected card to the deck
    fn add_selected(&mut self, count: u8) {
        if let Some(card_name) = self.selected_card() {
            let card_name = card_name.to_string();
            let entry = self.deck.entry(card_name.clone()).or_insert(0);
            *entry = entry.saturating_add(count);
            self.status_message = Some(format!("Added {}x {}", count, card_name));
        }
    }

    /// Remove one copy of the selected card from the deck
    fn remove_selected(&mut self) {
        if let Some(card_name) = self.selected_card() {
            let card_name = card_name.to_string();
            if let Some(count) = self.deck.get_mut(&card_name) {
                if *count > 1 {
                    *count -= 1;
                    self.status_message = Some(format!("Removed 1x {} ({}x remaining)", card_name, *count));
                } else {
                    self.deck.remove(&card_name);
                    self.status_message = Some(format!("Removed {} from deck", card_name));
                }
            }
        }
    }

    /// Move selection up
    fn select_previous(&mut self) {
        if !self.search_results.is_empty() && self.selected_index > 0 {
            self.selected_index -= 1;
            // Keep selected item visible
            if self.selected_index < self.scroll_offset {
                self.scroll_offset = self.selected_index;
            }
        }
    }

    /// Move selection down
    fn select_next(&mut self) {
        if !self.search_results.is_empty() && self.selected_index < self.search_results.len() - 1 {
            self.selected_index += 1;
            // Keep selected item visible
            if self.selected_index >= self.scroll_offset + self.max_results {
                self.scroll_offset = self.selected_index.saturating_sub(self.max_results - 1);
            }
        }
    }

    /// Get total card count in deck
    fn total_cards(&self) -> usize {
        self.deck.values().map(|&c| c as usize).sum()
    }

    /// Get unique card count in deck
    fn unique_cards(&self) -> usize {
        self.deck.len()
    }

    /// Build DeckList from current state
    fn to_deck_list(&self) -> DeckList {
        let mut main_deck: Vec<DeckEntry> = self
            .deck
            .iter()
            .map(|(name, &count)| DeckEntry {
                card_name: name.clone(),
                count,
            })
            .collect();

        // Sort alphabetically for consistent output
        main_deck.sort_by(|a, b| a.card_name.cmp(&b.card_name));

        DeckList {
            main_deck,
            sideboard: Vec::new(),
        }
    }

    /// Get flattened list of deck card names in display order (by category, then sorted within)
    fn get_deck_cards_ordered(&self) -> Vec<String> {
        let category_order = [
            CardCategory::Creature,
            CardCategory::Spell,
            CardCategory::Artifact,
            CardCategory::Land,
        ];

        // Group cards by category
        let mut by_category: HashMap<CardCategory, CardEntryGroup<'_>> = HashMap::new();
        for (name, count) in &self.deck {
            let card_def = self.card_definitions.get(name).map(|arc| arc.as_ref());
            let category = card_def
                .map(|c| CardCategory::from_types(&c.types))
                .unwrap_or(CardCategory::Spell);
            by_category.entry(category).or_default().push((name, count, card_def));
        }

        // Sort each category by color, then descending CMC
        for entries in by_category.values_mut() {
            entries.sort_by(|a, b| match (a.2, b.2) {
                (Some(ca), Some(cb)) => card_sort_key(ca).cmp(&card_sort_key(cb)),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => a.0.cmp(b.0),
            });
        }

        // Build flattened list
        let mut result = Vec::new();
        for category in category_order {
            if let Some(entries) = by_category.get(&category) {
                for (name, _, _) in entries {
                    result.push((*name).clone());
                }
            }
        }
        result
    }

    /// Get the currently selected card name based on focused pane
    fn get_active_selected_card(&self) -> Option<String> {
        match self.focused_pane {
            FocusedPane::Search => self.selected_card().map(|s| s.to_string()),
            FocusedPane::DeckSummary => {
                let ordered = self.get_deck_cards_ordered();
                ordered.get(self.deck_selected_index).cloned()
            }
        }
    }

    /// Move deck selection up
    fn deck_select_previous(&mut self) {
        if self.deck_selected_index > 0 {
            self.deck_selected_index -= 1;
        }
    }

    /// Move deck selection down
    fn deck_select_next(&mut self) {
        let count = self.deck.len();
        if count > 0 && self.deck_selected_index < count - 1 {
            self.deck_selected_index += 1;
        }
    }

    /// Move deck selection left (to previous column in same row)
    fn deck_select_left(&mut self, num_columns: usize) {
        if num_columns <= 1 || self.deck.is_empty() {
            return;
        }

        // Get category layout info for the current selection
        let category_info = self.get_category_layout_info(num_columns);
        if let Some((cat_start, _cat_size, num_rows)) = self.find_category_for_index(&category_info) {
            let local_idx = self.deck_selected_index - cat_start;
            let row = local_idx % num_rows;
            let col = local_idx / num_rows;

            if col > 0 {
                // Move left one column
                self.deck_selected_index = cat_start + row + (col - 1) * num_rows;
            }
            // If already in first column, stay put
        }
    }

    /// Move deck selection right (to next column in same row)
    fn deck_select_right(&mut self, num_columns: usize) {
        if num_columns <= 1 || self.deck.is_empty() {
            return;
        }

        let category_info = self.get_category_layout_info(num_columns);
        if let Some((cat_start, cat_size, num_rows)) = self.find_category_for_index(&category_info) {
            let local_idx = self.deck_selected_index - cat_start;
            let row = local_idx % num_rows;
            let col = local_idx / num_rows;

            // Check if there's a card in the next column at this row
            let next_idx = row + (col + 1) * num_rows;
            if next_idx < cat_size {
                self.deck_selected_index = cat_start + next_idx;
            }
            // If no card to the right, stay put
        }
    }

    /// Get layout info for each category: (start_index, size, num_rows)
    fn get_category_layout_info(&self, num_columns: usize) -> Vec<(usize, usize, usize)> {
        let category_order = [
            CardCategory::Creature,
            CardCategory::Spell,
            CardCategory::Artifact,
            CardCategory::Land,
        ];

        let mut by_category: HashMap<CardCategory, usize> = HashMap::new();
        for name in self.deck.keys() {
            let card_def = self.card_definitions.get(name).map(|arc| arc.as_ref());
            let category = card_def
                .map(|c| CardCategory::from_types(&c.types))
                .unwrap_or(CardCategory::Spell);
            *by_category.entry(category).or_insert(0) += 1;
        }

        let mut result = Vec::new();
        let mut start_idx = 0;
        for category in category_order {
            if let Some(&size) = by_category.get(&category) {
                let num_rows = size.div_ceil(num_columns);
                result.push((start_idx, size, num_rows));
                start_idx += size;
            }
        }
        result
    }

    /// Find which category contains the current deck_selected_index
    /// Returns (cat_start, cat_size, num_rows)
    fn find_category_for_index(&self, category_info: &[(usize, usize, usize)]) -> Option<(usize, usize, usize)> {
        for &(start, size, num_rows) in category_info {
            if self.deck_selected_index >= start && self.deck_selected_index < start + size {
                return Some((start, size, num_rows));
            }
        }
        None
    }
}

/// Match scoring: returns Some(score) if matches, None if no match
/// Lower score = better match:
/// - 0: Exact prefix match
/// - 1: Prefix match (case-insensitive, already lowercased)
/// - 2: Contains as substring
/// - 3: Subsequence match
fn match_score(target: &str, query: &str) -> Option<u8> {
    if query.is_empty() {
        return Some(0);
    }

    // Prefix match is highest priority
    if target.starts_with(query) {
        return Some(0);
    }

    // Word-start prefix match (e.g., "bolt" matches "Lightning Bolt")
    for word in target.split_whitespace() {
        if word.starts_with(query) {
            return Some(1);
        }
    }

    // Contains as substring
    if target.contains(query) {
        return Some(2);
    }

    // Tokenized match: "spider punk" matches "Spider-Punk"
    // Split query by whitespace, check if all tokens are found (as word prefixes or substrings)
    // in the target, considering both whitespace and hyphen as word separators
    let query_tokens: Vec<&str> = query.split_whitespace().collect();
    if query_tokens.len() > 1 {
        // Split target by both whitespace and hyphens to get all "words"
        let target_words: Vec<&str> = target
            .split(|c: char| c.is_whitespace() || c == '-' || c == '\'')
            .filter(|s| !s.is_empty())
            .collect();

        // Check if all query tokens match word prefixes in target (in order)
        let mut word_idx = 0;
        let mut all_match = true;
        for token in &query_tokens {
            let mut found = false;
            while word_idx < target_words.len() {
                if target_words[word_idx].starts_with(token) {
                    found = true;
                    word_idx += 1;
                    break;
                }
                word_idx += 1;
            }
            if !found {
                all_match = false;
                break;
            }
        }
        if all_match {
            return Some(2); // Score 2: same as substring match
        }
    }

    // For short queries (1-2 chars), don't allow subsequence match
    if query.len() <= 2 {
        return None;
    }

    // Subsequence match for longer queries
    let mut query_chars = query.chars().peekable();
    for target_char in target.chars() {
        if let Some(&query_char) = query_chars.peek() {
            if target_char == query_char {
                query_chars.next();
            }
        }
        if query_chars.peek().is_none() {
            return Some(3);
        }
    }

    None
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
                            FocusedPane::DeckSummary => state.deck_select_previous(),
                        },
                        KeyCode::Down => match state.focused_pane {
                            FocusedPane::Search => state.select_next(),
                            FocusedPane::DeckSummary => state.deck_select_next(),
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

/// Column width for card display: "▶ 3 CardName..." = cursor(2) + count(1) + space(1) + name(CARD_NAME_WIDTH)
const CARD_NAME_WIDTH: usize = 26;
const CARD_COLUMN_WIDTH: usize = 2 + 1 + 1 + CARD_NAME_WIDTH + 2; // "▶ 3 CardName...  "

/// Calculate the height needed for the deck summary based on content
fn calculate_deck_summary_height(state: &DeckBuilderState, available_width: u16) -> u16 {
    if state.deck.is_empty() {
        return 3; // Minimum: border + "No cards" message
    }

    // Group cards by category (same logic as draw_deck_summary)
    let mut by_category: HashMap<CardCategory, CardEntryGroup<'_>> = HashMap::new();
    for (name, count) in &state.deck {
        let card_def = state.card_definitions.get(name).map(|arc| arc.as_ref());
        let category = card_def
            .map(|c| CardCategory::from_types(&c.types))
            .unwrap_or(CardCategory::Spell);
        by_category.entry(category).or_default().push((name, count, card_def));
    }

    // Calculate number of columns that fit
    let inner_width = available_width.saturating_sub(2) as usize; // Account for borders
    let num_columns = (inner_width / CARD_COLUMN_WIDTH).max(1);

    // Count lines needed:
    // 1 for header (total + mana curve)
    // For each category: 1 for header + ceil(cards / num_columns) for card rows
    let mut total_lines = 1u16; // Header line

    let category_order = [
        CardCategory::Creature,
        CardCategory::Spell,
        CardCategory::Artifact,
        CardCategory::Land,
    ];

    for category in category_order {
        if let Some(entries) = by_category.get(&category) {
            total_lines += 1; // Category header
            let card_rows = entries.len().div_ceil(num_columns);
            total_lines += card_rows as u16;
        }
    }

    // Add 2 for borders, cap at reasonable max
    (total_lines + 2).min(20)
}

/// Draw the TUI
fn draw_ui(f: &mut Frame, state: &mut DeckBuilderState) {
    // Calculate deck summary height dynamically based on content
    let deck_summary_height = calculate_deck_summary_height(state, f.area().width.saturating_sub(2));

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(deck_summary_height), // Deck summary (dynamic)
            Constraint::Length(3),                   // Search input
            Constraint::Min(10),                     // Search results + card details
            Constraint::Length(2),                   // Status/help bar
        ])
        .split(f.area());

    // Store deck summary area for click detection
    state.deck_summary_area = Some(chunks[0]);

    // Deck summary
    draw_deck_summary(f, chunks[0], state);

    // Search input
    draw_search_input(f, chunks[1], state);

    // Update max_results based on available height (accounting for borders)
    let results_pane_height = chunks[2].height.saturating_sub(2) as usize;
    if state.max_results != results_pane_height && results_pane_height > 0 {
        state.max_results = results_pane_height;
        state.update_search(); // Re-run search with new limit
    }

    // Split the results area horizontally: results on left, card details on right
    let results_area = chunks[2];
    let horizontal_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(50), // Search results
            Constraint::Percentage(50), // Card details
        ])
        .split(results_area);

    // Store search results area for click detection
    state.search_results_area = Some(horizontal_chunks[0]);

    // Search results (left side)
    draw_search_results(f, horizontal_chunks[0], state);

    // Card details (right side)
    draw_card_details(f, horizontal_chunks[1], state);

    // Status bar
    draw_status_bar(f, chunks[3], state);

    // Exit dialog overlay
    if state.show_exit_dialog {
        draw_exit_dialog(f);
    }
}

/// Get terminal color for MTG card color(s)
fn mtg_color_to_term(colors: &[MtgColor]) -> Color {
    if colors.is_empty() {
        return Color::Gray; // Colorless
    }
    if colors.len() > 1 {
        return Color::Rgb(218, 165, 32); // Gold for multicolor
    }
    match colors[0] {
        MtgColor::White => Color::Rgb(255, 255, 224), // Light yellow/cream for white
        MtgColor::Blue => Color::Rgb(100, 149, 237),  // Cornflower blue
        MtgColor::Black => Color::Rgb(139, 69, 139),  // Dark magenta for black
        MtgColor::Red => Color::Rgb(220, 20, 60),     // Crimson
        MtgColor::Green => Color::Rgb(34, 139, 34),   // Forest green
        MtgColor::Colorless => Color::Gray,
    }
}

/// Card category for deck summary grouping
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum CardCategory {
    Creature,
    Land,
    Artifact,
    Spell, // Instants, Sorceries, Enchantments, Planeswalkers
}

impl CardCategory {
    fn from_types(types: &[CardType]) -> Self {
        if types.contains(&CardType::Creature) {
            CardCategory::Creature
        } else if types.contains(&CardType::Land) {
            CardCategory::Land
        } else if types.contains(&CardType::Artifact) {
            CardCategory::Artifact
        } else {
            CardCategory::Spell
        }
    }

    fn label(&self) -> &'static str {
        match self {
            CardCategory::Creature => "Creatures",
            CardCategory::Land => "Lands",
            CardCategory::Artifact => "Artifacts",
            CardCategory::Spell => "Spells",
        }
    }
}

/// Sort key for cards: (color_order, -cmc, name)
fn card_sort_key(card: &CardDefinition) -> (u8, i16, String) {
    // Color order: W, U, B, R, G, Colorless, Multicolor
    let color_order = if card.colors.is_empty() {
        5 // Colorless
    } else if card.colors.len() > 1 {
        6 // Multicolor last
    } else {
        match card.colors[0] {
            MtgColor::White => 0,
            MtgColor::Blue => 1,
            MtgColor::Black => 2,
            MtgColor::Red => 3,
            MtgColor::Green => 4,
            MtgColor::Colorless => 5,
        }
    };
    // Negative CMC for descending sort
    let cmc = -(card.mana_cost.cmc() as i16);
    (color_order, cmc, card.name.to_string())
}

fn draw_deck_summary(f: &mut Frame, area: Rect, state: &mut DeckBuilderState) {
    let is_focused = state.focused_pane == FocusedPane::DeckSummary;
    let border_color = if is_focused { Color::Yellow } else { Color::Cyan };
    let title = if is_focused {
        " Deck Summary [focused] "
    } else {
        " Deck Summary "
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let inner = block.inner(area);
    f.render_widget(block, area);

    if state.deck.is_empty() {
        let hint = Paragraph::new("No cards added yet").style(Style::default().fg(Color::DarkGray));
        f.render_widget(hint, inner);
        return;
    }

    // Group cards by category
    let mut by_category: HashMap<CardCategory, CardEntryGroup<'_>> = HashMap::new();

    for (name, count) in &state.deck {
        let card_def = state.card_definitions.get(name).map(|arc| arc.as_ref());
        let category = card_def
            .map(|c| CardCategory::from_types(&c.types))
            .unwrap_or(CardCategory::Spell);
        by_category.entry(category).or_default().push((name, count, card_def));
    }

    // Sort each category by color, then descending CMC
    for entries in by_category.values_mut() {
        entries.sort_by(|a, b| match (a.2, b.2) {
            (Some(ca), Some(cb)) => card_sort_key(ca).cmp(&card_sort_key(cb)),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.0.cmp(b.0),
        });
    }

    let mut lines = Vec::new();

    // Calculate mana curve (CMC distribution, excluding lands)
    let mut cmc_counts: [usize; 8] = [0; 8]; // 0, 1, 2, 3, 4, 5, 6, 7+
    for (name, count) in &state.deck {
        if let Some(card) = state.card_definitions.get(name) {
            // Skip lands for mana curve
            if !card.types.contains(&CardType::Land) {
                let cmc = card.mana_cost.cmc() as usize;
                let bucket = cmc.min(7);
                cmc_counts[bucket] += *count as usize;
            }
        }
    }

    // Header line: total cards + mana curve (bar chart + numeric)
    let total = state.total_cards();
    let unique = state.unique_cards();
    let mut header_spans = vec![
        Span::styled(
            format!("{} cards", total),
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" ("),
        Span::raw(format!("{} unique", unique)),
        Span::raw(")  "),
        Span::styled("Curve: ", Style::default().fg(Color::Cyan)),
    ];

    // Build mana curve bar chart (uninterrupted bars)
    let max_cmc_count = *cmc_counts.iter().max().unwrap_or(&0);
    let mut bar_string = String::new();
    for &count in &cmc_counts {
        let bar_char = if max_cmc_count == 0 {
            '▁'
        } else {
            let height = (count * 8) / max_cmc_count.max(1);
            match height {
                0 => '▁',
                1 => '▂',
                2 => '▃',
                3 => '▄',
                4 => '▅',
                5 => '▆',
                6 => '▇',
                _ => '█',
            }
        };
        bar_string.push(bar_char);
    }
    header_spans.push(Span::styled(bar_string, Style::default().fg(Color::Cyan)));

    // Add numeric counts after bars: 0(0) 1(2) 2(10) ...
    header_spans.push(Span::raw(" "));
    for (cmc, &count) in cmc_counts.iter().enumerate() {
        if count > 0 || cmc <= 5 {
            let label = if cmc == 7 { "7+".to_string() } else { cmc.to_string() };
            header_spans.push(Span::styled(
                format!("{}({}) ", label, count),
                Style::default().fg(Color::DarkGray),
            ));
        }
    }
    lines.push(Line::from(header_spans));

    // Calculate number of columns for multi-column layout
    let inner_width = inner.width as usize;
    let num_columns = (inner_width / CARD_COLUMN_WIDTH).max(1);
    state.deck_num_columns = num_columns; // Store for left/right navigation

    // Categories in order: Creatures, Spells, Artifacts, Lands
    let category_order = [
        CardCategory::Creature,
        CardCategory::Spell,
        CardCategory::Artifact,
        CardCategory::Land,
    ];

    // Track flattened index for cursor display
    let mut flat_index = 0usize;
    let show_cursor = is_focused;

    for category in category_order {
        if let Some(entries) = by_category.get(&category) {
            let cat_count: usize = entries.iter().map(|(_, c, _)| **c as usize).sum();

            // Category header line
            lines.push(Line::from(vec![Span::styled(
                format!("{}: [{}]", category.label(), cat_count),
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
            )]));

            // Arrange cards in columns (N-order: down first, then across)
            let num_cards = entries.len();
            let num_rows = num_cards.div_ceil(num_columns);

            for row in 0..num_rows {
                let mut row_spans = Vec::new();
                for col in 0..num_columns {
                    // N-order index: row + col * num_rows
                    let idx = row + col * num_rows;
                    if idx < num_cards {
                        let (name, count, card_def) = &entries[idx];
                        let color = card_def.map(|c| mtg_color_to_term(&c.colors)).unwrap_or(Color::White);

                        // Show cursor if this is the selected card
                        let current_flat_idx = flat_index + idx;
                        let cursor = if show_cursor && current_flat_idx == state.deck_selected_index {
                            "▶ "
                        } else {
                            "  "
                        };

                        // Format: "▶ 3 CardName..." padded to column width
                        let card_text = format!("{}{} {}", cursor, count, truncate_name(name, CARD_NAME_WIDTH));
                        let padded = format!("{:width$}", card_text, width = CARD_COLUMN_WIDTH);
                        row_spans.push(Span::styled(padded, Style::default().fg(color)));
                    }
                }
                if !row_spans.is_empty() {
                    lines.push(Line::from(row_spans));
                }
            }

            flat_index += num_cards;
        }
    }

    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, inner);
}

/// Truncate a card name to max_len characters
fn truncate_name(name: &str, max_len: usize) -> String {
    if name.len() <= max_len {
        name.to_string()
    } else {
        format!("{}…", &name[..max_len - 1])
    }
}

fn draw_search_input(f: &mut Frame, area: Rect, state: &DeckBuilderState) {
    let block = Block::default()
        .title(" Search Cards ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Green));

    let inner = block.inner(area);
    f.render_widget(block, area);

    // Show search query with cursor
    let display_text = format!("{}_", state.search_query);
    let paragraph = Paragraph::new(display_text).style(Style::default().fg(Color::White));
    f.render_widget(paragraph, inner);
}

fn draw_search_results(f: &mut Frame, area: Rect, state: &DeckBuilderState) {
    let is_focused = state.focused_pane == FocusedPane::Search;
    let border_color = if is_focused { Color::Yellow } else { Color::Blue };

    // Build title with hit count status
    let title = if state.search_results.is_empty() {
        if is_focused {
            " Results [focused] (↑↓ navigate, Enter/1-9 add) ".to_string()
        } else {
            " Results (↑↓ navigate, Enter/1-9 add) ".to_string()
        }
    } else {
        let total = state.search_results.len();
        let start = state.scroll_offset + 1;
        let end = (state.scroll_offset + state.max_results).min(total);
        let focus_str = if is_focused { "[focused] " } else { "" };
        format!(" Results {focus_str}({total} hits, {start}-{end} shown) ")
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let inner = block.inner(area);
    f.render_widget(block, area);

    if state.search_query.is_empty() {
        let hint = Paragraph::new("Start typing to search...").style(Style::default().fg(Color::DarkGray));
        f.render_widget(hint, inner);
        return;
    }

    if state.search_results.is_empty() {
        let no_results = Paragraph::new("No matching cards found").style(Style::default().fg(Color::Red));
        f.render_widget(no_results, inner);
        return;
    }

    // Apply pagination: only show items from scroll_offset to scroll_offset + max_results
    let visible_end = (state.scroll_offset + state.max_results).min(state.search_results.len());
    let items: Vec<ListItem> = state.search_results[state.scroll_offset..visible_end]
        .iter()
        .enumerate()
        .map(|(visible_i, &card_idx)| {
            let actual_i = state.scroll_offset + visible_i;
            let card_name = &state.all_cards[card_idx];
            let in_deck = state.deck.get(card_name).copied().unwrap_or(0);

            let style = if actual_i == state.selected_index {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };

            let prefix = if actual_i == state.selected_index { "▶ " } else { "  " };

            let text = if in_deck > 0 {
                format!("{}{} ({}x)", prefix, card_name, in_deck)
            } else {
                format!("{}{}", prefix, card_name)
            };

            ListItem::new(text).style(style)
        })
        .collect();

    let list = List::new(items);
    f.render_widget(list, inner);
}

/// Draw card details panel (reuses logic from fancy_tui_renderer)
fn draw_card_details(f: &mut Frame, area: Rect, state: &DeckBuilderState) {
    let block = Block::default()
        .title(" Card Details ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Magenta));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let selected_name = state.get_active_selected_card();
    let card_def = selected_name
        .as_ref()
        .and_then(|name| state.card_definitions.get(name))
        .map(|arc| arc.as_ref());

    if let Some(card) = card_def {
        let mut lines = Vec::new();

        // Card name
        lines.push(Line::from(Span::styled(
            card.name.to_string(),
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        )));

        // Mana cost
        if card.mana_cost.cmc() > 0 {
            lines.push(Line::from(vec![
                Span::raw("Cost: "),
                Span::styled(format!("{}", card.mana_cost), Style::default().fg(Color::Cyan)),
            ]));
        }

        // Type line
        let type_names: Vec<String> = card.types.iter().map(|t| format!("{:?}", t)).collect();
        let subtype_names: Vec<String> = card.subtypes.iter().map(|s| s.as_str().to_string()).collect();
        let type_line = if subtype_names.is_empty() {
            type_names.join(" ")
        } else {
            format!("{} — {}", type_names.join(" "), subtype_names.join(" "))
        };
        lines.push(Line::from(vec![
            Span::raw("Type: "),
            Span::styled(type_line, Style::default().fg(Color::White)),
        ]));

        // P/T for creatures
        if let (Some(power), Some(toughness)) = (card.power, card.toughness) {
            lines.push(Line::from(vec![
                Span::raw("P/T: "),
                Span::styled(format!("{}/{}", power, toughness), Style::default().fg(Color::Green)),
            ]));
        }

        // Set codes and years (from edition index) - deduplicated
        if let Some(ref edition_index) = state.edition_index {
            if let Some(printings) = edition_index.get_card_printings(card.name.as_str()) {
                if !printings.is_empty() {
                    // Deduplicate set codes while preserving order
                    let mut seen_sets = std::collections::HashSet::new();
                    let set_codes: Vec<&str> = printings
                        .iter()
                        .filter_map(|p| {
                            if seen_sets.insert(p.set_code.as_str()) {
                                Some(p.set_code.as_str())
                            } else {
                                None
                            }
                        })
                        .collect();

                    // Deduplicate years while preserving order
                    let mut seen_years = std::collections::HashSet::new();
                    let years: Vec<String> = printings
                        .iter()
                        .filter_map(|p| {
                            if seen_years.insert(p.year) {
                                Some(p.year.to_string())
                            } else {
                                None
                            }
                        })
                        .collect();

                    lines.push(Line::from(vec![
                        Span::raw("Sets: "),
                        Span::styled(set_codes.join(", "), Style::default().fg(Color::DarkGray)),
                    ]));
                    lines.push(Line::from(vec![
                        Span::raw("Years: "),
                        Span::styled(years.join(", "), Style::default().fg(Color::DarkGray)),
                    ]));
                }
            }
        }

        // Oracle text
        if !card.oracle.is_empty() {
            lines.push(Line::from(""));
            for oracle_line in card.oracle.split('\n') {
                lines.push(Line::from(Span::styled(
                    oracle_line.to_string(),
                    Style::default().fg(Color::White),
                )));
            }
        }

        let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
        f.render_widget(paragraph, inner);
    } else if selected_name.is_some() {
        // Card selected but no definition available
        let hint = Paragraph::new("Card details not loaded")
            .style(Style::default().fg(Color::DarkGray))
            .wrap(Wrap { trim: false });
        f.render_widget(hint, inner);
    } else {
        let hint = Paragraph::new("Select a card to view details")
            .style(Style::default().fg(Color::DarkGray))
            .wrap(Wrap { trim: false });
        f.render_widget(hint, inner);
    }
}

fn draw_status_bar(f: &mut Frame, area: Rect, state: &DeckBuilderState) {
    let status = if let Some(ref msg) = state.status_message {
        Line::from(vec![
            Span::styled("✓ ", Style::default().fg(Color::Green)),
            Span::styled(msg.as_str(), Style::default().fg(Color::Green)),
        ])
    } else {
        Line::from(vec![
            Span::styled("Tab", Style::default().fg(Color::Yellow)),
            Span::raw(" focus  "),
            Span::styled("ESC", Style::default().fg(Color::Yellow)),
            Span::raw(" clear/exit  "),
            Span::styled("Enter", Style::default().fg(Color::Yellow)),
            Span::raw(" add  "),
            Span::styled("1-9", Style::default().fg(Color::Yellow)),
            Span::raw(" add N  "),
            Span::styled("Del", Style::default().fg(Color::Yellow)),
            Span::raw(" remove"),
        ])
    };

    let paragraph = Paragraph::new(status).style(Style::default().fg(Color::DarkGray));
    f.render_widget(paragraph, area);
}

fn draw_exit_dialog(f: &mut Frame) {
    let area = centered_rect(40, 20, f.area());

    // Clear background
    f.render_widget(Clear, area);

    let block = Block::default()
        .title(" Save Deck? ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let text = vec![
        Line::raw(""),
        Line::from(vec![
            Span::styled("[Y]", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
            Span::raw(" Save and exit"),
        ]),
        Line::raw(""),
        Line::from(vec![
            Span::styled("[Q]", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
            Span::raw(" Quit without saving"),
        ]),
        Line::raw(""),
        Line::from(vec![
            Span::styled("[N]", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::raw(" Cancel (go back)"),
        ]),
    ];

    let paragraph = Paragraph::new(text);
    f.render_widget(paragraph, inner);
}

/// Create a centered rectangle
fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

/// Save deck to file in .dck format (uses shared DeckList::save_to_file)
fn save_deck(state: &DeckBuilderState, output_file: &str) -> Result<()> {
    let deck_list = state.to_deck_list();
    deck_list.save_to_file(std::path::Path::new(output_file), None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_match_score() {
        // Prefix match is highest priority (score 0)
        assert_eq!(match_score("lightning bolt", "light"), Some(0));
        assert_eq!(match_score("lightning bolt", "lightning"), Some(0));

        // Word-start match (score 1)
        assert_eq!(match_score("lightning bolt", "bolt"), Some(1));

        // Substring match (score 2)
        assert_eq!(match_score("lightning bolt", "ning"), Some(2));
        assert_eq!(match_score("lightning bolt", "olt"), Some(2));

        // Subsequence match for 3+ chars (score 3)
        assert_eq!(match_score("lightning bolt", "lbolt"), Some(3));
        assert_eq!(match_score("lightning bolt", "lgb"), Some(3));

        // No match
        assert_eq!(match_score("lightning bolt", "xyz"), None);

        // Short queries (1-2 chars) only allow substring, not subsequence
        assert_eq!(match_score("lightning bolt", "li"), Some(0)); // prefix
        assert_eq!(match_score("lightning bolt", "bo"), Some(1)); // word-start
        assert_eq!(match_score("lightning bolt", "tn"), Some(2)); // substring
        assert_eq!(match_score("lightning bolt", "lb"), None); // subsequence not allowed for short
    }

    #[test]
    fn test_deck_builder_state() {
        let cards = vec![
            "Lightning Bolt".to_string(),
            "Lightning Helix".to_string(),
            "Grizzly Bears".to_string(),
        ];

        let mut state = DeckBuilderState::new(cards, HashMap::new(), None);

        // Initially no results
        assert!(state.search_results.is_empty());

        // Search for "light" - should get prefix matches
        state.search_query = "light".to_string();
        state.update_search();
        assert_eq!(state.search_results.len(), 2);

        // Add cards
        state.add_selected(4);
        assert_eq!(state.deck.get("Lightning Bolt"), Some(&4));

        // Total count
        assert_eq!(state.total_cards(), 4);
        assert_eq!(state.unique_cards(), 1);

        // Remove one copy
        state.remove_selected();
        assert_eq!(state.deck.get("Lightning Bolt"), Some(&3));
        assert_eq!(state.total_cards(), 3);

        // Remove remaining copies
        state.remove_selected();
        state.remove_selected();
        state.remove_selected();
        assert_eq!(state.deck.get("Lightning Bolt"), None);
        assert_eq!(state.total_cards(), 0);
        assert_eq!(state.unique_cards(), 0);

        // Removing from empty deck should do nothing
        state.remove_selected();
        assert_eq!(state.total_cards(), 0);
    }
}
