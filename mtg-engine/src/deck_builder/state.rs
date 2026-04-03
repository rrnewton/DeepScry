//! Shared deck builder state and logic
//!
//! This module contains the core state management for the deck builder that is
//! shared between native (crossterm) and WASM (ratzilla) implementations.
//! It has no platform-specific dependencies.

use crate::core::{CardType, Color as MtgColor};
#[cfg(feature = "native")]
use crate::loader::CardEditionIndex;
use crate::loader::{CardDefinition, DeckEntry, DeckList, ImportProblem};
use ratatui::layout::Rect;
use std::collections::HashMap;
use std::sync::Arc;

/// Type alias for grouped card entries: (card_name, count, optional_definition)
pub type CardEntryGroup<'a> = Vec<(&'a String, &'a u8, Option<&'a CardDefinition>)>;

/// Which pane currently has keyboard focus
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusedPane {
    Search,
    DeckSummary,
    /// Problems pane (only shown when there are import problems)
    Problems,
}

/// State for the deck builder TUI
///
/// This struct is public to allow sharing between native and WASM implementations.
/// The core state management is identical; only the event loop differs.
pub struct DeckBuilderState {
    /// Optional name/title of the deck being edited
    pub deck_name: Option<String>,
    /// All available card names (sorted)
    pub all_cards: Vec<String>,
    /// Card definitions cache (loaded on demand for details display)
    pub card_definitions: HashMap<String, Arc<CardDefinition>>,
    /// Current search query
    pub search_query: String,
    /// Filtered search results (indices into all_cards)
    pub search_results: Vec<usize>,
    /// Currently selected result index in search results
    pub selected_index: usize,
    /// First visible result index (for pagination)
    pub scroll_offset: usize,
    /// Current deck: card name -> count
    pub deck: HashMap<String, u8>,
    /// Whether to show exit confirmation dialog
    pub show_exit_dialog: bool,
    /// Message to show at bottom (e.g., "Added 4x Lightning Bolt")
    pub status_message: Option<String>,
    /// Maximum number of results to show (dynamic based on pane height)
    pub max_results: usize,
    /// Which pane has keyboard focus
    pub focused_pane: FocusedPane,
    /// Rect areas for click detection (set during draw)
    pub deck_summary_area: Option<Rect>,
    pub search_input_area: Option<Rect>,
    pub search_results_area: Option<Rect>,
    /// Selected card index in deck summary (flattened across all categories)
    pub deck_selected_index: usize,
    /// Edition index for showing card release info (optional, native only)
    #[cfg(feature = "native")]
    pub edition_index: Option<CardEditionIndex>,
    /// Number of columns in deck summary (updated during draw)
    pub deck_num_columns: usize,
    /// Dirty flag for WASM: set to true when state changes, cleared after draw
    pub needs_redraw: bool,

    // --- Import problems (repair mode) ---
    /// List of import problems (parse failures, missing cards)
    pub import_problems: Vec<ImportProblem>,
    /// Selected problem index in the problems pane
    pub problems_selected_index: usize,
    /// Scroll offset for problems pane
    pub problems_scroll_offset: usize,
    /// Rect area for problems pane (for click detection)
    pub problems_area: Option<Rect>,
}

impl DeckBuilderState {
    /// Create a new deck builder state (native version with edition index)
    #[cfg(feature = "native")]
    pub fn new(
        all_cards: Vec<String>,
        card_definitions: HashMap<String, Arc<CardDefinition>>,
        edition_index: Option<CardEditionIndex>,
    ) -> Self {
        Self {
            deck_name: None,
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
            search_input_area: None,
            search_results_area: None,
            deck_selected_index: 0,
            edition_index,
            deck_num_columns: 1, // Will be updated during draw
            needs_redraw: true,  // Start dirty so first frame draws
            import_problems: Vec::new(),
            problems_selected_index: 0,
            problems_scroll_offset: 0,
            problems_area: None,
        }
    }

    /// Create a new deck builder state (WASM version without edition index)
    #[cfg(not(feature = "native"))]
    pub fn new(all_cards: Vec<String>, card_definitions: HashMap<String, Arc<CardDefinition>>) -> Self {
        Self {
            deck_name: None,
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
            search_input_area: None,
            search_results_area: None,
            deck_selected_index: 0,
            deck_num_columns: 1, // Will be updated during draw
            needs_redraw: true,  // Start dirty so first frame draws
            import_problems: Vec::new(),
            problems_selected_index: 0,
            problems_scroll_offset: 0,
            problems_area: None,
        }
    }

