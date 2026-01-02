//! Shared deck builder UI rendering
//!
//! This module contains the rendering logic using ratatui that works with
//! both native (crossterm) and WASM (ratzilla) backends.

use super::state::{
    card_sort_key, mtg_color_to_term, truncate_name, CardCategory, CardEntryGroup, DeckBuilderState, FocusedPane,
    CARD_COLUMN_WIDTH, CARD_NAME_WIDTH,
};
use crate::core::CardType;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
    Frame,
};
use std::collections::HashMap;

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

/// Draw the TUI (public for WASM rendering)
pub fn draw_ui(f: &mut Frame, state: &mut DeckBuilderState) {
    // Calculate deck summary height dynamically based on content
    let deck_summary_height = calculate_deck_summary_height(state, f.area().width.saturating_sub(2));

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(deck_summary_height), // Deck summary (dynamic)
            Constraint::Length(3),                   // Search input
            Constraint::Min(10),                     // Search results + card details + problems
            Constraint::Length(2),                   // Status/help bar
        ])
        .split(f.area());

    // Store deck summary area for click detection
    state.deck_summary_area = Some(chunks[0]);

    // Deck summary
    draw_deck_summary(f, chunks[0], state);

    // Search input - store area for click detection
    state.search_input_area = Some(chunks[1]);
    draw_search_input(f, chunks[1], state);

    // Update max_results based on available height (accounting for borders)
    let results_pane_height = chunks[2].height.saturating_sub(2) as usize;
    if state.max_results != results_pane_height && results_pane_height > 0 {
        state.max_results = results_pane_height;
        state.update_search(); // Re-run search with new limit
    }

    // Split the results area horizontally
    // If we have problems, show 3 panes: results | details | problems
    // Otherwise, show 2 panes: results | details
    let results_area = chunks[2];
    let horizontal_chunks = if state.has_problems() {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(35), // Search results
                Constraint::Percentage(35), // Card details
                Constraint::Percentage(30), // Problems
            ])
            .split(results_area)
    } else {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(50), // Search results
                Constraint::Percentage(50), // Card details
            ])
            .split(results_area)
    };

    // Store search results area for click detection
    state.search_results_area = Some(horizontal_chunks[0]);

    // Search results (left side)
    draw_search_results(f, horizontal_chunks[0], state);

    // Card details (middle or right side)
    draw_card_details(f, horizontal_chunks[1], state);

    // Problems pane (right side, only if there are problems)
    if state.has_problems() {
        state.problems_area = Some(horizontal_chunks[2]);
        draw_problems_pane(f, horizontal_chunks[2], state);
    } else {
        state.problems_area = None;
    }

    // Status bar
    draw_status_bar(f, chunks[3], state);

    // Exit dialog overlay
    if state.show_exit_dialog {
        draw_exit_dialog(f);
    }
}

