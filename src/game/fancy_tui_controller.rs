//! Fancy TUI controller with full-screen ratatui interface
//!
//! This controller provides a rich, multi-panel TUI interface similar to MTG Arena,
//! with separate panels for battlefield, hand, card details, prompts, and game state.

use crate::core::{CardId, ManaCost, PlayerId, SpellAbility};
use crate::game::controller::{GameStateView, PlayerController};
use crate::game::Step;
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
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

/// Tab indices for left panels
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
enum LeftTab {
    Stack = 0,
    Combat = 1,
    Log = 2,
}

/// Tab indices for bottom-left panels
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
enum BottomLeftTab {
    Prompt = 0,
    Dock = 1,
}

/// Currently focused pane for keyboard navigation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FocusedPane {
    /// (H)and pane
    Hand,
    /// (I)nfo pane (Stack/Combat/Log)
    Info,
    /// (Y)our battlefield
    YourBattlefield,
    /// (O)pponent battlefield
    OpponentBattlefield,
    /// (A)ctions pane (Prompt/Dock)
    Actions,
}

/// Application state for the fancy TUI
struct FancyTuiState {
    /// Currently selected left tab
    left_tab: LeftTab,
    /// Currently selected bottom-left tab
    bottom_left_tab: BottomLeftTab,
    /// Currently highlighted choice index (if in choice mode)
    highlighted_choice: usize,
    /// Currently selected card for details view
    selected_card_id: Option<CardId>,
    /// Whether logger was configured for memory-only mode
    logger_memory_mode_enabled: bool,
    /// Currently focused pane
    focused_pane: FocusedPane,
    /// Selected card index in hand (for navigation) - reserved for future use
    #[allow(dead_code)]
    selected_card_in_hand: Option<usize>,
    /// Selected card in your battlefield (for navigation) - reserved for future use
    #[allow(dead_code)]
    selected_card_in_your_bf: Option<CardId>,
    /// Selected card in opponent battlefield (for navigation) - reserved for future use
    #[allow(dead_code)]
    selected_card_in_opp_bf: Option<CardId>,
}

