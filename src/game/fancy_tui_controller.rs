//! Fancy TUI controller with full-screen ratatui interface
//!
//! This controller provides a rich, multi-panel TUI interface similar to MTG Arena,
//! with separate panels for battlefield, hand, card details, prompts, and game state.

use crate::core::{CardId, ManaCost, PlayerId, SpellAbility};
use crate::game::controller::{GameStateView, PlayerController};
use crossterm::{
    event::{self, Event, KeyCode},
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
use std::io;

/// Input action result from user interaction
enum InputAction {
    /// Continue - need to redraw UI (arrow key pressed)
    Continue,
    /// Select a specific choice index
    Select(usize),
    /// Pass/cancel the choice
    Pass,
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

/// Application state for the fancy TUI
struct FancyTuiState {
    /// Currently selected left tab
    left_tab: LeftTab,
    /// Currently selected bottom-left tab
    bottom_left_tab: BottomLeftTab,
    /// Currently highlighted choice index (if in choice mode)
    highlighted_choice: usize,
    /// Game log messages
    log_messages: Vec<String>,
    /// Currently selected card for details view
    selected_card_id: Option<CardId>,
}

impl FancyTuiState {
    fn new() -> Self {
        Self {
            left_tab: LeftTab::Stack,
            bottom_left_tab: BottomLeftTab::Prompt,
            highlighted_choice: 0,
            log_messages: Vec::new(),
            selected_card_id: None,
        }
    }

    fn add_log_message(&mut self, msg: String) {
        self.log_messages.push(msg);
        // Keep only last 100 messages
        if self.log_messages.len() > 100 {
            self.log_messages.remove(0);
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
        let titles = vec!["Stack", "Combat", "Log"];
        let tabs = Tabs::new(titles)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Info")
                    .border_style(Style::default().fg(Color::Gray)),
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
            LeftTab::Log => self.draw_log_view(f, content_area),
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
    fn draw_log_view(&self, f: &mut Frame, area: Rect) {
        let items: Vec<ListItem> = self
            .state
            .log_messages
            .iter()
            .rev()
            .take(area.height as usize)
            .map(|msg| ListItem::new(msg.as_str()))
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
        let titles = vec!["Prompt", "Dock"];
        let tabs = Tabs::new(titles)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Actions")
                    .border_style(Style::default().fg(Color::Gray)),
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

        let info_text = format!(
            "{}: {} life | Hand: {} | GY: {} | Lib: {}",
            player_label, life, hand_size, graveyard_size, library_size
        );

        let paragraph = Paragraph::new(info_text)
            .style(Style::default().fg(Color::White).add_modifier(Modifier::BOLD))
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Gray)),
            );

        f.render_widget(paragraph, area);
    }

    /// Draw a single battlefield
    fn draw_battlefield(&self, f: &mut Frame, area: Rect, view: &GameStateView, owner_id: PlayerId, title: &str) {
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

        let block = Block::default()
            .borders(Borders::ALL)
            .title(title)
            .border_style(Style::default().fg(Color::Gray));
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

        let mut rendered_height = 1;

        // Render each card
        for &card_id in cards.iter().take(((area.height - y_offset - 1) / 3) as usize) {
            let card_y = area.y + y_offset + rendered_height;
            if card_y + 2 >= area.y + area.height {
                break;
            }

            let card_area = Rect {
                x: area.x + 2,
                y: card_y,
                width: area.width.saturating_sub(2),
                height: 3,
            };

            self.render_card_box(f, card_area, view, card_id);
            rendered_height += 3;
        }

        rendered_height
    }

    /// Render a single card as a box
    fn render_card_box(&self, f: &mut Frame, area: Rect, view: &GameStateView, card_id: CardId) {
        let name = view.card_name(card_id).unwrap_or_else(|| format!("{:?}", card_id));
        let is_tapped = view.is_tapped(card_id);

        let card = view.get_card(card_id);
        let pt_text = card
            .filter(|c| c.is_creature())
            .map(|c| format!(" {}/{}", c.power.unwrap_or(0), c.toughness.unwrap_or(0)))
            .unwrap_or_default();

        let card_text = if is_tapped {
            format!("[T] {}{}", name, pt_text)
        } else {
            format!("{}{}", name, pt_text)
        };

        let style = if is_tapped {
            Style::default().fg(Color::Gray)
        } else {
            Style::default().fg(Color::White)
        };

        let block = Block::default().borders(Borders::ALL).style(style);
        let paragraph = Paragraph::new(card_text).block(block).wrap(Wrap { trim: true });
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

        let block = Block::default()
            .borders(Borders::ALL)
            .title(format!("Hand ({})", hand.len()))
            .border_style(Style::default().fg(Color::Gray));

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
                        KeyCode::Up | KeyCode::Char('k') => {
                            if self.state.highlighted_choice > 0 {
                                self.state.highlighted_choice -= 1;
                            }
                            return Ok(InputAction::Continue); // Redraw needed
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            if self.state.highlighted_choice + 1 < num_choices {
                                self.state.highlighted_choice += 1;
                            }
                            return Ok(InputAction::Continue); // Redraw needed
                        }
                        KeyCode::Enter => {
                            return Ok(InputAction::Select(self.state.highlighted_choice));
                        }
                        KeyCode::Char('p') | KeyCode::Esc => {
                            return Ok(InputAction::Pass);
                        }
                        KeyCode::Char('q') => {
                            return Ok(InputAction::Pass);
                        }
                        KeyCode::Char(c) if c.is_ascii_digit() => {
                            let digit = c.to_digit(10).unwrap() as usize;
                            if digit < num_choices {
                                return Ok(InputAction::Select(digit));
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

        let choices: Vec<String> = std::iter::once("No target".to_string())
            .chain(valid_targets.iter().map(|&card_id| {
                let name = view.card_name(card_id).unwrap_or_default();
                let tapped = if view.is_tapped(card_id) { " (T)" } else { "" };
                format!("{}{}", name, tapped)
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

        // For each blocker, ask which attacker to block
        for &blocker_id in available_blockers {
            let blocker_name = view.card_name(blocker_id).unwrap_or_default();
            let prompt = format!("{}: Block which attacker?", blocker_name);

            let choices: Vec<String> = std::iter::once("Skip".to_string())
                .chain(
                    attackers
                        .iter()
                        .map(|&attacker_id| view.card_name(attacker_id).unwrap_or_default()),
                )
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
        self.state.add_log_message("Priority passed".to_string());
    }

    fn on_game_end(&mut self, view: &GameStateView, won: bool) {
        let msg = format!(
            "Game Over: You {}! Final life: {}",
            if won { "WON" } else { "LOST" },
            view.life()
        );
        self.state.add_log_message(msg);
    }

    fn get_controller_type(&self) -> crate::game::snapshot::ControllerType {
        // Fancy TUI is treated as a variant of the TUI controller
        crate::game::snapshot::ControllerType::Tui
    }
}