fn draw_deck_summary(f: &mut Frame, area: Rect, state: &mut DeckBuilderState) {
    let is_focused = state.focused_pane == FocusedPane::DeckSummary;
    let border_color = if is_focused { Color::Yellow } else { Color::Cyan };

    // Build title with optional deck name
    let title = match (&state.deck_name, is_focused) {
        (Some(name), true) => format!(" Deck Summary: {} [focused] ", name),
        (Some(name), false) => format!(" Deck Summary: {} ", name),
        (None, true) => " Deck Summary [focused] ".to_string(),
        (None, false) => " Deck Summary ".to_string(),
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
            '\u{2581}'
        } else {
            let height = (count * 8) / max_cmc_count.max(1);
            match height {
                0 => '\u{2581}',
                1 => '\u{2582}',
                2 => '\u{2583}',
                3 => '\u{2584}',
                4 => '\u{2585}',
                5 => '\u{2586}',
                6 => '\u{2587}',
                _ => '\u{2588}',
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
                            "\u{25B6} "
                        } else {
                            "  "
                        };

                        // Format: "  3 CardName..." padded to column width
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
            " Results [focused] (Up/Down navigate, Enter/1-9 add) ".to_string()
        } else {
            " Results (Up/Down navigate, Enter/1-9 add) ".to_string()
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

            let is_selected = actual_i == state.selected_index;
            let style = if is_selected {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };

            // Only show cursor arrow when this pane is focused
            let prefix = if is_selected && is_focused { "\u{25B6} " } else { "  " };

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
            format!("{} - {}", type_names.join(" "), subtype_names.join(" "))
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

        // Set codes and years (from edition index) - native only
        #[cfg(feature = "native")]
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

/// Draw the import problems pane (repair mode)
fn draw_problems_pane(f: &mut Frame, area: Rect, state: &DeckBuilderState) {
    let is_focused = state.focused_pane == FocusedPane::Problems;
    let border_color = if is_focused { Color::Yellow } else { Color::Red };

    let title = if is_focused {
        format!(" REMAINING PROBLEMS [focused] ({}) ", state.import_problems.len())
    } else {
        format!(" REMAINING PROBLEMS ({}) ", state.import_problems.len())
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let inner = block.inner(area);
    f.render_widget(block, area);

    if state.import_problems.is_empty() {
        let hint = Paragraph::new("No problems remaining!").style(Style::default().fg(Color::Green));
        f.render_widget(hint, inner);
        return;
    }

    // Calculate visible range
    let visible_height = inner.height as usize;
    let visible_end = (state.problems_scroll_offset + visible_height).min(state.import_problems.len());

    let items: Vec<ListItem> = state.import_problems[state.problems_scroll_offset..visible_end]
        .iter()
        .enumerate()
        .map(|(visible_i, problem)| {
            let actual_i = state.problems_scroll_offset + visible_i;
            let is_selected = actual_i == state.problems_selected_index;

            let style = if is_selected {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };

            let prefix = if is_selected { "\u{25B6} " } else { "  " };
            let label = problem.label();

            // Truncate label if too long for the pane
            let max_len = inner.width.saturating_sub(3) as usize;
            let display_text = if label.len() > max_len && max_len > 3 {
                format!("{}{}...", prefix, &label[..max_len - 3])
            } else {
                format!("{}{}", prefix, label)
            };

            ListItem::new(display_text).style(style)
        })
        .collect();

    let list = List::new(items);
    f.render_widget(list, inner);
}

fn draw_status_bar(f: &mut Frame, area: Rect, state: &DeckBuilderState) {
    let status = if let Some(ref msg) = state.status_message {
        Line::from(vec![
            Span::styled("\u{2713} ", Style::default().fg(Color::Green)),
            Span::styled(msg.as_str(), Style::default().fg(Color::Green)),
        ])
    } else if state.focused_pane == FocusedPane::Problems {
        // Context-sensitive hints for Problems pane
        Line::from(vec![
            Span::styled("Tab", Style::default().fg(Color::Yellow)),
            Span::raw(" focus  "),
            Span::styled("Del/Bksp", Style::default().fg(Color::Yellow)),
            Span::raw(" dismiss problem  "),
            Span::styled("\u{2191}\u{2193}", Style::default().fg(Color::Yellow)),
            Span::raw(" navigate"),
        ])
    } else {
        // Default hints for Search/DeckSummary
        Line::from(vec![
            Span::styled("Tab", Style::default().fg(Color::Yellow)),
            Span::raw(" focus  "),
            Span::styled("ESC", Style::default().fg(Color::Yellow)),
            Span::raw(" clear/exit  "),
            Span::styled("Enter", Style::default().fg(Color::Yellow)),
            Span::raw(" +1  "),
            Span::styled("1-9", Style::default().fg(Color::Yellow)),
            Span::raw(" set N  "),
            Span::styled("Del/Bksp", Style::default().fg(Color::Yellow)),
            Span::raw(" -1"),
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