impl FancyTuiState {
    fn new() -> Self {
        Self {
            left_tab: LeftTab::Stack,
            bottom_left_tab: BottomLeftTab::Prompt,
            highlighted_choice: 0,
            selected_card_id: None,
            logger_memory_mode_enabled: false,
            focused_pane: FocusedPane::Actions, // Start with Actions focused
            selected_card_in_hand: None,
            selected_card_in_your_bf: None,
            selected_card_in_opp_bf: None,
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
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        Terminal::new(backend)
    }

    /// Restore the terminal to normal mode
    fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> io::Result<()> {
        disable_raw_mode()?;
        execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
        terminal.show_cursor()?;
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

    /// Draw the complete UI with all panels
    fn draw_ui(
        &self,
        f: &mut Frame,
        view: &GameStateView,
        current_prompt: Option<&str>,
        choices: &[(String, bool)], // (text, is_highlighted)
    ) {
        // Main layout: 3 columns
        let main_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(25), // Left panels
                Constraint::Percentage(50), // Battlefields
                Constraint::Percentage(25), // Right panels (Hand + Card Details)
            ])
            .split(f.area());

        // Left column: split into top tabs and bottom tabs
        let left_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(60), // Stack|Combat|Log tabs
                Constraint::Percentage(40), // Prompt|Dock tabs
            ])
            .split(main_chunks[0]);

        // Right column: split into Card Details and Hand
        let right_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(50), // Card Details
                Constraint::Percentage(50), // Hand
            ])
            .split(main_chunks[2]);

        // Draw all panels
        self.draw_left_tabs(f, left_chunks[0], view);
        self.draw_bottom_left_tabs(f, left_chunks[1], view, current_prompt, choices);
        self.draw_battlefields(f, main_chunks[1], view);
        self.draw_card_details(f, right_chunks[0], view);
        self.draw_hand(f, right_chunks[1], view);
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

        let titles = vec!["Stack", "Combat", "Log"];
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
            LeftTab::Stack => self.draw_stack_view(f, content_area, view),
            LeftTab::Combat => self.draw_combat_view(f, content_area, view),
            LeftTab::Log => self.draw_log_view(f, content_area, view),
        }
    }

    /// Draw the stack view
    fn draw_stack_view(&self, f: &mut Frame, area: Rect, _view: &GameStateView) {
        let text = Text::from("(Stack empty)");
        let paragraph = Paragraph::new(text).wrap(Wrap { trim: true });
        f.render_widget(paragraph, area);
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

    /// Draw the bottom-left tabbed panel (Prompt|Dock)
    fn draw_bottom_left_tabs(
        &self,
        f: &mut Frame,
        area: Rect,
        view: &GameStateView,
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

        let titles = vec!["Prompt", "Dock"];
        let tabs = Tabs::new(titles)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(title)
                    .border_style(Style::default().fg(border_color)),
            )
            .select(self.state.bottom_left_tab as usize)
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

        match self.state.bottom_left_tab {
            BottomLeftTab::Prompt => self.draw_prompt_view(f, content_area, view, current_prompt, choices),
            BottomLeftTab::Dock => self.draw_dock_view(f, content_area, view),
        }
    }

    /// Draw the prompt view with choices
    fn draw_prompt_view(
        &self,
        f: &mut Frame,
        area: Rect,
        _view: &GameStateView,
        current_prompt: Option<&str>,
        choices: &[(String, bool)],
    ) {
        if let Some(prompt) = current_prompt {
            // Show prompt text
            let prompt_text = Text::from(prompt);
            let prompt_height = 3; // Reserve lines for prompt
            let prompt_area = Rect {
                x: area.x,
                y: area.y,
                width: area.width,
                height: prompt_height.min(area.height),
            };
            let paragraph = Paragraph::new(prompt_text)
                .wrap(Wrap { trim: true })
                .style(Style::default().fg(Color::Cyan));
            f.render_widget(paragraph, prompt_area);

            // Show choices below prompt
            if !choices.is_empty() {
                let choices_area = Rect {
                    x: area.x,
                    y: area.y + prompt_height,
                    width: area.width,
                    height: area.height.saturating_sub(prompt_height),
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
            f.render_widget(paragraph, area);
        }
    }

    /// Draw the dock view (for future expansion - card library, etc.)
    fn draw_dock_view(&self, f: &mut Frame, area: Rect, _view: &GameStateView) {
        let text = Text::from("(Dock - future feature)");
        let paragraph = Paragraph::new(text).style(Style::default().fg(Color::DarkGray));
        f.render_widget(paragraph, area);
    }

    /// Draw both battlefields (opponent on top, player on bottom)
    fn draw_battlefields(&self, f: &mut Frame, area: Rect, view: &GameStateView) {
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

        let mut phase_spans = vec![Span::raw(format!("Turn: {}, ", turn_number))];

        for (i, step) in all_steps.iter().enumerate() {
            let abbrev = Self::step_abbrev(*step);
            let span = if *step == current_step {
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
        // Phase text without underline formatting for length calc
        let phase_text_plain = format!("Turn: {}, UP UK DR M1 BC DA DB CD EC M2 ET", turn_number);
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
    fn draw_battlefield(&self, f: &mut Frame, area: Rect, view: &GameStateView, owner_id: PlayerId, _title: &str) {
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

        // Render card groups
        let mut y_offset = 0;

        if !lands.is_empty() {
            y_offset += self.render_card_group(f, inner_area, y_offset, view, &lands, "Lands", Color::Green);
        }

        if !creatures.is_empty() {
            y_offset += self.render_card_group(f, inner_area, y_offset, view, &creatures, "Creatures", Color::Red);
        }

        if !others.is_empty() {
            self.render_card_group(f, inner_area, y_offset, view, &others, "Other", Color::Blue);
        }

        if player_cards.is_empty() {
            let empty_text = Text::from("(Empty)");
            let paragraph = Paragraph::new(empty_text)
                .style(Style::default().fg(Color::DarkGray))
                .alignment(Alignment::Center);
            f.render_widget(paragraph, inner_area);
        }
    }

    /// Render a group of cards (lands, creatures, others)
    #[allow(clippy::too_many_arguments)]
    fn render_card_group(
        &self,
        f: &mut Frame,
        area: Rect,
        y_offset: u16,
        view: &GameStateView,
        cards: &[CardId],
        label: &str,
        color: Color,
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

        // Card dimensions for 2D layout
        const CARD_WIDTH: u16 = 10;
        const CARD_HEIGHT: u16 = 7;
        const CARD_SPACING: u16 = 1;

        let mut rendered_height = 1; // Start after label

        // Calculate how many cards fit per row
        let cards_per_row = ((area.width + CARD_SPACING) / (CARD_WIDTH + CARD_SPACING)).max(1);

        // Calculate how many rows we need
        let num_cards = cards.len();
        let num_rows = (num_cards as u16).div_ceil(cards_per_row).max(1);

        // Render cards in 2D grid
        for (card_index, &card_id) in cards.iter().enumerate() {
            let row = (card_index as u16) / cards_per_row;
            let col = (card_index as u16) % cards_per_row;

            let card_x = area.x + col * (CARD_WIDTH + CARD_SPACING);
            let card_y = area.y + y_offset + rendered_height + row * (CARD_HEIGHT + CARD_SPACING);

            // Check if this card fits in the available space
            if card_y + CARD_HEIGHT > area.y + area.height {
                break;
            }

            let card_area = Rect {
                x: card_x,
                y: card_y,
                width: CARD_WIDTH,
                height: CARD_HEIGHT,
            };

            self.render_card_box(f, card_area, view, card_id);
        }

        // Total height used by this group
        let rows_rendered =
            num_rows.min(((area.height - y_offset - rendered_height) + CARD_SPACING) / (CARD_HEIGHT + CARD_SPACING));
        rendered_height += rows_rendered * (CARD_HEIGHT + CARD_SPACING);

        rendered_height
    }

    /// Render a single card as a box
    fn render_card_box(&self, f: &mut Frame, area: Rect, view: &GameStateView, card_id: CardId) {
        let name = view.card_name(card_id).unwrap_or_else(|| format!("{:?}", card_id));
        let is_tapped = view.is_tapped(card_id);

        let card = view.get_card(card_id);

        // Build multiline content for the card
        let mut lines = Vec::new();

        // Line 1: Card name (truncated to fit width)
        let max_name_len = area.width.saturating_sub(4) as usize; // Account for borders + padding
        let display_name = if name.len() > max_name_len {
            format!("{}...", &name[..max_name_len.saturating_sub(3)])
        } else {
            name.clone()
        };
        lines.push(Line::from(display_name));

        // Line 2: Tapped status or empty line
        if is_tapped {
            lines.push(Line::from("[TAPPED]"));
        } else {
            lines.push(Line::from(""));
        }

        // Line 3-4: P/T for creatures, or mana cost for other spells
        if let Some(card) = &card {
            if card.is_creature() {
                let pt_line = format!("{}/{}", card.power.unwrap_or(0), card.toughness.unwrap_or(0));
                lines.push(Line::from(""));
                lines.push(Line::from(pt_line));
            }
        }

        // Determine text style (dimmed if tapped)
        let text_style = if is_tapped {
            Style::default().fg(Color::Gray)
        } else {
            Style::default().fg(Color::White)
        };

        // Determine border color based on card colors
        let border_color = if let Some(card) = card {
            match card.colors.len() {
                0 => Color::Gray, // Colorless
                1 => match card.colors[0] {
                    crate::core::Color::Red => Color::Red,
                    crate::core::Color::Green => Color::Green,
                    crate::core::Color::Blue => Color::Blue,
                    crate::core::Color::White => Color::White,
                    crate::core::Color::Black => Color::DarkGray,
                    crate::core::Color::Colorless => Color::Gray,
                },
                _ => Color::Yellow, // Multicolor
            }
        } else {
            Color::Gray // Fallback if card not found
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color))
            .style(text_style);

        let text = Text::from(lines);
        let paragraph = Paragraph::new(text).block(block).alignment(Alignment::Center);
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
                    lines.push(Line::from(card.text.clone()));
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

        if hand.is_empty() {
            let text = Text::from("(Empty)");
            let paragraph = Paragraph::new(text)
                .style(Style::default().fg(Color::DarkGray))
                .alignment(Alignment::Center);
            f.render_widget(paragraph, inner_area);
            return;
        }

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

                ListItem::new(format!("[{}] {}{}", idx, name, cost))
            })
            .collect();

        let list = List::new(items).style(Style::default().fg(Color::White));
        f.render_widget(list, inner_area);
    }

    /// Wait for user input and update highlighted choice
    fn wait_for_choice_input(&mut self, num_choices: usize) -> io::Result<InputAction> {
        loop {
            if event::poll(std::time::Duration::from_millis(100))? {
                if let Event::Key(key) = event::read()? {
                    match key.code {
                        // Pane focus switching (H, I, Y, O, A)
                        KeyCode::Char('h') | KeyCode::Char('H') => {
                            self.state.focused_pane = FocusedPane::Hand;
                            return Ok(InputAction::Continue); // Redraw needed
                        }
                        KeyCode::Char('i') | KeyCode::Char('I') => {
                            self.state.focused_pane = FocusedPane::Info;
                            return Ok(InputAction::Continue); // Redraw needed
                        }
                        KeyCode::Char('y') | KeyCode::Char('Y') => {
                            self.state.focused_pane = FocusedPane::YourBattlefield;
                            return Ok(InputAction::Continue); // Redraw needed
                        }
                        KeyCode::Char('o') | KeyCode::Char('O') => {
                            self.state.focused_pane = FocusedPane::OpponentBattlefield;
                            return Ok(InputAction::Continue); // Redraw needed
                        }
                        KeyCode::Char('a') | KeyCode::Char('A') => {
                            self.state.focused_pane = FocusedPane::Actions;
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
                                _ => {
                                    // TODO: Navigate cards in other panes (future feature)
                                    // For now, just redraw without change
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
                                _ => {
                                    // TODO: Navigate cards in other panes (future feature)
                                    // For now, just redraw without change
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

            match self.wait_for_choice_input(choices.len())? {
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

        match self.prompt_for_choice(view, &prompt, &choices) {
            Ok(Some(0)) | Ok(None) => None, // Pass
            Ok(Some(idx)) if idx > 0 && idx <= available.len() => Some(available[idx - 1].clone()),
            _ => None,
        }
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
