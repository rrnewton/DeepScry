//! Fancy TUI controller with full-screen ratatui interface
//!
//! This controller provides a rich, multi-panel TUI interface similar to MTG Arena,
//! with separate panels for battlefield, hand, card details, prompts, and game state.

use crate::core::{CardId, ManaCost, PlayerId, SpellAbility};
use crate::game::controller::{GameStateView, PlayerController};
use crate::game::Step;
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers, MouseButton, MouseEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, List, ListItem, Paragraph, Tabs, Wrap},
    Frame, Terminal,
};
use smallvec::SmallVec;
use std::collections::HashMap;
use std::io;

/// Input action result from user interaction
enum InputAction {
    /// Continue - need to redraw UI (arrow key pressed)
    Continue,
    /// Select a specific choice index
    Select(usize),
    /// Pass/cancel the choice
    Pass,
    /// Exit the game (Ctrl-C pressed)
    Exit,
}

/// Tab indices for left panels (Combat|Log only)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
enum LeftTab {
    Combat = 0,
    Log = 1,
}

/// Currently focused pane for keyboard navigation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FocusedPane {
    /// (H)and pane
    Hand,
    /// (I)nfo pane (Combat/Log)
    Info,
    /// (Y)our battlefield
    YourBattlefield,
    /// (O)pponent battlefield
    OpponentBattlefield,
    /// (A)ctions pane (Prompt - no tabs)
    Actions,
    /// (S)tack pane
    Stack,
}

/// Card position for hit testing during mouse clicks
#[derive(Debug, Clone)]
struct CardPosition {
    card_id: CardId,
    area: Rect,
}

/// Context for what kind of choice is being made
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChoiceContext {
    /// Playing a spell or activating an ability
    PlayingSpell,
    /// Declaring attackers
    DeclareAttackers,
    /// Declaring blockers
    DeclareBlockers,
    /// Selecting a target for a spell/ability
    TargetSelection,
    /// No active choice context
    None,
}

/// Application state for the fancy TUI
struct FancyTuiState {
    /// Currently selected left tab
    left_tab: LeftTab,
    /// Currently highlighted choice index (if in choice mode)
    highlighted_choice: usize,
    /// Currently selected card for details view
    selected_card_id: Option<CardId>,
    /// Whether logger was configured for memory-only mode
    logger_memory_mode_enabled: bool,
    /// Currently focused pane
    focused_pane: FocusedPane,
    /// Selected card index in hand (for navigation)
    selected_card_in_hand: Option<usize>,
    /// Selected card in your battlefield (for navigation)
    selected_card_in_your_bf: Option<CardId>,
    /// Selected card in opponent battlefield (for navigation)
    selected_card_in_opp_bf: Option<CardId>,
    /// Card positions for mouse hit testing (cleared and rebuilt each frame)
    card_positions: Vec<CardPosition>,
    /// Cards that can currently be chosen (for highlighting)
    valid_choices: Vec<CardId>,
    /// What kind of choice is being made
    choice_context: ChoiceContext,
}

impl FancyTuiState {
    fn new() -> Self {
        Self {
            left_tab: LeftTab::Log, // Log is default tab
            highlighted_choice: 0,
            selected_card_id: None,
            logger_memory_mode_enabled: false,
            focused_pane: FocusedPane::Actions, // Start with Actions focused
            selected_card_in_hand: None,
            selected_card_in_your_bf: None,
            selected_card_in_opp_bf: None,
            card_positions: Vec::new(),
            valid_choices: Vec::new(),
            choice_context: ChoiceContext::None,
        }
    }
}

/// A controller that provides a rich TUI interface using ratatui
pub struct FancyTuiController {
    player_id: PlayerId,
    state: FancyTuiState,
}

impl FancyTuiController {
    /// Create a new fancy TUI controller
    pub fn new(player_id: PlayerId) -> io::Result<Self> {
        Ok(FancyTuiController {
            player_id,
            state: FancyTuiState::new(),
        })
    }

