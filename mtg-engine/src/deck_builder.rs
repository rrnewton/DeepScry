//! Fast Deck Entry Mode - Interactive TUI for rapid deck building
//!
//! Provides a streamlined interface for entering paper decks with minimal keystrokes:
//! - Fuzzy search with real-time preview (prefix matches prioritized)
//! - Number keys (1-9) to add multiple copies
//! - Enter to add single copy
//! - Arrow keys to navigate results
//! - First ESC clears search, second ESC saves and exits

use crate::loader::{CardDefinition, DeckEntry, DeckList};
use crate::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
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
use std::sync::Arc;

/// Deck builder configuration
pub struct DeckBuilderConfig {
    /// Output file path
    pub output_file: String,
    /// Only include cards from sets released on or after this year
    pub start_year: Option<u16>,
    /// Only include cards from sets released on or before this year
    pub end_year: Option<u16>,
}

impl Default for DeckBuilderConfig {
    fn default() -> Self {
        Self {
            output_file: "output.dck".to_string(),
            start_year: None,
            end_year: None,
        }
    }
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
    /// Currently selected result index
    selected_index: usize,
    /// Current deck: card name -> count
    deck: HashMap<String, u8>,
    /// Whether to show exit confirmation dialog
    show_exit_dialog: bool,
    /// Message to show at bottom (e.g., "Added 4x Lightning Bolt")
    status_message: Option<String>,
    /// Maximum number of results to show (dynamic based on pane height)
    max_results: usize,
}

impl DeckBuilderState {
    fn new(all_cards: Vec<String>, card_definitions: HashMap<String, Arc<CardDefinition>>) -> Self {
        Self {
            all_cards,
            card_definitions,
            search_query: String::new(),
            search_results: Vec::new(),
            selected_index: 0,
            deck: HashMap::new(),
            show_exit_dialog: false,
            status_message: None,
            max_results: 10, // Will be updated based on pane height
        }
    }

    /// Update search results based on current query
    fn update_search(&mut self) {
        if self.search_query.is_empty() {
            self.search_results.clear();
            self.selected_index = 0;
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

        // Take top N results
        self.search_results = scored_results
            .into_iter()
            .take(self.max_results)
            .map(|(idx, _)| idx)
            .collect();

        // Try to follow the previously selected card
        if let Some(prev_idx) = previously_selected {
            if let Some(new_pos) = self.search_results.iter().position(|&idx| idx == prev_idx) {
                self.selected_index = new_pos;
            } else {
                self.selected_index = 0;
            }
        } else {
            self.selected_index = 0;
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

    /// Get the CardDefinition for the selected card (if available)
    fn selected_card_definition(&self) -> Option<&CardDefinition> {
        self.selected_card()
            .and_then(|name| self.card_definitions.get(name))
            .map(|arc| arc.as_ref())
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

    /// Move selection up
    fn select_previous(&mut self) {
        if !self.search_results.is_empty() && self.selected_index > 0 {
            self.selected_index -= 1;
        }
    }

    /// Move selection down
    fn select_next(&mut self) {
        if !self.search_results.is_empty() && self.selected_index < self.search_results.len() - 1 {
            self.selected_index += 1;
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
) -> Result<()> {
    // Filter cards by year if specified (placeholder - we don't have set year data yet)
    let filtered_cards = if config.start_year.is_some() || config.end_year.is_some() {
        // TODO: Filter by set release year when we have that data
        eprintln!(
            "Note: Year filtering not yet implemented, showing all {} cards",
            card_names.len()
        );
        card_names
    } else {
        card_names
    };

    println!("Loaded {} cards for deck building", filtered_cards.len());
    println!("Output will be saved to: {}", config.output_file);
    println!("\nStarting deck builder...\n");

    // Small delay so user can see the message
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    let mut state = DeckBuilderState::new(filtered_cards, card_definitions);

    // Setup terminal
    enable_raw_mode().map_err(crate::MtgError::IoError)?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen).map_err(crate::MtgError::IoError)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).map_err(crate::MtgError::IoError)?;

    // Main loop
    let result = run_main_loop(&mut terminal, &mut state);

    // Restore terminal
    disable_raw_mode().map_err(crate::MtgError::IoError)?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen).map_err(crate::MtgError::IoError)?;
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
            if let Event::Key(key) = event::read().map_err(crate::MtgError::IoError)? {
                // Only process key press events, not release
                if key.kind != KeyEventKind::Press {
                    continue;
                }

                // Handle exit dialog
                if state.show_exit_dialog {
                    match key.code {
                        KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                            return Ok(true); // Save and exit
                        }
                        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                            state.show_exit_dialog = false;
                        }
                        _ => {}
                    }
                    continue;
                }

                // Clear status message on any key
                state.status_message = None;

                match key.code {
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
                    KeyCode::Up => {
                        state.select_previous();
                    }
                    KeyCode::Down => {
                        state.select_next();
                    }
                    KeyCode::Enter => {
                        state.add_selected(1);
                    }
                    KeyCode::Char(c) if c.is_ascii_digit() && c != '0' => {
                        let count = c.to_digit(10).unwrap() as u8;
                        state.add_selected(count);
                    }
                    KeyCode::Char(c) => {
                        state.search_query.push(c);
                        state.update_search();
                    }
                    KeyCode::Backspace => {
                        state.search_query.pop();
                        state.update_search();
                    }
                    _ => {}
                }
            }
        }
    }
}