    /// Toggle focus between panes (cycles through available panes)
    pub fn toggle_focus(&mut self) {
        self.focused_pane = match self.focused_pane {
            FocusedPane::Search => {
                if !self.import_problems.is_empty() {
                    FocusedPane::Problems
                } else {
                    FocusedPane::DeckSummary
                }
            }
            FocusedPane::Problems => FocusedPane::DeckSummary,
            FocusedPane::DeckSummary => FocusedPane::Search,
        };
        self.needs_redraw = true;
    }

    /// Check if we have import problems (repair mode active)
    pub fn has_problems(&self) -> bool {
        !self.import_problems.is_empty()
    }

    /// Remove the currently selected problem from the list
    pub fn remove_selected_problem(&mut self) {
        if !self.import_problems.is_empty() && self.problems_selected_index < self.import_problems.len() {
            let removed = self.import_problems.remove(self.problems_selected_index);
            self.status_message = Some(format!("Dismissed: {}", removed.label()));

            // Adjust selection if needed
            if self.problems_selected_index >= self.import_problems.len() && !self.import_problems.is_empty() {
                self.problems_selected_index = self.import_problems.len() - 1;
            }

            // If no more problems, switch focus away from Problems pane
            if self.import_problems.is_empty() && self.focused_pane == FocusedPane::Problems {
                self.focused_pane = FocusedPane::Search;
            }

            self.needs_redraw = true;
        }
    }

    /// Move problems selection up
    pub fn problems_select_previous(&mut self) {
        if !self.import_problems.is_empty() && self.problems_selected_index > 0 {
            self.problems_selected_index -= 1;
            // Keep selected item visible
            if self.problems_selected_index < self.problems_scroll_offset {
                self.problems_scroll_offset = self.problems_selected_index;
            }
            self.needs_redraw = true;
        }
    }

    /// Move problems selection down
    pub fn problems_select_next(&mut self) {
        if !self.import_problems.is_empty() && self.problems_selected_index < self.import_problems.len() - 1 {
            self.problems_selected_index += 1;
            // Keep selected item visible (assuming max_results height for problems too)
            if self.problems_selected_index >= self.problems_scroll_offset + self.max_results {
                self.problems_scroll_offset = self.problems_selected_index.saturating_sub(self.max_results - 1);
            }
            self.needs_redraw = true;
        }
    }

    /// Add import problems to the state
    pub fn set_import_problems(&mut self, problems: Vec<ImportProblem>) {
        self.import_problems = problems;
        self.problems_selected_index = 0;
        self.problems_scroll_offset = 0;
        // If there are problems, start with focus on the problems pane
        if !self.import_problems.is_empty() {
            self.focused_pane = FocusedPane::Problems;
        }
        self.needs_redraw = true;
    }