    /// Get abbreviated phase name for display
    fn step_abbrev(step: Step) -> &'static str {
        match step {
            Step::Untap => "UP",
            Step::Upkeep => "UK", // Upkeep
            Step::Draw => "DR",
            Step::Main1 => "M1",
            Step::BeginCombat => "BC",
            Step::DeclareAttackers => "DA",
            Step::DeclareBlockers => "DB",
            Step::CombatDamage => "CD",
            Step::EndCombat => "EC",
            Step::Main2 => "M2",
            Step::End => "ET",
            Step::Cleanup => "CL",
        }
    }

    /// Initialize the terminal for TUI mode
    fn setup_terminal() -> io::Result<Terminal<CrosstermBackend<io::Stdout>>> {
        use crossterm::event::EnableMouseCapture;

        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        Terminal::new(backend)
    }

    /// Restore the terminal to normal mode
    fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> io::Result<()> {
        use crossterm::event::DisableMouseCapture;
        use std::io::Write;

        // Restore terminal state in reverse order of setup
        disable_raw_mode()?;
        execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
        terminal.show_cursor()?;

        // Flush all pending operations to ensure terminal is fully restored
        terminal.backend_mut().flush()?;

        Ok(())
    }

    /// Configure game logger for memory-only mode (suppress stdout)
    pub fn configure_logger_for_tui(&mut self, _view: &GameStateView) {
        // The logger is in the GameState which we can't mutate through GameStateView
        // We'll need to do this differently - see implementation note
        self.state.logger_memory_mode_enabled = true;
    }

    /// Save buffered logs to a temp file and print the location
    /// Call this after the game ends and terminal is restored
    pub fn save_logs_on_exit(&self, view: &GameStateView) -> io::Result<()> {
        if !self.state.logger_memory_mode_enabled {
            return Ok(());
        }

        let logs = view.logger().logs();
        let log_count = logs.len();

        if log_count == 0 {
            eprintln!("No game logs captured.");
            return Ok(());
        }

        // Create temp file for logs
        let temp_dir = std::env::temp_dir();
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let log_path = temp_dir.join(format!("mtg_forge_game_{}.log", timestamp));

        // Write logs to file
        use std::io::Write;
        let mut file = std::fs::File::create(&log_path)?;
        for entry in logs.iter() {
            writeln!(file, "{}", entry.message)?;
        }

        eprintln!("\n>>> Game log saved: {} lines written to:", log_count);
        eprintln!("    {}", log_path.display());

        Ok(())
    }

    /// Get all cards for a battlefield in display order (lands, creatures, others)
    fn get_battlefield_cards_in_order(view: &GameStateView, owner_id: PlayerId) -> Vec<CardId> {
        let battlefield = view.battlefield();
        let player_cards: Vec<CardId> = battlefield
            .iter()
            .filter(|&&card_id| {
                view.get_card(card_id)
                    .map(|c| c.controller == owner_id)
                    .unwrap_or(false)
            })
            .copied()
            .collect();

        // Group cards: lands, creatures, other
        let (lands, creatures, others): (Vec<_>, Vec<_>, Vec<_>) = player_cards.iter().fold(
            (Vec::new(), Vec::new(), Vec::new()),
            |(mut lands, mut creatures, mut others), &card_id| {
                if let Some(card) = view.get_card(card_id) {
                    if card.is_land() {
                        lands.push(card_id);
                    } else if card.is_creature() {
                        creatures.push(card_id);
                    } else {
                        others.push(card_id);
                    }
                }
                (lands, creatures, others)
            },
        );

        // Concatenate in display order
        let mut result = Vec::new();
        result.extend(lands);
        result.extend(creatures);
        result.extend(others);
        result
    }

    /// Draw the complete UI with all panels
    fn draw_ui(
        &mut self,
        f: &mut Frame,
        view: &GameStateView,
        current_prompt: Option<&str>,
        choices: &[(String, bool)], // (text, is_highlighted)
    ) {
        // Clear card positions from previous frame
        self.state.card_positions.clear();
        // Main layout: 3 columns
        let main_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(25), // Left panels
                Constraint::Percentage(50), // Battlefields
                Constraint::Percentage(25), // Right panels (Card Details + Hand + Stack)
            ])
            .split(f.area());

        // Left column: split into top tabs (Combat|Log) and bottom Prompt pane
        let left_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(60), // Combat|Log tabs
                Constraint::Percentage(40), // Prompt pane (no tabs)
            ])
            .split(main_chunks[0]);

        // Right column: split into Card Details, Hand, and Stack (33% each)
        let right_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(33), // Card Details
                Constraint::Percentage(33), // Hand
                Constraint::Percentage(34), // Stack (34% to account for rounding)
            ])
            .split(main_chunks[2]);

        // Draw all panels
        self.draw_left_tabs(f, left_chunks[0], view);
        self.draw_prompt(f, left_chunks[1], view, current_prompt, choices);
        self.draw_battlefields(f, main_chunks[1], view);
        self.draw_card_details(f, right_chunks[0], view);
        self.draw_hand(f, right_chunks[1], view);
        self.draw_stack(f, right_chunks[2], view);
    }

    /// Draw the left tabbed panel (Stack|Combat|Log)
    fn draw_left_tabs(&self, f: &mut Frame, area: Rect, view: &GameStateView) {
        // Determine focus state
        let is_focused = self.state.focused_pane == FocusedPane::Info;
        let border_color = if is_focused { Color::White } else { Color::Gray };

        // Create title with highlighted first letter
        let title = if is_focused {
            Line::from(vec![
                Span::styled("(I)", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                Span::styled("nfo", Style::default().add_modifier(Modifier::BOLD)),
            ])
        } else {
            Line::from(vec![
                Span::styled("(I)", Style::default().fg(Color::Yellow)),
                Span::raw("nfo"),
            ])
        };

        let titles = vec!["Combat", "Log"];
        let tabs = Tabs::new(titles)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(title)
                    .border_style(Style::default().fg(border_color)),
            )
            .select(self.state.left_tab as usize)
            .style(Style::default().fg(Color::White))
            .highlight_style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD));

        let inner_area = Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: 3,
        };
        f.render_widget(tabs, inner_area);

        // Content area (below tabs)
        let content_area = Rect {
            x: area.x + 1,
            y: area.y + 3,
            width: area.width.saturating_sub(2),
            height: area.height.saturating_sub(4),
        };

        match self.state.left_tab {
            LeftTab::Combat => self.draw_combat_view(f, content_area, view),
            LeftTab::Log => self.draw_log_view(f, content_area, view),
        }
    }

    /// Draw the combat view
    fn draw_combat_view(&self, f: &mut Frame, area: Rect, _view: &GameStateView) {
        let text = Text::from("(No combat)");
        let paragraph = Paragraph::new(text).wrap(Wrap { trim: true });
        f.render_widget(paragraph, area);
    }

    /// Draw the log view
    fn draw_log_view(&self, f: &mut Frame, area: Rect, view: &GameStateView) {
        // Get logs from the game logger
        let logs = view.logger().logs();

        // Take the last N logs that fit in the area
        let items: Vec<ListItem> = logs
            .iter()
            .rev()
            .take(area.height as usize)
            .map(|entry| ListItem::new(entry.message.as_str()))
            .collect();

        let list = List::new(items);
        f.render_widget(list, area);
    }

    /// Draw the Prompt pane (Actions) with choices
    fn draw_prompt(
        &self,
        f: &mut Frame,
        area: Rect,
        _view: &GameStateView,
        current_prompt: Option<&str>,
        choices: &[(String, bool)],
    ) {
        // Determine focus state
        let is_focused = self.state.focused_pane == FocusedPane::Actions;
        let border_color = if is_focused { Color::White } else { Color::Gray };

        // Create title with highlighted first letter
        let title = if is_focused {
            Line::from(vec![
                Span::styled("(A)", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                Span::styled("ctions", Style::default().add_modifier(Modifier::BOLD)),
            ])
        } else {
            Line::from(vec![
                Span::styled("(A)", Style::default().fg(Color::Yellow)),
                Span::raw("ctions"),
            ])
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .title(title)
            .border_style(Style::default().fg(border_color));
        let inner_area = block.inner(area);
        f.render_widget(block, area);

        if let Some(prompt) = current_prompt {
            // Show prompt text
            let prompt_text = Text::from(prompt);
            let prompt_height = 3; // Reserve lines for prompt
            let prompt_area = Rect {
                x: inner_area.x,
                y: inner_area.y,
                width: inner_area.width,
                height: prompt_height.min(inner_area.height),
            };
            let paragraph = Paragraph::new(prompt_text)
                .wrap(Wrap { trim: true })
                .style(Style::default().fg(Color::Cyan));
            f.render_widget(paragraph, prompt_area);

            // Show choices below prompt
            if !choices.is_empty() {
                let choices_area = Rect {
                    x: inner_area.x,
                    y: inner_area.y + prompt_height,
                    width: inner_area.width,
                    height: inner_area.height.saturating_sub(prompt_height),
                };

                let items: Vec<ListItem> = choices
                    .iter()
                    .map(|(text, is_highlighted)| {
                        let style = if *is_highlighted {
                            Style::default()
                                .fg(Color::Black)
                                .bg(Color::Yellow)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default()
                        };
                        ListItem::new(text.as_str()).style(style)
                    })
                    .collect();

                let list = List::new(items);
                f.render_widget(list, choices_area);
            }
        } else {
            let text = Text::from("(Waiting for input...)");
            let paragraph = Paragraph::new(text).style(Style::default().fg(Color::DarkGray));
            f.render_widget(paragraph, inner_area);
        }
    }

    /// Draw both battlefields (opponent on top, player on bottom)
    fn draw_battlefields(&mut self, f: &mut Frame, area: Rect, view: &GameStateView) {
        // Split into opponent and player battlefields
        let bf_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(3),         // Player info header
                Constraint::Percentage(45), // Opponent battlefield
                Constraint::Percentage(45), // Player battlefield
                Constraint::Min(3),         // Player info footer
            ])
            .split(area);

        // Draw opponent info
        let opponent_id = view.opponents().next();
        if let Some(opp_id) = opponent_id {
            self.draw_player_info(f, bf_chunks[0], view, opp_id);
            self.draw_battlefield(f, bf_chunks[1], view, opp_id, "Opponent Battlefield");
        }

        // Draw player's own battlefield
        self.draw_battlefield(f, bf_chunks[2], view, view.player_id(), "Your Battlefield");
        self.draw_player_info(f, bf_chunks[3], view, view.player_id());
    }

    /// Draw player info bar (life, zones, etc.)
    fn draw_player_info(&self, f: &mut Frame, area: Rect, view: &GameStateView, player_id: PlayerId) {
        let life = view.player_life(player_id);
        let hand_size = view.player_hand(player_id).len();
        let graveyard_size = view.player_graveyard(player_id).len();
        let library_size = view.player_library(player_id).len();

        let player_label = if player_id == view.player_id() { "You" } else { "Opp" };

        // Left side: player stats
        let stats_text = format!(
            "{}: {} life | Hand: {} | GY: {} | Lib: {}",
            player_label, life, hand_size, graveyard_size, library_size
        );

        // Right side: turn and phase info
        let turn_number = view.turn_number();
        let current_step = view.current_step();
        let active_player = view.active_player();
        let is_active = player_id == active_player;

        // Calculate player's turn number (P1 goes on odd turns, P2 on even)
        let is_first_player = (active_player == player_id) == (turn_number % 2 == 1);
        let player_turn = if is_first_player {
            turn_number.div_ceil(2)
        } else {
            turn_number / 2
        };

        // All phases with current one underlined
        let all_steps = [
            Step::Untap,
            Step::Upkeep,
            Step::Draw,
            Step::Main1,
            Step::BeginCombat,
            Step::DeclareAttackers,
            Step::DeclareBlockers,
            Step::CombatDamage,
            Step::EndCombat,
            Step::Main2,
            Step::End,
        ];

        // Display player turn or "_" if inactive
        let turn_display = if is_active {
            player_turn.to_string()
        } else {
            "_".to_string()
        };

        let mut phase_spans = vec![Span::raw(format!("Turn: {} ({}) | ", turn_display, turn_number))];

        for (i, step) in all_steps.iter().enumerate() {
            let abbrev = Self::step_abbrev(*step);
            // Only underline current step if this is the active player
            let span = if is_active && *step == current_step {
                Span::styled(abbrev, Style::default().add_modifier(Modifier::UNDERLINED))
            } else {
                Span::raw(abbrev)
            };
            phase_spans.push(span);

            if i < all_steps.len() - 1 {
                phase_spans.push(Span::raw(" "));
            }
        }

        // Calculate spacing for right alignment
        let inner_width = area.width.saturating_sub(4); // Account for borders and padding
        let stats_len = stats_text.len() as u16;
        // Phase text without underline formatting for length calc (use max width with turn number)
        let phase_text_plain = format!(
            "Turn: {} ({}) | UP UK DR M1 BC DA DB CD EC M2 ET",
            turn_display, turn_number
        );
        let phase_len = phase_text_plain.len() as u16;
        let padding = inner_width.saturating_sub(stats_len + phase_len);

        // Combine with spacing
        let mut line_spans = vec![Span::raw(stats_text)];
        line_spans.push(Span::raw(" ".repeat(padding as usize)));
        line_spans.extend(phase_spans);

        let line = Line::from(line_spans);

        // Bold the entire line if this is the active player
        let base_style = if is_active {
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };

        let paragraph = Paragraph::new(Text::from(line)).style(base_style).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Gray)),
        );

        f.render_widget(paragraph, area);
    }

    /// Draw a single battlefield
    fn draw_battlefield(&mut self, f: &mut Frame, area: Rect, view: &GameStateView, owner_id: PlayerId, _title: &str) {
        let battlefield = view.battlefield();
        let player_cards: Vec<CardId> = battlefield
            .iter()
            .filter(|&&card_id| {
                view.get_card(card_id)
                    .map(|c| c.controller == owner_id)
                    .unwrap_or(false)
            })
            .copied()
            .collect();

        // Group cards: lands, creatures, other
        let (lands, creatures, others): (Vec<_>, Vec<_>, Vec<_>) = player_cards.iter().fold(
            (Vec::new(), Vec::new(), Vec::new()),
            |(mut lands, mut creatures, mut others), &card_id| {
                if let Some(card) = view.get_card(card_id) {
                    if card.is_land() {
                        lands.push(card_id);
                    } else if card.is_creature() {
                        creatures.push(card_id);
                    } else {
                        others.push(card_id);
                    }
                }
                (lands, creatures, others)
            },
        );

        // Determine focus state
        let is_player_bf = owner_id == view.player_id();
        let is_focused = if is_player_bf {
            self.state.focused_pane == FocusedPane::YourBattlefield
        } else {
            self.state.focused_pane == FocusedPane::OpponentBattlefield
        };
        let border_color = if is_focused { Color::White } else { Color::Gray };

        // Create title with highlighted first letter
        let title_line = if is_player_bf {
            if is_focused {
                Line::from(vec![
                    Span::styled("(Y)", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                    Span::styled("our Battlefield", Style::default().add_modifier(Modifier::BOLD)),
                ])
            } else {
                Line::from(vec![
                    Span::styled("(Y)", Style::default().fg(Color::Yellow)),
                    Span::raw("our Battlefield"),
                ])
            }
        } else if is_focused {
            Line::from(vec![
                Span::styled("(O)", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                Span::styled("pponent Battlefield", Style::default().add_modifier(Modifier::BOLD)),
            ])
        } else {
            Line::from(vec![
                Span::styled("(O)", Style::default().fg(Color::Yellow)),
                Span::raw("pponent Battlefield"),
            ])
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .title(title_line)
            .border_style(Style::default().fg(border_color));
        let inner_area = block.inner(area);
        f.render_widget(block, area);

        if player_cards.is_empty() {
            let empty_text = Text::from("(Empty)");
            let paragraph = Paragraph::new(empty_text)
                .style(Style::default().fg(Color::DarkGray))
                .alignment(Alignment::Center);
            f.render_widget(paragraph, inner_area);
            return;
        }

        // Build card groups for size calculation
        let mut card_groups = Vec::new();
        if !lands.is_empty() {
            card_groups.push((lands.clone(), "Lands"));
        }
        if !creatures.is_empty() {
            card_groups.push((creatures.clone(), "Creatures"));
        }
        if !others.is_empty() {
            card_groups.push((others.clone(), "Other"));
        }

        // Calculate optimal card size for this battlefield
        let (card_width, card_height) = Self::calculate_optimal_card_size(inner_area, &card_groups, view);

        // Render card groups with optimal size
        let mut y_offset = 0;

        if !lands.is_empty() {
            y_offset += self.render_card_group(
                f,
                inner_area,
                y_offset,
                view,
                &lands,
                "Lands",
                Color::Green,
                card_width,
                card_height,
            );
        }

        if !creatures.is_empty() {
            y_offset += self.render_card_group(
                f,
                inner_area,
                y_offset,
                view,
                &creatures,
                "Creatures",
                Color::Red,
                card_width,
                card_height,
            );
        }

        if !others.is_empty() {
            self.render_card_group(
                f,
                inner_area,
                y_offset,
                view,
                &others,
                "Other",
                Color::Blue,
                card_width,
                card_height,
            );
        }
    }

    /// Card size constants
    /// Default size maintains MTG card aspect ratio: width/height = 10/7 ≈ 1.43
    /// This accounts for terminal character aspect (~2:1) to create visually proportional cards
    const DEFAULT_CARD_WIDTH: u16 = 10;
    const DEFAULT_CARD_HEIGHT: u16 = 7;
    const MIN_CARD_WIDTH: u16 = 5;
    const MIN_CARD_HEIGHT: u16 = 4;
    const CARD_SPACING: u16 = 1;

    /// Compute card width from height while maintaining the default aspect ratio
    /// This is the centralized function for all aspect ratio calculations
    fn compute_width_from_height(height: u16) -> u16 {
        ((height as f32 * Self::DEFAULT_CARD_WIDTH as f32) / Self::DEFAULT_CARD_HEIGHT as f32).round() as u16
    }

    /// Get card dimensions based on tapped state and base size
    /// Tapped cards swap width and height to simulate 90-degree rotation
    fn get_card_dimensions_with_size(
        view: &GameStateView,
        card_id: CardId,
        base_width: u16,
        base_height: u16,
    ) -> (u16, u16) {
        let is_tapped = view.is_tapped(card_id);
        if is_tapped {
            // Swap dimensions for tapped cards (simulate rotation)
            (base_height, base_width)
        } else {
            (base_width, base_height)
        }
    }

    /// Test if all cards fit in the battlefield area with given card size
    fn test_card_size_fits(
        area: Rect,
        card_groups: &[(Vec<CardId>, &str)], // (cards, label)
        view: &GameStateView,
        card_width: u16,
        card_height: u16,
    ) -> bool {
        let mut y_offset = 0u16;

        for (cards, _label) in card_groups {
            if y_offset >= area.height {
                return false;
            }

            // Account for label height
            y_offset += 1;

            // Simulate packing for this group
            let mut current_x = 0u16;
            let mut row_height = 0u16;

            for &card_id in cards {
                let (card_w, card_h) = Self::get_card_dimensions_with_size(view, card_id, card_width, card_height);

                // Check if card fits on current row
                if current_x + card_w > area.width && current_x > 0 {
                    // Need to wrap to next row
                    current_x = 0;
                    y_offset += row_height + Self::CARD_SPACING;
                    row_height = 0;

                    // Check if we have vertical space for new row
                    if y_offset >= area.height {
                        return false;
                    }
                }

                // Check if this card fits vertically
                if y_offset + card_h > area.height {
                    return false;
                }

                // Update position for next card
                current_x += card_w + Self::CARD_SPACING;
                row_height = row_height.max(card_h);
            }

            // Finalize this group's height
            if current_x > 0 {
                y_offset += row_height;
            }
        }

        true
    }

    /// Calculate optimal card size for battlefield
    /// Returns (width, height) that maximizes card size while fitting all cards
    ///
    /// This function uses a greedy algorithm that increments height and computes
    /// width from height to maintain the correct aspect ratio. This ensures
    /// consistent aspect ratios across all cards regardless of tapped state.
    fn calculate_optimal_card_size(
        area: Rect,
        card_groups: &[(Vec<CardId>, &str)],
        view: &GameStateView,
    ) -> (u16, u16) {
        // Try default size first
        if Self::test_card_size_fits(
            area,
            card_groups,
            view,
            Self::DEFAULT_CARD_WIDTH,
            Self::DEFAULT_CARD_HEIGHT,
        ) {
            // Try increasing size (greedy algorithm)
            // Increment height and compute width to maintain aspect ratio
            let mut height = Self::DEFAULT_CARD_HEIGHT;
            let mut width = Self::DEFAULT_CARD_WIDTH;

            loop {
                let next_height = height + 1;
                // Compute width from height using centralized aspect ratio function
                let next_width = Self::compute_width_from_height(next_height);

                if Self::test_card_size_fits(area, card_groups, view, next_width, next_height) {
                    width = next_width;
                    height = next_height;
                } else {
                    break;
                }
            }

            (width, height)
        } else {
            // Default doesn't fit, shrink down
            // Decrement height and compute width to maintain aspect ratio
            let mut height = Self::DEFAULT_CARD_HEIGHT;
            let mut width = Self::DEFAULT_CARD_WIDTH;

            while !Self::test_card_size_fits(area, card_groups, view, width, height) && height > Self::MIN_CARD_HEIGHT {
                height -= 1;
                // Compute width from height using centralized aspect ratio function
                width = Self::compute_width_from_height(height).max(Self::MIN_CARD_WIDTH);
            }

            (width.max(Self::MIN_CARD_WIDTH), height.max(Self::MIN_CARD_HEIGHT))
        }
    }

    /// Render a group of cards (lands, creatures, others) with dynamic packing
    #[allow(clippy::too_many_arguments)]
    fn render_card_group(
        &mut self,
        f: &mut Frame,
        area: Rect,
        y_offset: u16,
        view: &GameStateView,
        cards: &[CardId],
        label: &str,
        color: Color,
        card_width: u16,
        card_height: u16,
    ) -> u16 {
        if y_offset >= area.height {
            return 0;
        }

        // Draw group label
        let label_area = Rect {
            x: area.x,
            y: area.y + y_offset,
            width: area.width,
            height: 1,
        };
        let label_text = Text::from(Span::styled(
            format!("{}:", label),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ));
        f.render_widget(Paragraph::new(label_text), label_area);

        let mut rendered_height = 1; // Start after label

        // Dynamic packing: pack cards left-to-right, wrapping when needed
        let mut current_x = area.x;
        let mut current_y = area.y + y_offset + rendered_height;
        let mut row_height = 0u16;

        for &card_id in cards {
            let (card_w, card_h) = Self::get_card_dimensions_with_size(view, card_id, card_width, card_height);

            // Check if card fits on current row
            if current_x + card_w > area.x + area.width && current_x > area.x {
                // Wrap to next row
                current_x = area.x;
                current_y += row_height + Self::CARD_SPACING;
                row_height = 0;
            }

            // Check if we have vertical space
            if current_y + card_h > area.y + area.height {
                break; // No more vertical space
            }

            // Render this card
            let card_area = Rect {
                x: current_x,
                y: current_y,
                width: card_w,
                height: card_h,
            };
            self.render_card_box(f, card_area, view, card_id);

            // Update position for next card
            current_x += card_w + Self::CARD_SPACING;
            row_height = row_height.max(card_h);
        }

        // Total height used by this group
        if current_x > area.x {
            // We rendered at least one card on the last row
            rendered_height = (current_y - (area.y + y_offset)) + row_height;
        }

        rendered_height
    }

    /// Render a single card as a box with priority-based content layout
    fn render_card_box(&mut self, f: &mut Frame, area: Rect, view: &GameStateView, card_id: CardId) {
        // Track card position for mouse hit testing
        self.state.card_positions.push(CardPosition { card_id, area });
        let name = view.card_name(card_id).unwrap_or_else(|| format!("{:?}", card_id));
        let is_tapped = view.is_tapped(card_id);
        let card = view.get_card(card_id);

        // Check if this card is currently selected
        let is_selected =
            Some(card_id) == self.state.selected_card_in_your_bf || Some(card_id) == self.state.selected_card_in_opp_bf;

        // Calculate available content dimensions (excluding borders)
        let content_width = area.width.saturating_sub(2) as usize;
        let content_height = area.height.saturating_sub(2);

        // Determine border color and text style
        let border_color = if let Some(card) = card.as_ref() {
            match card.colors.len() {
                0 => Color::Gray,
                1 => match card.colors[0] {
                    crate::core::Color::Red => Color::Red,
                    crate::core::Color::Green => Color::Green,
                    crate::core::Color::Blue => Color::Blue,
                    crate::core::Color::White => Color::White,
                    crate::core::Color::Black => Color::DarkGray,
                    crate::core::Color::Colorless => Color::Gray,
                },
                _ => Color::Yellow,
            }
        } else {
            Color::Gray
        };

        let border_style = if is_selected {
            Style::default().fg(border_color).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(border_color)
        };

        // Determine if card is in valid choices list
        let is_valid_choice = self.state.valid_choices.contains(&card_id);
        let has_choice_context = self.state.choice_context != ChoiceContext::None;

        // Apply highlighting/dimming based on choice context
        let text_style = if has_choice_context {
            if is_valid_choice {
                // Valid choice: bright/normal
                Style::default().fg(Color::White)
            } else {
                // Invalid choice: dimmed
                Style::default().fg(Color::DarkGray)
            }
        } else if is_tapped {
            // No choice context: show tapped state as usual
            Style::default().fg(Color::Gray)
        } else {
            Style::default().fg(Color::White)
        };

        // Build card content with priority-based layout
        let mut lines = Vec::new();

        // Priority 1: Title (always included, prefer full name over truncation)
        let cost_str = card.as_ref().map(|c| c.mana_cost.to_string()).unwrap_or_default();
        let cost_len = cost_str.len();

        let title_style = if is_selected {
            Style::default()
                .add_modifier(Modifier::BOLD)
                .add_modifier(Modifier::UNDERLINED)
        } else {
            Style::default()
        };

        // Strategy: Try to fit name + cost on one line
        // If name would be truncated and we have vertical space, use two lines instead
        let name_and_cost_fit = name.len() + cost_len < content_width;
        let name_fits_alone = name.len() <= content_width;
        let have_vertical_space = content_height >= 3; // Need at least 3 lines (name, cost, something else)

        if !cost_str.is_empty() && name_and_cost_fit {
            // Both fit on one line - ideal case
            let padding = content_width.saturating_sub(name.len() + cost_len);
            lines.push(Line::from(vec![
                Span::styled(name.clone(), title_style),
                Span::raw(" ".repeat(padding)),
                Span::raw(cost_str.clone()),
            ]));
        } else if !cost_str.is_empty() && !name_fits_alone && have_vertical_space {
            // Name would be truncated, but we have space for cost on separate line
            // Truncate name minimally
            let display_name = if name.len() > content_width {
                if content_width <= 5 {
                    name.chars().take(content_width).collect::<String>()
                } else {
                    format!(
                        "{}..",
                        name.chars().take(content_width.saturating_sub(2)).collect::<String>()
                    )
                }
            } else {
                name.clone()
            };
            lines.push(Line::from(Span::styled(display_name, title_style)));
            lines.push(Line::from(cost_str.clone()));
        } else if !cost_str.is_empty() && !name_and_cost_fit && name_fits_alone && have_vertical_space {
            // Name fits, cost doesn't fit on same line, use two lines
            lines.push(Line::from(Span::styled(name.clone(), title_style)));
            lines.push(Line::from(cost_str.clone()));
        } else {
            // Fallback: Single line with truncation if needed
            let display_name = if name.len() > content_width {
                if content_width <= 5 {
                    name.chars().take(content_width).collect::<String>()
                } else {
                    format!(
                        "{}..",
                        name.chars().take(content_width.saturating_sub(2)).collect::<String>()
                    )
                }
            } else {
                name.clone()
            };
            lines.push(Line::from(Span::styled(display_name, title_style)));
        }

        // Priority 2: Tapped indicator (only if tapped and room)
        // Use compact "[T]" for narrow cards, full "[TAPPED]" only when we have plenty of space
        if is_tapped && lines.len() < content_height as usize {
            let tapped_text = if content_width >= 12 {
                "[TAPPED]"
            } else if content_width >= 3 {
                "[T]"
            } else {
                "T" // Ultra-compact for very narrow cards
            };
            lines.push(Line::from(tapped_text));
        }

        // Determine if we need to reserve last line for P/T
        let is_creature = card.as_ref().map(|c| c.is_creature()).unwrap_or(false);
        let pt_str = if is_creature {
            card.as_ref()
                .map(|c| format!("{}/{}", c.power.unwrap_or(0), c.toughness.unwrap_or(0)))
                .unwrap_or_default()
        } else {
            String::new()
        };

        let reserve_last_line_for_pt = is_creature && !pt_str.is_empty() && pt_str.len() <= content_width;

        // Calculate available lines for description/type
        let max_total_lines = if reserve_last_line_for_pt {
            content_height.saturating_sub(1) as usize
        } else {
            content_height as usize
        };

        // Priority 4: Description (fit as much as possible with "...")
        if let Some(card) = card.as_ref() {
            if !card.text.is_empty() && lines.len() < max_total_lines {
                let available_lines = max_total_lines.saturating_sub(lines.len());
                let desc_lines = card.text.split('\n').collect::<Vec<_>>();

                for (i, desc_line) in desc_lines.iter().enumerate().take(available_lines) {
                    if i == available_lines - 1 && (i < desc_lines.len() - 1 || desc_line.len() > content_width) {
                        // Last line of available space - add elision if there's more
                        let truncated = if desc_line.len() > content_width.saturating_sub(3) {
                            format!(
                                "{}...",
                                desc_line
                                    .chars()
                                    .take(content_width.saturating_sub(3))
                                    .collect::<String>()
                            )
                        } else if i < desc_lines.len() - 1 {
                            format!("{}...", desc_line)
                        } else {
                            desc_line.to_string()
                        };
                        lines.push(Line::from(truncated));
                    } else if desc_line.len() > content_width {
                        // Line too long, truncate
                        let truncated = format!(
                            "{}...",
                            desc_line
                                .chars()
                                .take(content_width.saturating_sub(3))
                                .collect::<String>()
                        );
                        lines.push(Line::from(truncated));
                    } else {
                        lines.push(Line::from(*desc_line));
                    }
                }
            }
        }

        // Priority 6: Type line (only if completely fits)
        if let Some(card) = card.as_ref() {
            if !card.types.is_empty() && lines.len() < max_total_lines {
                let types_str = card
                    .types
                    .iter()
                    .map(|t| format!("{:?}", t))
                    .collect::<Vec<_>>()
                    .join(" ");
                if types_str.len() <= content_width {
                    lines.push(Line::from(types_str));
                }
            }
        }

        // Fill empty lines up to max_total_lines
        while lines.len() < max_total_lines {
            lines.push(Line::from(""));
        }

        // Priority 3: P/T (bottom right, only if room was reserved)
        if reserve_last_line_for_pt {
            let padding = content_width.saturating_sub(pt_str.len());
            lines.push(Line::from(vec![Span::raw(" ".repeat(padding)), Span::raw(pt_str)]));
        }

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .style(text_style);

        let text = Text::from(lines);
        let paragraph = Paragraph::new(text).block(block);
        f.render_widget(paragraph, area);
    }

    /// Draw the card details panel
    fn draw_card_details(&self, f: &mut Frame, area: Rect, view: &GameStateView) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title("Card Details")
            .border_style(Style::default().fg(Color::Gray));

        let inner_area = block.inner(area);
        f.render_widget(block, area);

        if let Some(card_id) = self.state.selected_card_id {
            if let Some(card) = view.get_card(card_id) {
                let types_str = card
                    .types
                    .iter()
                    .map(|t| format!("{:?}", t))
                    .collect::<Vec<_>>()
                    .join(" ");

                let mut lines = vec![
                    Line::from(Span::styled(
                        card.name.as_str(),
                        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                    )),
                    Line::from(""),
                    Line::from(format!("Type: {}", types_str)),
                    Line::from(format!("Cost: {}", card.mana_cost)),
                ];

                if card.is_creature() {
                    lines.push(Line::from(format!(
                        "P/T: {}/{}",
                        card.power.unwrap_or(0),
                        card.toughness.unwrap_or(0)
                    )));
                }

                if !card.text.is_empty() {
                    lines.push(Line::from(""));
                    // Split card text on newlines for natural multi-paragraph display
                    for text_line in card.text.split('\n') {
                        lines.push(Line::from(text_line));
                    }
                }

                let text = Text::from(lines);
                let paragraph = Paragraph::new(text).wrap(Wrap { trim: true });
                f.render_widget(paragraph, inner_area);
                return;
            }
        }

        // No card selected
        let text = Text::from("(No card selected)");
        let paragraph = Paragraph::new(text).style(Style::default().fg(Color::DarkGray));
        f.render_widget(paragraph, inner_area);
    }

    /// Calculate maximum mana production from battlefield
    ///
    /// Returns (total, W, U, B, R, G, C) where:
    /// - total = number of untapped mana sources
    /// - W/U/B/R/G/C = max of each color we could produce
    ///
    /// Note: For dual lands, this counts +1 for both colors but only +1 total.
    ///
    /// This now delegates to GameStateView::max_mana_capacity() which uses
    /// the ManaEngine for correct calculation.
    fn calculate_max_mana(view: &GameStateView) -> (u8, u8, u8, u8, u8, u8, u8) {
        view.max_mana_capacity()
    }

    /// Draw the hand panel
    fn draw_hand(&self, f: &mut Frame, area: Rect, view: &GameStateView) {
        let hand = view.hand();

        // Determine focus state
        let is_focused = self.state.focused_pane == FocusedPane::Hand;
        let border_color = if is_focused { Color::White } else { Color::Gray };

        // Create title with highlighted first letter
        let title = if is_focused {
            vec![
                Span::styled("(H)", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                Span::styled(
                    format!("and ({})", hand.len()),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
            ]
        } else {
            vec![
                Span::styled("(H)", Style::default().fg(Color::Yellow)),
                Span::raw(format!("and ({})", hand.len())),
            ]
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .title(Line::from(title))
            .border_style(Style::default().fg(border_color));

        let inner_area = block.inner(area);
        f.render_widget(block, area);

        // Reserve bottom line for mana display
        let mana_line_height = 1;
        let hand_area = Rect {
            x: inner_area.x,
            y: inner_area.y,
            width: inner_area.width,
            height: inner_area.height.saturating_sub(mana_line_height),
        };
        let mana_area = Rect {
            x: inner_area.x,
            y: inner_area.y + hand_area.height,
            width: inner_area.width,
            height: mana_line_height,
        };

        if hand.is_empty() {
            let text = Text::from("(Empty)");
            let paragraph = Paragraph::new(text)
                .style(Style::default().fg(Color::DarkGray))
                .alignment(Alignment::Center);
            f.render_widget(paragraph, hand_area);
        } else {
            // Display cards vertically as list
            let items: Vec<ListItem> = hand
                .iter()
                .enumerate()
                .map(|(idx, &card_id)| {
                    let name = view.card_name(card_id).unwrap_or_else(|| format!("{:?}", card_id));

                    let cost = view
                        .get_card(card_id)
                        .map(|c| format!(" ({})", c.mana_cost))
                        .unwrap_or_default();

                    let text = format!("[{}] {}{}", idx, name, cost);

                    // Highlight selected card if Hand pane is focused
                    let is_selected = is_focused && self.state.selected_card_in_hand == Some(idx);
                    let style = if is_selected {
                        Style::default()
                            .fg(Color::Black)
                            .bg(Color::Yellow)
                            .add_modifier(Modifier::BOLD)
                            .add_modifier(Modifier::UNDERLINED)
                    } else {
                        Style::default().fg(Color::White)
                    };

                    ListItem::new(text).style(style)
                })
                .collect();

            let list = List::new(items);
            f.render_widget(list, hand_area);
        }

        // Draw max mana line at bottom
        let (total, w, u, b, r, g, c) = Self::calculate_max_mana(view);
        let mana_text = format!("Max Mana: {} ~= {}W {}U {}B {}R {}G {}C", total, w, u, b, r, g, c);
        let mana_paragraph = Paragraph::new(mana_text).style(Style::default().fg(Color::Cyan));
        f.render_widget(mana_paragraph, mana_area);
    }

    /// Draw the Stack pane
    fn draw_stack(&self, f: &mut Frame, area: Rect, _view: &GameStateView) {
        // Determine focus state
        let is_focused = self.state.focused_pane == FocusedPane::Stack;
        let border_color = if is_focused { Color::White } else { Color::Gray };

        // Create title with highlighted first letter
        let title = if is_focused {
            Line::from(vec![
                Span::styled("(S)", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                Span::styled("tack", Style::default().add_modifier(Modifier::BOLD)),
            ])
        } else {
            Line::from(vec![
                Span::styled("(S)", Style::default().fg(Color::Yellow)),
                Span::raw("tack"),
            ])
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .title(title)
            .border_style(Style::default().fg(border_color));
        let inner_area = block.inner(area);
        f.render_widget(block, area);

        // TODO: Display actual stack contents from game state
        let text = Text::from("(Stack empty)");
        let paragraph = Paragraph::new(text).style(Style::default().fg(Color::DarkGray));
        f.render_widget(paragraph, inner_area);
    }

    /// Wait for user input and update highlighted choice
    fn wait_for_choice_input(&mut self, num_choices: usize, view: &GameStateView) -> io::Result<InputAction> {
        loop {
            if event::poll(std::time::Duration::from_millis(100))? {
                let event = event::read()?;
                match event {
                    Event::Mouse(mouse_event) => {
                        if let MouseEventKind::Down(MouseButton::Left) = mouse_event.kind {
                            let (x, y) = (mouse_event.column, mouse_event.row);

                            // Check if any card was clicked
                            for card_pos in &self.state.card_positions {
                                if x >= card_pos.area.x
                                    && x < card_pos.area.x + card_pos.area.width
                                    && y >= card_pos.area.y
                                    && y < card_pos.area.y + card_pos.area.height
                                {
                                    // Card clicked! Select it and show details
                                    self.state.selected_card_id = Some(card_pos.card_id);

                                    // Update battlefield selection if it's in a battlefield
                                    if let Some(card) = view.get_card(card_pos.card_id) {
                                        if card.controller == view.player_id() {
                                            self.state.selected_card_in_your_bf = Some(card_pos.card_id);
                                            self.state.focused_pane = FocusedPane::YourBattlefield;
                                        } else {
                                            self.state.selected_card_in_opp_bf = Some(card_pos.card_id);
                                            self.state.focused_pane = FocusedPane::OpponentBattlefield;
                                        }
                                    }

                                    return Ok(InputAction::Continue); // Redraw with new selection
                                }
                            }
                        }
                    }
                    Event::Key(key) => {
                        match key.code {
                            // Pane focus switching (H, I, Y, O, A)
                            KeyCode::Char('h') | KeyCode::Char('H') => {
                                self.state.focused_pane = FocusedPane::Hand;
                                // Initialize selection to first card if hand not empty
                                let hand = view.hand();
                                if !hand.is_empty() && self.state.selected_card_in_hand.is_none() {
                                    self.state.selected_card_in_hand = Some(0);
                                    self.state.selected_card_id = Some(hand[0]);
                                }
                                return Ok(InputAction::Continue); // Redraw needed
                            }
                            KeyCode::Char('i') | KeyCode::Char('I') => {
                                self.state.focused_pane = FocusedPane::Info;
                                return Ok(InputAction::Continue); // Redraw needed
                            }
                            KeyCode::Char('y') | KeyCode::Char('Y') => {
                                self.state.focused_pane = FocusedPane::YourBattlefield;
                                // Initialize selection to first card if battlefield not empty
                                let bf_cards = Self::get_battlefield_cards_in_order(view, view.player_id());
                                if !bf_cards.is_empty() && self.state.selected_card_in_your_bf.is_none() {
                                    self.state.selected_card_in_your_bf = Some(bf_cards[0]);
                                    self.state.selected_card_id = Some(bf_cards[0]);
                                }
                                return Ok(InputAction::Continue); // Redraw needed
                            }
                            KeyCode::Char('o') | KeyCode::Char('O') => {
                                self.state.focused_pane = FocusedPane::OpponentBattlefield;
                                // Initialize selection to first card if battlefield not empty
                                if let Some(opp_id) = view.opponents().next() {
                                    let bf_cards = Self::get_battlefield_cards_in_order(view, opp_id);
                                    if !bf_cards.is_empty() && self.state.selected_card_in_opp_bf.is_none() {
                                        self.state.selected_card_in_opp_bf = Some(bf_cards[0]);
                                        self.state.selected_card_id = Some(bf_cards[0]);
                                    }
                                }
                                return Ok(InputAction::Continue); // Redraw needed
                            }
                            KeyCode::Char('a') | KeyCode::Char('A') => {
                                self.state.focused_pane = FocusedPane::Actions;
                                return Ok(InputAction::Continue); // Redraw needed
                            }
                            KeyCode::Char('s') | KeyCode::Char('S') => {
                                self.state.focused_pane = FocusedPane::Stack;
                                return Ok(InputAction::Continue); // Redraw needed
                            }
                            // Arrow key navigation - route based on focused pane
                            KeyCode::Up | KeyCode::Char('k') => {
                                match self.state.focused_pane {
                                    FocusedPane::Actions => {
                                        // Navigate choices in Actions pane
                                        if self.state.highlighted_choice > 0 {
                                            self.state.highlighted_choice -= 1;
                                        }
                                        return Ok(InputAction::Continue);
                                    }
                                    FocusedPane::Hand => {
                                        // Navigate cards in Hand pane
                                        let hand = view.hand();
                                        if !hand.is_empty() {
                                            let current = self.state.selected_card_in_hand.unwrap_or(0);
                                            if current > 0 {
                                                self.state.selected_card_in_hand = Some(current - 1);
                                                self.state.selected_card_id = Some(hand[current - 1]);
                                            }
                                        }
                                        return Ok(InputAction::Continue);
                                    }
                                    FocusedPane::YourBattlefield => {
                                        // Navigate cards in Your Battlefield (2D: move up one row)
                                        let bf_cards = Self::get_battlefield_cards_in_order(view, view.player_id());
                                        if !bf_cards.is_empty() {
                                            let current_card = self.state.selected_card_in_your_bf;
                                            if let Some(current_idx) =
                                                current_card.and_then(|id| bf_cards.iter().position(|&c| c == id))
                                            {
                                                const CARDS_PER_ROW: usize = 4; // Estimate based on typical terminal width
                                                if current_idx >= CARDS_PER_ROW {
                                                    let new_idx = current_idx - CARDS_PER_ROW;
                                                    let new_card = bf_cards[new_idx];
                                                    self.state.selected_card_in_your_bf = Some(new_card);
                                                    self.state.selected_card_id = Some(new_card);
                                                }
                                            }
                                        }
                                        return Ok(InputAction::Continue);
                                    }
                                    FocusedPane::OpponentBattlefield => {
                                        // Navigate cards in Opponent Battlefield (2D: move up one row)
                                        if let Some(opp_id) = view.opponents().next() {
                                            let bf_cards = Self::get_battlefield_cards_in_order(view, opp_id);
                                            if !bf_cards.is_empty() {
                                                let current_card = self.state.selected_card_in_opp_bf;
                                                if let Some(current_idx) =
                                                    current_card.and_then(|id| bf_cards.iter().position(|&c| c == id))
                                                {
                                                    const CARDS_PER_ROW: usize = 4;
                                                    if current_idx >= CARDS_PER_ROW {
                                                        let new_idx = current_idx - CARDS_PER_ROW;
                                                        let new_card = bf_cards[new_idx];
                                                        self.state.selected_card_in_opp_bf = Some(new_card);
                                                        self.state.selected_card_id = Some(new_card);
                                                    }
                                                }
                                            }
                                        }
                                        return Ok(InputAction::Continue);
                                    }
                                    _ => {
                                        return Ok(InputAction::Continue);
                                    }
                                }
                            }
                            KeyCode::Down | KeyCode::Char('j') => {
                                match self.state.focused_pane {
                                    FocusedPane::Actions => {
                                        // Navigate choices in Actions pane
                                        if self.state.highlighted_choice + 1 < num_choices {
                                            self.state.highlighted_choice += 1;
                                        }
                                        return Ok(InputAction::Continue);
                                    }
                                    FocusedPane::Hand => {
                                        // Navigate cards in Hand pane
                                        let hand = view.hand();
                                        if !hand.is_empty() {
                                            let current = self.state.selected_card_in_hand.unwrap_or(0);
                                            if current + 1 < hand.len() {
                                                self.state.selected_card_in_hand = Some(current + 1);
                                                self.state.selected_card_id = Some(hand[current + 1]);
                                            }
                                        }
                                        return Ok(InputAction::Continue);
                                    }
                                    FocusedPane::YourBattlefield => {
                                        // Navigate cards in Your Battlefield (2D: move down one row)
                                        let bf_cards = Self::get_battlefield_cards_in_order(view, view.player_id());
                                        if !bf_cards.is_empty() {
                                            let current_card = self.state.selected_card_in_your_bf;
                                            if let Some(current_idx) =
                                                current_card.and_then(|id| bf_cards.iter().position(|&c| c == id))
                                            {
                                                const CARDS_PER_ROW: usize = 4;
                                                let new_idx = current_idx + CARDS_PER_ROW;
                                                if new_idx < bf_cards.len() {
                                                    let new_card = bf_cards[new_idx];
                                                    self.state.selected_card_in_your_bf = Some(new_card);
                                                    self.state.selected_card_id = Some(new_card);
                                                }
                                            }
                                        }
                                        return Ok(InputAction::Continue);
                                    }
                                    FocusedPane::OpponentBattlefield => {
                                        // Navigate cards in Opponent Battlefield (2D: move down one row)
                                        if let Some(opp_id) = view.opponents().next() {
                                            let bf_cards = Self::get_battlefield_cards_in_order(view, opp_id);
                                            if !bf_cards.is_empty() {
                                                let current_card = self.state.selected_card_in_opp_bf;
                                                if let Some(current_idx) =
                                                    current_card.and_then(|id| bf_cards.iter().position(|&c| c == id))
                                                {
                                                    const CARDS_PER_ROW: usize = 4;
                                                    let new_idx = current_idx + CARDS_PER_ROW;
                                                    if new_idx < bf_cards.len() {
                                                        let new_card = bf_cards[new_idx];
                                                        self.state.selected_card_in_opp_bf = Some(new_card);
                                                        self.state.selected_card_id = Some(new_card);
                                                    }
                                                }
                                            }
                                        }
                                        return Ok(InputAction::Continue);
                                    }
                                    _ => {
                                        return Ok(InputAction::Continue);
                                    }
                                }
                            }
                            KeyCode::Left => {
                                match self.state.focused_pane {
                                    FocusedPane::YourBattlefield => {
                                        // Navigate left in Your Battlefield (2D: move left with wrapping)
                                        let bf_cards = Self::get_battlefield_cards_in_order(view, view.player_id());
                                        if !bf_cards.is_empty() {
                                            let current_card = self.state.selected_card_in_your_bf;
                                            if let Some(current_idx) =
                                                current_card.and_then(|id| bf_cards.iter().position(|&c| c == id))
                                            {
                                                const CARDS_PER_ROW: usize = 4;
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
                                                self.state.selected_card_in_your_bf = Some(new_card);
                                                self.state.selected_card_id = Some(new_card);
                                            }
                                        }
                                        return Ok(InputAction::Continue);
                                    }
                                    FocusedPane::OpponentBattlefield => {
                                        // Navigate left in Opponent Battlefield (2D: move left with wrapping)
                                        if let Some(opp_id) = view.opponents().next() {
                                            let bf_cards = Self::get_battlefield_cards_in_order(view, opp_id);
                                            if !bf_cards.is_empty() {
                                                let current_card = self.state.selected_card_in_opp_bf;
                                                if let Some(current_idx) =
                                                    current_card.and_then(|id| bf_cards.iter().position(|&c| c == id))
                                                {
                                                    const CARDS_PER_ROW: usize = 4;
                                                    let row = current_idx / CARDS_PER_ROW;
                                                    let col = current_idx % CARDS_PER_ROW;

                                                    let new_idx = if col > 0 {
                                                        current_idx - 1
                                                    } else {
                                                        let row_end = ((row + 1) * CARDS_PER_ROW).min(bf_cards.len());
                                                        row_end - 1
                                                    };

                                                    let new_card = bf_cards[new_idx];
                                                    self.state.selected_card_in_opp_bf = Some(new_card);
                                                    self.state.selected_card_id = Some(new_card);
                                                }
                                            }
                                        }
                                        return Ok(InputAction::Continue);
                                    }
                                    _ => {
                                        return Ok(InputAction::Continue);
                                    }
                                }
                            }
                            KeyCode::Right => {
                                match self.state.focused_pane {
                                    FocusedPane::YourBattlefield => {
                                        // Navigate right in Your Battlefield (2D: move right with wrapping)
                                        let bf_cards = Self::get_battlefield_cards_in_order(view, view.player_id());
                                        if !bf_cards.is_empty() {
                                            let current_card = self.state.selected_card_in_your_bf;
                                            if let Some(current_idx) =
                                                current_card.and_then(|id| bf_cards.iter().position(|&c| c == id))
                                            {
                                                const CARDS_PER_ROW: usize = 4;
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
                                                self.state.selected_card_in_your_bf = Some(new_card);
                                                self.state.selected_card_id = Some(new_card);
                                            }
                                        }
                                        return Ok(InputAction::Continue);
                                    }
                                    FocusedPane::OpponentBattlefield => {
                                        // Navigate right in Opponent Battlefield (2D: move right with wrapping)
                                        if let Some(opp_id) = view.opponents().next() {
                                            let bf_cards = Self::get_battlefield_cards_in_order(view, opp_id);
                                            if !bf_cards.is_empty() {
                                                let current_card = self.state.selected_card_in_opp_bf;
                                                if let Some(current_idx) =
                                                    current_card.and_then(|id| bf_cards.iter().position(|&c| c == id))
                                                {
                                                    const CARDS_PER_ROW: usize = 4;
                                                    let row = current_idx / CARDS_PER_ROW;
                                                    let row_start = row * CARDS_PER_ROW;
                                                    let row_end = ((row + 1) * CARDS_PER_ROW).min(bf_cards.len());

                                                    let new_idx = if current_idx + 1 < row_end {
                                                        current_idx + 1
                                                    } else {
                                                        row_start
                                                    };

                                                    let new_card = bf_cards[new_idx];
                                                    self.state.selected_card_in_opp_bf = Some(new_card);
                                                    self.state.selected_card_id = Some(new_card);
                                                }
                                            }
                                        }
                                        return Ok(InputAction::Continue);
                                    }
                                    _ => {
                                        return Ok(InputAction::Continue);
                                    }
                                }
                            }
                            KeyCode::Enter => {
                                // Enter only selects when Actions pane is focused
                                if self.state.focused_pane == FocusedPane::Actions {
                                    return Ok(InputAction::Select(self.state.highlighted_choice));
                                }
                                // TODO: In other panes, Enter could select a card for details
                                return Ok(InputAction::Continue);
                            }
                            KeyCode::Char('p') | KeyCode::Esc => {
                                return Ok(InputAction::Pass);
                            }
                            KeyCode::Char('q') => {
                                return Ok(InputAction::Pass);
                            }
                            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                return Ok(InputAction::Exit);
                            }
                            KeyCode::Char('z') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                // TODO: Full suspend/resume (SIGTSTP/SIGCONT) would require signal-hook crate
                                // For now, treat Ctrl-Z as graceful exit
                                return Ok(InputAction::Exit);
                            }
                            KeyCode::Char(c) if c.is_ascii_digit() => {
                                // Digit selection only works when Actions pane is focused
                                if self.state.focused_pane == FocusedPane::Actions {
                                    let digit = c.to_digit(10).unwrap() as usize;
                                    if digit < num_choices {
                                        return Ok(InputAction::Select(digit));
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    /// Show a choice prompt and get user selection
    fn prompt_for_choice(
        &mut self,
        view: &GameStateView,
        prompt: &str,
        choices: &[String],
    ) -> io::Result<Option<usize>> {
        self.state.highlighted_choice = 0;

        let mut terminal = Self::setup_terminal()?;

        loop {
            // Prepare choices with highlighting and numbers
            let choice_tuples: Vec<(String, bool)> = choices
                .iter()
                .enumerate()
                .map(|(idx, text)| {
                    let numbered_text = format!("[{}] {}", idx, text);
                    (numbered_text, idx == self.state.highlighted_choice)
                })
                .collect();

            terminal.draw(|f| {
                self.draw_ui(f, view, Some(prompt), &choice_tuples);
            })?;

            match self.wait_for_choice_input(choices.len(), view)? {
                InputAction::Continue => {
                    // Arrow key pressed, continue loop to redraw
                    continue;
                }
                InputAction::Select(choice) => {
                    Self::restore_terminal(&mut terminal)?;
                    return Ok(Some(choice));
                }
                InputAction::Pass => {
                    Self::restore_terminal(&mut terminal)?;
                    return Ok(None);
                }
                InputAction::Exit => {
                    // Ctrl-C pressed - restore terminal and exit gracefully
                    Self::restore_terminal(&mut terminal)?;
                    eprintln!("Exiting game (Ctrl-C pressed)");
                    std::process::exit(0);
                }
            }
        }
    }
}

impl PlayerController for FancyTuiController {
    fn player_id(&self) -> PlayerId {
        self.player_id
    }

    fn choose_spell_ability_to_play(
        &mut self,
        view: &GameStateView,
        available: &[SpellAbility],
    ) -> Option<SpellAbility> {
        if available.is_empty() {
            return None;
        }

        // Set choice context and valid choices for highlighting
        self.state.choice_context = ChoiceContext::PlayingSpell;
        self.state.valid_choices = available
            .iter()
            .map(|ability| match ability {
                SpellAbility::PlayLand { card_id } => *card_id,
                SpellAbility::CastSpell { card_id } => *card_id,
                SpellAbility::ActivateAbility { card_id, .. } => *card_id,
            })
            .collect();

        let player_name = view.player_name();
        let prompt = format!("Priority {}: Choose action", player_name);

        let choices: Vec<String> = std::iter::once("Pass".to_string())
            .chain(available.iter().map(|ability| match ability {
                SpellAbility::PlayLand { card_id } => {
                    let name = view.card_name(*card_id).unwrap_or_default();
                    format!("Play land: {}", name)
                }
                SpellAbility::CastSpell { card_id } => {
                    let name = view.card_name(*card_id).unwrap_or_default();
                    format!("Cast spell: {}", name)
                }
                SpellAbility::ActivateAbility { card_id, .. } => {
                    let name = view.card_name(*card_id).unwrap_or_default();
                    format!("Activate: {}", name)
                }
            }))
            .collect();

        let result = match self.prompt_for_choice(view, &prompt, &choices) {
            Ok(Some(0)) | Ok(None) => None, // Pass
            Ok(Some(idx)) if idx > 0 && idx <= available.len() => Some(available[idx - 1].clone()),
            _ => None,
        };

        // Clear choice context after making choice
        self.state.choice_context = ChoiceContext::None;
        self.state.valid_choices.clear();

        result
    }

    fn choose_targets(
        &mut self,
        view: &GameStateView,
        spell: CardId,
        valid_targets: &[CardId],
    ) -> SmallVec<[CardId; 4]> {
        if valid_targets.is_empty() {
            return SmallVec::new();
        }

        // Set choice context and valid choices for highlighting
        self.state.choice_context = ChoiceContext::TargetSelection;
        self.state.valid_choices = valid_targets.to_vec();

        let spell_name = view.card_name(spell).unwrap_or_default();
        let prompt = format!("Choose target for: {}", spell_name);

        // Count how many times each card name appears (to detect duplicates)
        let mut name_counts: HashMap<String, usize> = HashMap::new();
        for &card_id in valid_targets {
            let name = view.card_name(card_id).unwrap_or_default();
            *name_counts.entry(name).or_insert(0) += 1;
        }

        let choices: Vec<String> = std::iter::once("No target".to_string())
            .chain(valid_targets.iter().map(|&card_id| {
                let name = view.card_name(card_id).unwrap_or_default();

                // Determine ownership
                let controller = view.get_card(card_id).map(|c| c.controller);
                let ownership = if controller == Some(self.player_id) {
                    "(yours)"
                } else {
                    "(theirs)"
                };

                // Show ID only if there are duplicates of this card name
                let id_part = if *name_counts.get(&name).unwrap_or(&0) > 1 {
                    format!(" #{}", card_id.as_u32())
                } else {
                    String::new()
                };

                let tapped = if view.is_tapped(card_id) { " [T]" } else { "" };
                format!("{}{}{} {}", name, id_part, tapped, ownership)
            }))
            .collect();

        let mut targets = SmallVec::new();
        match self.prompt_for_choice(view, &prompt, &choices) {
            Ok(Some(idx)) if idx > 0 && idx <= valid_targets.len() => {
                targets.push(valid_targets[idx - 1]);
            }
            _ => {}
        }

        // Clear choice context after making choice
        self.state.choice_context = ChoiceContext::None;
        self.state.valid_choices.clear();

        targets
    }

    fn choose_mana_sources_to_pay(
        &mut self,
        view: &GameStateView,
        cost: &ManaCost,
        available_sources: &[CardId],
    ) -> SmallVec<[CardId; 8]> {
        let mut sources = SmallVec::new();
        let needed = cost.cmc() as usize;

        if needed == 0 || available_sources.is_empty() {
            return sources;
        }

        for i in 0..needed {
            let prompt = format!("Pay mana {}/{}: Select source", i + 1, needed);
            let choices: Vec<String> = available_sources
                .iter()
                .map(|&card_id| view.card_name(card_id).unwrap_or_else(|| format!("{:?}", card_id)))
                .collect();

            match self.prompt_for_choice(view, &prompt, &choices) {
                Ok(Some(idx)) if idx < available_sources.len() => {
                    sources.push(available_sources[idx]);
                }
                _ => break,
            }
        }

        sources
    }

    fn choose_attackers(&mut self, view: &GameStateView, available_creatures: &[CardId]) -> SmallVec<[CardId; 8]> {
        if available_creatures.is_empty() {
            return SmallVec::new();
        }

        // Set choice context and valid choices for highlighting
        self.state.choice_context = ChoiceContext::DeclareAttackers;
        self.state.valid_choices = available_creatures.to_vec();

        let mut attackers = SmallVec::new();

        loop {
            let prompt = "Declare Attackers (select creatures or Done)";
            let choices: Vec<String> = std::iter::once("Done".to_string())
                .chain(available_creatures.iter().map(|&card_id| {
                    let name = view.card_name(card_id).unwrap_or_default();
                    let selected = if attackers.contains(&card_id) { " [X]" } else { "" };
                    format!("{}{}", name, selected)
                }))
                .collect();

            match self.prompt_for_choice(view, prompt, &choices) {
                Ok(Some(0)) | Ok(None) => break,
                Ok(Some(idx)) if idx > 0 && idx <= available_creatures.len() => {
                    let card_id = available_creatures[idx - 1];
                    if !attackers.contains(&card_id) {
                        attackers.push(card_id);
                    }
                }
                _ => break,
            }
        }

        // Clear choice context after making choice
        self.state.choice_context = ChoiceContext::None;
        self.state.valid_choices.clear();

        attackers
    }

    fn choose_blockers(
        &mut self,
        view: &GameStateView,
        available_blockers: &[CardId],
        attackers: &[CardId],
    ) -> SmallVec<[(CardId, CardId); 8]> {
        if attackers.is_empty() || available_blockers.is_empty() {
            return SmallVec::new();
        }

        // Set choice context: both blockers and attackers are valid choices
        self.state.choice_context = ChoiceContext::DeclareBlockers;
        self.state.valid_choices = available_blockers.iter().chain(attackers.iter()).copied().collect();

        let mut blocks = SmallVec::new();

        // Count how many times each attacker name appears (to detect duplicates)
        let mut name_counts: HashMap<String, usize> = HashMap::new();
        for &card_id in attackers {
            let name = view.card_name(card_id).unwrap_or_default();
            *name_counts.entry(name).or_insert(0) += 1;
        }

        // For each blocker, ask which attacker to block
        for &blocker_id in available_blockers {
            let blocker_name = view.card_name(blocker_id).unwrap_or_default();
            let prompt = format!("{}: Block which attacker?", blocker_name);

            let choices: Vec<String> = std::iter::once("Skip".to_string())
                .chain(attackers.iter().map(|&attacker_id| {
                    let name = view.card_name(attacker_id).unwrap_or_default();

                    // Show ID only if there are duplicates of this card name
                    let id_part = if *name_counts.get(&name).unwrap_or(&0) > 1 {
                        format!(" #{}", attacker_id.as_u32())
                    } else {
                        String::new()
                    };

                    format!("{}{}", name, id_part)
                }))
                .collect();

            match self.prompt_for_choice(view, &prompt, &choices) {
                Ok(Some(0)) | Ok(None) => continue,
                Ok(Some(idx)) if idx > 0 && idx <= attackers.len() => {
                    blocks.push((blocker_id, attackers[idx - 1]));
                }
                _ => break,
            }
        }

        // Clear choice context after making choice
        self.state.choice_context = ChoiceContext::None;
        self.state.valid_choices.clear();

        blocks
    }

    fn choose_damage_assignment_order(
        &mut self,
        _view: &GameStateView,
        _attacker: CardId,
        blockers: &[CardId],
    ) -> SmallVec<[CardId; 4]> {
        // For simplicity, just return blockers in order
        // TODO: implement UI for reordering
        blockers.iter().copied().collect()
    }

    fn choose_cards_to_discard(
        &mut self,
        view: &GameStateView,
        hand: &[CardId],
        count: usize,
    ) -> SmallVec<[CardId; 7]> {
        let mut discards = SmallVec::new();

        for i in 0..count {
            let prompt = format!("Discard card {}/{}", i + 1, count);
            let choices: Vec<String> = hand
                .iter()
                .filter(|&card_id| !discards.contains(card_id))
                .map(|&card_id| view.card_name(card_id).unwrap_or_default())
                .collect();

            if choices.is_empty() {
                break;
            }

            match self.prompt_for_choice(view, &prompt, &choices) {
                Ok(Some(idx)) if idx < hand.len() => {
                    let card_id = hand
                        .iter()
                        .filter(|&card_id| !discards.contains(card_id))
                        .nth(idx)
                        .copied();
                    if let Some(card_id) = card_id {
                        discards.push(card_id);
                    }
                }
                _ => break,
            }
        }

        discards
    }

    fn on_priority_passed(&mut self, _view: &GameStateView) {
        // Logging is handled by the game logger, no local state tracking needed
    }

    fn on_game_end(&mut self, _view: &GameStateView, _won: bool) {
        // Logging is handled by the game logger, no local state tracking needed
    }

    fn get_controller_type(&self) -> crate::game::snapshot::ControllerType {
        // Fancy TUI is treated as a variant of the TUI controller
        crate::game::snapshot::ControllerType::Tui
    }
}