/// Draw the TUI
fn draw_ui(f: &mut Frame, state: &mut DeckBuilderState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(6), // Deck summary
            Constraint::Length(3), // Search input
            Constraint::Min(10),   // Search results + card details
            Constraint::Length(2), // Status/help bar
        ])
        .split(f.area());

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

fn draw_deck_summary(f: &mut Frame, area: Rect, state: &DeckBuilderState) {
    let block = Block::default()
        .title(" Deck Summary ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let inner = block.inner(area);
    f.render_widget(block, area);

    // Build summary text
    let total = state.total_cards();
    let unique = state.unique_cards();

    let mut lines = vec![Line::from(vec![
        Span::styled(
            format!("{} cards", total),
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" ("),
        Span::raw(format!("{} unique", unique)),
        Span::raw(")"),
    ])];

    // Show last few cards added (up to 3)
    if !state.deck.is_empty() {
        let mut entries: Vec<_> = state.deck.iter().collect();
        entries.sort_by(|a, b| a.0.cmp(b.0));
        let preview_count = entries.len().min(3);
        let mut preview_line = vec![Span::raw("  ")];
        for (i, (name, count)) in entries.iter().take(preview_count).enumerate() {
            if i > 0 {
                preview_line.push(Span::raw(", "));
            }
            preview_line.push(Span::styled(
                format!("{}x {}", count, name),
                Style::default().fg(Color::White),
            ));
        }
        if entries.len() > preview_count {
            preview_line.push(Span::styled(
                format!(" (+{} more)", entries.len() - preview_count),
                Style::default().fg(Color::DarkGray),
            ));
        }
        lines.push(Line::from(preview_line));
    }

    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, inner);
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
    let block = Block::default()
        .title(" Results (↑↓ navigate, Enter/1-9 add) ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Blue));

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

    let items: Vec<ListItem> = state
        .search_results
        .iter()
        .enumerate()
        .map(|(i, &card_idx)| {
            let card_name = &state.all_cards[card_idx];
            let in_deck = state.deck.get(card_name).copied().unwrap_or(0);

            let style = if i == state.selected_index {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };

            let prefix = if i == state.selected_index { "▶ " } else { "  " };

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

    if let Some(card) = state.selected_card_definition() {
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
    } else if state.selected_card().is_some() {
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
            Span::styled("ESC", Style::default().fg(Color::Yellow)),
            Span::raw(" clear/exit  "),
            Span::styled("Ctrl+C", Style::default().fg(Color::Yellow)),
            Span::raw(" quit  "),
            Span::styled("Enter", Style::default().fg(Color::Yellow)),
            Span::raw(" add 1  "),
            Span::styled("1-9", Style::default().fg(Color::Yellow)),
            Span::raw(" add N"),
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
            Span::styled("[N]", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
            Span::raw(" Cancel"),
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

        let mut state = DeckBuilderState::new(cards, HashMap::new());

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
    }
}