    /// Update search results based on current query
    pub fn update_search(&mut self) {
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
    pub fn selected_card(&self) -> Option<&str> {
        if self.search_results.is_empty() {
            return None;
        }
        let idx = self.search_results.get(self.selected_index)?;
        Some(&self.all_cards[*idx])
    }

    /// Add copies of the selected card to the deck (for Enter key - adds 1)
    pub fn add_selected(&mut self, count: u8) {
        if let Some(card_name) = self.selected_card() {
            let card_name = card_name.to_string();
            let entry = self.deck.entry(card_name.clone()).or_insert(0);
            *entry = entry.saturating_add(count);
            self.status_message = Some(format!("Added {}x {}", count, card_name));
            self.needs_redraw = true;
        }
    }

    /// Set the count of the selected card to a specific value (for number keys)
    pub fn set_selected(&mut self, count: u8) {
        if let Some(card_name) = self.selected_card() {
            let card_name = card_name.to_string();
            self.deck.insert(card_name.clone(), count);
            self.status_message = Some(format!("Set {}x {}", count, card_name));
            self.needs_redraw = true;
        }
    }

    /// Set the count of the selected deck card (when DeckSummary is focused)
    pub fn set_deck_selected(&mut self, count: u8) {
        let ordered = self.get_deck_cards_ordered();
        if let Some(card_name) = ordered.get(self.deck_selected_index).cloned() {
            if count == 0 {
                self.deck.remove(&card_name);
                self.status_message = Some(format!("Removed {} from deck", card_name));
                // Adjust selection if needed
                let new_ordered = self.get_deck_cards_ordered();
                if self.deck_selected_index >= new_ordered.len() && !new_ordered.is_empty() {
                    self.deck_selected_index = new_ordered.len() - 1;
                }
            } else {
                self.deck.insert(card_name.clone(), count);
                self.status_message = Some(format!("Set {}x {}", count, card_name));
            }
            self.needs_redraw = true;
        }
    }

    /// Increment the count of the selected deck card by 1 (Enter key in DeckSummary)
    pub fn increment_deck_selected(&mut self) {
        let ordered = self.get_deck_cards_ordered();
        if let Some(card_name) = ordered.get(self.deck_selected_index).cloned() {
            let entry = self.deck.entry(card_name.clone()).or_insert(0);
            *entry = entry.saturating_add(1);
            self.status_message = Some(format!("Added 1x {} (now {}x)", card_name, *entry));
            self.needs_redraw = true;
        }
    }

    /// Remove one copy of the selected card from the deck
    /// Uses the focused pane's selection (Search or DeckSummary)
    pub fn remove_selected(&mut self) {
        if let Some(card_name) = self.get_active_selected_card() {
            if let Some(count) = self.deck.get_mut(&card_name) {
                if *count > 1 {
                    *count -= 1;
                    self.status_message = Some(format!("Removed 1x {} ({}x remaining)", card_name, *count));
                } else {
                    self.deck.remove(&card_name);
                    self.status_message = Some(format!("Removed {} from deck", card_name));
                    // Adjust deck_selected_index if needed after removal
                    let ordered = self.get_deck_cards_ordered();
                    if self.deck_selected_index >= ordered.len() && !ordered.is_empty() {
                        self.deck_selected_index = ordered.len() - 1;
                    }
                }
                self.needs_redraw = true;
            }
        }
    }

    /// Move selection up
    pub fn select_previous(&mut self) {
        if !self.search_results.is_empty() && self.selected_index > 0 {
            self.selected_index -= 1;
            // Keep selected item visible
            if self.selected_index < self.scroll_offset {
                self.scroll_offset = self.selected_index;
            }
            self.needs_redraw = true;
        }
    }

    /// Move selection down
    pub fn select_next(&mut self) {
        if !self.search_results.is_empty() && self.selected_index < self.search_results.len() - 1 {
            self.selected_index += 1;
            // Keep selected item visible
            if self.selected_index >= self.scroll_offset + self.max_results {
                self.scroll_offset = self.selected_index.saturating_sub(self.max_results - 1);
            }
            self.needs_redraw = true;
        }
    }

    /// Get total card count in deck
    pub fn total_cards(&self) -> usize {
        self.deck.values().map(|&c| c as usize).sum()
    }

    /// Get unique card count in deck
    pub fn unique_cards(&self) -> usize {
        self.deck.len()
    }

    /// Build DeckList from current state
    pub fn to_deck_list(&self) -> DeckList {
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
            commanders: Vec::new(),
        }
    }

    /// Get flattened list of deck card names in display order (by category, then sorted within)
    pub fn get_deck_cards_ordered(&self) -> Vec<String> {
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
    pub fn get_active_selected_card(&self) -> Option<String> {
        match self.focused_pane {
            FocusedPane::Search => self.selected_card().map(|s| s.to_string()),
            FocusedPane::DeckSummary => {
                let ordered = self.get_deck_cards_ordered();
                ordered.get(self.deck_selected_index).cloned()
            }
            FocusedPane::Problems => {
                // For Problems pane, return the card name if it's a CardMissing problem
                self.import_problems
                    .get(self.problems_selected_index)
                    .and_then(|p| p.card_name.clone())
            }
        }
    }

    /// Move deck selection up (vertical navigation within column)
    pub fn deck_select_previous(&mut self, num_columns: usize) {
        if self.deck.is_empty() || num_columns == 0 {
            return;
        }

        let category_info = self.get_category_layout_info(num_columns);
        if let Some((cat_idx, cat_start, _cat_size, num_rows)) = self.find_category_for_index_with_idx(&category_info) {
            let local_idx = self.deck_selected_index - cat_start;
            let row = local_idx % num_rows;
            let col = local_idx / num_rows;

            if row > 0 {
                // Move up within same category
                self.deck_selected_index = cat_start + (row - 1) + col * num_rows;
            } else if cat_idx > 0 {
                // Move to previous category, same column, last row
                let (prev_start, prev_size, prev_num_rows) = category_info[cat_idx - 1];
                let prev_num_cols = prev_size.div_ceil(prev_num_rows);
                // Clamp column to what's available in previous category
                let target_col = col.min(prev_num_cols.saturating_sub(1));
                // Go to last row of that column
                let last_row_in_col = if target_col == prev_num_cols - 1 {
                    // Last column may be partial
                    (prev_size - 1) % prev_num_rows
                } else {
                    prev_num_rows - 1
                };
                let target_idx = prev_start + last_row_in_col + target_col * prev_num_rows;
                if target_idx < prev_start + prev_size {
                    self.deck_selected_index = target_idx;
                }
            }
            // If already at top of first category, stay put
        }
    }

    /// Move deck selection down (vertical navigation within column)
    pub fn deck_select_next(&mut self, num_columns: usize) {
        if self.deck.is_empty() || num_columns == 0 {
            return;
        }

        let category_info = self.get_category_layout_info(num_columns);

        if let Some((cat_idx, cat_start, cat_size, num_rows)) = self.find_category_for_index_with_idx(&category_info) {
            let local_idx = self.deck_selected_index - cat_start;
            let row = local_idx % num_rows;
            let col = local_idx / num_rows;

            // Check if there's a card below in the same column
            let next_row_idx = cat_start + (row + 1) + col * num_rows;
            if row + 1 < num_rows && next_row_idx < cat_start + cat_size {
                // Move down within same category
                self.deck_selected_index = next_row_idx;
            } else if cat_idx + 1 < category_info.len() {
                // Move to next category, same column, first row
                let (next_start, next_size, next_num_rows) = category_info[cat_idx + 1];
                let next_num_cols = next_size.div_ceil(next_num_rows);
                // Clamp column to what's available in next category
                let target_col = col.min(next_num_cols.saturating_sub(1));
                let target_idx = next_start + target_col * next_num_rows;
                if target_idx < next_start + next_size {
                    self.deck_selected_index = target_idx;
                }
            } else {
                // At bottom of last category - wrap to next column if possible
                if col + 1 < num_columns {
                    // Find first card in next column (could be in any category)
                    for (start, size, rows) in &category_info {
                        let target_idx = start + (col + 1) * rows;
                        if target_idx < start + size {
                            self.deck_selected_index = target_idx;
                            return;
                        }
                    }
                }
                // If no next column or no card there, stay put
            }
        }
    }

    /// Move deck selection left (to previous column in same row)
    pub fn deck_select_left(&mut self, num_columns: usize) {
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
    pub fn deck_select_right(&mut self, num_columns: usize) {
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
    pub fn get_category_layout_info(&self, num_columns: usize) -> Vec<(usize, usize, usize)> {
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

    /// Find which category contains the current deck_selected_index, with category index
    /// Returns (cat_idx, cat_start, cat_size, num_rows)
    fn find_category_for_index_with_idx(
        &self,
        category_info: &[(usize, usize, usize)],
    ) -> Option<(usize, usize, usize, usize)> {
        for (cat_idx, &(start, size, num_rows)) in category_info.iter().enumerate() {
            if self.deck_selected_index >= start && self.deck_selected_index < start + size {
                return Some((cat_idx, start, size, num_rows));
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
pub fn match_score(target: &str, query: &str) -> Option<u8> {
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

/// Card category for deck summary grouping
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CardCategory {
    Creature,
    Land,
    Artifact,
    Spell, // Instants, Sorceries, Enchantments, Planeswalkers
}

impl CardCategory {
    pub fn from_types(types: &[CardType]) -> Self {
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

    pub fn label(&self) -> &'static str {
        match self {
            CardCategory::Creature => "Creatures",
            CardCategory::Land => "Lands",
            CardCategory::Artifact => "Artifacts",
            CardCategory::Spell => "Spells",
        }
    }
}

/// Sort key for cards: (color_order, -cmc, name)
pub fn card_sort_key(card: &CardDefinition) -> (u8, i16, String) {
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
    let cmc = -i16::from(card.mana_cost.cmc());
    (color_order, cmc, card.name.to_string())
}

/// Get terminal color for MTG card color(s)
pub fn mtg_color_to_term(colors: &[MtgColor]) -> ratatui::style::Color {
    use ratatui::style::Color;

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

/// Truncate a card name to max_len characters
pub fn truncate_name(name: &str, max_len: usize) -> String {
    if name.len() <= max_len {
        name.to_string()
    } else {
        format!("{}...", &name[..max_len - 3])
    }
}

/// Column width for card display: "  3 CardName..." = cursor(2) + count(1) + space(1) + name(CARD_NAME_WIDTH)
pub const CARD_NAME_WIDTH: usize = 26;
pub const CARD_COLUMN_WIDTH: usize = 2 + 1 + 1 + CARD_NAME_WIDTH + 2; // "  3 CardName...  "

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
