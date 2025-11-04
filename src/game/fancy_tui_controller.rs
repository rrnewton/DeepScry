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
use signal_hook::consts::signal::{SIGCONT, SIGTSTP};
use signal_hook::flag as signal_flag;
use smallvec::SmallVec;
use std::collections::HashMap;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

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
    /// Undo the most recent action (Z key pressed)
    Undo,
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

/// Trait for entities that can be rendered on the battlefield
trait BattlefieldEntity {
    /// Get all card IDs represented by this entity
    #[allow(dead_code)]
    fn card_ids(&self) -> &[CardId];

    /// Get a representative card ID for rendering details
    fn representative_card(&self) -> CardId;

    /// Get the count of cards in this entity (1 for single cards, N for stacks)
    #[allow(dead_code)]
    fn count(&self) -> usize;

    /// Get the display name for this entity
    fn display_name(&self, view: &GameStateView) -> String;

    /// Check if this entity is tapped
    fn is_tapped(&self, view: &GameStateView) -> bool;

    /// Check if any card in this entity matches the given card_id
    #[allow(dead_code)]
    fn contains_card(&self, card_id: CardId) -> bool;
}

/// Concrete entity type - single card, simple stack, or visual stack
#[derive(Debug, Clone)]
enum Entity {
    SingleCard {
        card_id: CardId,
    },
    SimpleStack {
        card_ids: SmallVec<[CardId; 8]>,
        card_name: String,
        is_tapped: bool,
    },
    VisualStack {
        card_ids: SmallVec<[CardId; 8]>,
        card_name: String,
        tapped_count: usize, // How many are tapped (tapped ones on top)
    },
}

impl BattlefieldEntity for Entity {
    fn card_ids(&self) -> &[CardId] {
        match self {
            Entity::SingleCard { card_id } => std::slice::from_ref(card_id),
            Entity::SimpleStack { card_ids, .. } => card_ids,
            Entity::VisualStack { card_ids, .. } => card_ids,
        }
    }

    fn representative_card(&self) -> CardId {
        match self {
            Entity::SingleCard { card_id } => *card_id,
            Entity::SimpleStack { card_ids, .. } => card_ids[0],
            Entity::VisualStack { card_ids, .. } => card_ids[0],
        }
    }

    fn count(&self) -> usize {
        match self {
            Entity::SingleCard { .. } => 1,
            Entity::SimpleStack { card_ids, .. } => card_ids.len(),
            Entity::VisualStack { card_ids, .. } => card_ids.len(),
        }
    }

    fn display_name(&self, view: &GameStateView) -> String {
        match self {
            Entity::SingleCard { card_id } => view.card_name(*card_id).unwrap_or_else(|| format!("{:?}", card_id)),
            Entity::SimpleStack {
                card_name, card_ids, ..
            } => {
                let count = card_ids.len();
                if count > 1 {
                    format!("{}x {}", count, card_name)
                } else {
                    card_name.clone()
                }
            }
            Entity::VisualStack {
                card_name, card_ids, ..
            } => {
                // For visual stacks, don't show the "Nx" prefix since
                // the visual diagonal offsets convey the count
                let count = card_ids.len();
                if count > 1 {
                    format!("{}x {}", count, card_name)
                } else {
                    card_name.clone()
                }
            }
        }
    }

    fn is_tapped(&self, view: &GameStateView) -> bool {
        match self {
            Entity::SingleCard { card_id } => view.is_tapped(*card_id),
            Entity::SimpleStack { is_tapped, .. } => *is_tapped,
            Entity::VisualStack { tapped_count, .. } => *tapped_count > 0,
        }
    }

    fn contains_card(&self, card_id: CardId) -> bool {
        self.card_ids().contains(&card_id)
    }
}

/// Entity position for hit testing during mouse clicks
#[derive(Debug, Clone)]
struct EntityPosition {
    entity: Entity,
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
    /// Entity positions for mouse hit testing (cleared and rebuilt each frame)
    entity_positions: Vec<EntityPosition>,
    /// Cards that can currently be chosen (for highlighting)
    valid_choices: Vec<CardId>,
    /// What kind of choice is being made
    choice_context: ChoiceContext,
    /// Actions pane area (for mouse click detection)
    actions_pane_area: Option<Rect>,
    /// Hand pane area (for mouse click detection)
    hand_pane_area: Option<Rect>,
    /// Rewind message to display after undo operation
    rewind_message: Option<String>,
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
            entity_positions: Vec::new(),
            valid_choices: Vec::new(),
            choice_context: ChoiceContext::None,
            actions_pane_area: None,
            hand_pane_area: None,
            rewind_message: None,
        }
    }
}

/// A controller that provides a rich TUI interface using ratatui
pub struct FancyTuiController {
    player_id: PlayerId,
    state: FancyTuiState,
    /// Whether to use visual stacking (diagonal offsets) or simple stacking
    visual_stacks: bool,
}

impl FancyTuiController {
    // Minimum acceptable widths for each pane (in terminal columns)
    const MIN_WIDTH_INFO_PANE: u16 = 40; // Combat/Log pane (left column top)
    const MIN_WIDTH_ACTIONS_PANE: u16 = 40; // Prompt/Actions pane (left column bottom)
    const MIN_WIDTH_CARD_DETAILS: u16 = 30; // Card details pane (right column top)
    const MIN_WIDTH_HAND: u16 = 30; // Hand pane (right column middle)
    const MIN_WIDTH_STACK: u16 = 30; // Stack pane (right column bottom)
    const MIN_WIDTH_BATTLEFIELD: u16 = 60; // Battlefield pane (middle column)

    // Default column percentages
    const DEFAULT_LEFT_COLUMN_PCT: u16 = 25;
    const DEFAULT_MIDDLE_COLUMN_PCT: u16 = 50;
    const DEFAULT_RIGHT_COLUMN_PCT: u16 = 25;

    // Boosted left column percentage (20% increase)
    const BOOSTED_LEFT_COLUMN_PCT: u16 = 30; // 25 * 1.2 = 30

    /// Create a new fancy TUI controller
    pub fn new(player_id: PlayerId, visual_stacks: bool) -> io::Result<Self> {
        Ok(FancyTuiController {
            player_id,
            state: FancyTuiState::new(),
            visual_stacks,
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

        // Group cards: lands, creatures, artifacts, enchantments
        let (lands, creatures, artifacts, enchantments): (Vec<_>, Vec<_>, Vec<_>, Vec<_>) = player_cards.iter().fold(
            (Vec::new(), Vec::new(), Vec::new(), Vec::new()),
            |(mut lands, mut creatures, mut artifacts, mut enchantments), &card_id| {
                if let Some(card) = view.get_card(card_id) {
                    if card.is_land() {
                        lands.push(card_id);
                    } else if card.is_creature() {
                        creatures.push(card_id);
                    } else if card.is_artifact() {
                        artifacts.push(card_id);
                    } else if card.is_enchantment() {
                        enchantments.push(card_id);
                    }
                    // Note: Some cards might not fit any category (e.g., planeswalkers)
                    // They will be omitted for now
                }
                (lands, creatures, artifacts, enchantments)
            },
        );

        // Concatenate in display order
        let mut result = Vec::new();
        result.extend(lands);
        result.extend(creatures);
        result.extend(artifacts);
        result.extend(enchantments);
        result
    }

    /// Group cards into battlefield entities
    ///
    /// Groups cards by name, then uses a mode-specific constructor to create entities.
    /// With visual_stacks=true: creates VisualStack entities with diagonal offsets
    /// With visual_stacks=false: creates separate SimpleStack entities for tapped/untapped
    fn group_cards_into_entities(&self, cards: &[CardId], view: &GameStateView) -> Vec<Entity> {
        use std::collections::HashMap;

        // Group cards by name only
        let mut groups: HashMap<String, SmallVec<[CardId; 8]>> = HashMap::new();

        for &card_id in cards {
            let name = view.card_name(card_id).unwrap_or_else(|| format!("{:?}", card_id));
            groups.entry(name).or_default().push(card_id);
        }

        // Closure that takes tapped/untapped portions and constructs entities
        let construct_entities = |card_name: String, mut card_ids: SmallVec<[CardId; 8]>| -> Vec<Entity> {
            if self.visual_stacks {
                // Visual stacking: create one VisualStack entity
                if card_ids.len() > 1 {
                    // Sort cards: untapped first, then tapped (tapped ones will be on top visually)
                    card_ids.sort_by_key(|&id| view.is_tapped(id));

                    // Count how many are tapped
                    let tapped_count = card_ids.iter().filter(|&&id| view.is_tapped(id)).count();

                    vec![Entity::VisualStack {
                        card_ids,
                        card_name,
                        tapped_count,
                    }]
                } else {
                    vec![Entity::SingleCard { card_id: card_ids[0] }]
                }
            } else {
                // Simple stacking: create separate SimpleStack entities for tapped/untapped
                let (tapped, untapped): (SmallVec<[CardId; 8]>, SmallVec<[CardId; 8]>) =
                    card_ids.into_iter().partition(|&id| view.is_tapped(id));

                let mut result = Vec::new();

                // Create entity for untapped cards
                if !untapped.is_empty() {
                    if untapped.len() > 1 {
                        result.push(Entity::SimpleStack {
                            card_ids: untapped,
                            card_name: card_name.clone(),
                            is_tapped: false,
                        });
                    } else {
                        result.push(Entity::SingleCard { card_id: untapped[0] });
                    }
                }

                // Create entity for tapped cards
                if !tapped.is_empty() {
                    if tapped.len() > 1 {
                        result.push(Entity::SimpleStack {
                            card_ids: tapped,
                            card_name,
                            is_tapped: true,
                        });
                    } else {
                        result.push(Entity::SingleCard { card_id: tapped[0] });
                    }
                }

                result
            }
        };

        // Convert groups to entities using the closure
        let mut entities: Vec<Entity> = groups
            .into_iter()
            .flat_map(|(card_name, card_ids)| construct_entities(card_name, card_ids))
            .collect();

        // Sort for consistent ordering: single cards first, then stacks
        entities.sort_by_key(|e| match e {
            Entity::SingleCard { card_id } => (0, *card_id),
            Entity::VisualStack { card_ids, .. } => (1, card_ids[0]),
            Entity::SimpleStack { card_ids, .. } => (1, card_ids[0]),
        });

        entities
    }

    /// Draw the complete UI with all panels
    fn draw_ui(
        &mut self,
        f: &mut Frame,
        view: &GameStateView,
        current_prompt: Option<&str>,
        choices: &[(String, bool)], // (text, is_highlighted)
    ) {
        // Clear entity positions and pane areas from previous frame
        self.state.entity_positions.clear();
        self.state.actions_pane_area = None;
        self.state.hand_pane_area = None;

        // Calculate optimal column widths
        // Try to boost left column width by 20% if all panes remain above their minimums
        let total_width = f.area().width;

        // Calculate what the widths would be with boosted left column
        let boosted_left_width = (total_width * Self::BOOSTED_LEFT_COLUMN_PCT) / 100;
        let boosted_middle_width = (total_width * (Self::DEFAULT_MIDDLE_COLUMN_PCT - 5)) / 100; // Reduce middle by 5%
        let boosted_right_width = total_width.saturating_sub(boosted_left_width + boosted_middle_width);

        // Check if all panes would meet their minimum widths with boosted layout
        let can_boost = boosted_left_width >= Self::MIN_WIDTH_INFO_PANE
            && boosted_left_width >= Self::MIN_WIDTH_ACTIONS_PANE
            && boosted_middle_width >= Self::MIN_WIDTH_BATTLEFIELD
            && boosted_right_width >= Self::MIN_WIDTH_CARD_DETAILS
            && boosted_right_width >= Self::MIN_WIDTH_HAND
            && boosted_right_width >= Self::MIN_WIDTH_STACK;

        // Use boosted layout if possible, otherwise use default
        let (left_pct, middle_pct, right_pct) = if can_boost {
            (
                Self::BOOSTED_LEFT_COLUMN_PCT,
                Self::DEFAULT_MIDDLE_COLUMN_PCT - 5,
                Self::DEFAULT_RIGHT_COLUMN_PCT,
            )
        } else {
            (
                Self::DEFAULT_LEFT_COLUMN_PCT,
                Self::DEFAULT_MIDDLE_COLUMN_PCT,
                Self::DEFAULT_RIGHT_COLUMN_PCT,
            )
        };

        // Main layout: 3 columns
        let main_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(left_pct),   // Left panels
                Constraint::Percentage(middle_pct), // Battlefields
                Constraint::Percentage(right_pct),  // Right panels (Card Details + Hand + Stack)
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

        // Right column: split into Card Details, Stack, and Hand (bottom to top)
        let right_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(33), // Card Details
                Constraint::Percentage(33), // Stack
                Constraint::Percentage(34), // Hand (34% to account for rounding, on bottom)
            ])
            .split(main_chunks[2]);

        // Draw all panels
        self.draw_left_tabs(f, left_chunks[0], view);
        self.draw_prompt(f, left_chunks[1], view, current_prompt, choices);
        // Track Actions pane area for mouse clicks
        self.state.actions_pane_area = Some(left_chunks[1]);

        self.draw_battlefields(f, main_chunks[1], view);
        self.draw_card_details(f, right_chunks[0], view);
        self.draw_stack(f, right_chunks[1], view);
        self.draw_hand(f, right_chunks[2], view);
        // Track Hand pane area for mouse clicks
        self.state.hand_pane_area = Some(right_chunks[2]);
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
    fn draw_combat_view(&self, f: &mut Frame, area: Rect, view: &GameStateView) {
        let combat = view.combat();

        if !combat.combat_active {
            let text = Text::from("(No combat)");
            let paragraph = Paragraph::new(text).wrap(Wrap { trim: true });
            f.render_widget(paragraph, area);
            return;
        }

        // Display combat information
        let mut lines = Vec::new();

        // Show attackers
        let attackers = combat.attackers.iter().collect::<Vec<_>>();
        if !attackers.is_empty() {
            lines.push(Line::from(Span::styled(
                format!("Attackers ({})", attackers.len()),
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            )));

            for (&attacker_id, &defending_player) in attackers.iter() {
                let name = view
                    .card_name(attacker_id)
                    .unwrap_or_else(|| format!("Card {:?}", attacker_id));
                let defender_name = view.get_player_name_by_id(defending_player);

                // Check if blocked
                let blockers = combat.get_blockers(attacker_id);
                let blocked_info = if blockers.is_empty() {
                    Span::styled(" (unblocked)", Style::default().fg(Color::Green))
                } else {
                    Span::styled(
                        format!(" (blocked by {})", blockers.len()),
                        Style::default().fg(Color::Red),
                    )
                };

                lines.push(Line::from(vec![
                    Span::raw("  → "),
                    Span::styled(name, Style::default().fg(Color::White)),
                    Span::raw(format!(" attacking {}", defender_name)),
                    blocked_info,
                ]));
            }
        }

        // Show blockers
        let blockers = combat.blockers.iter().collect::<Vec<_>>();
        if !blockers.is_empty() {
            if !lines.is_empty() {
                lines.push(Line::from(""));
            }

            lines.push(Line::from(Span::styled(
                format!("Blockers ({})", blockers.len()),
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            )));

            for (&blocker_id, blocking_attackers) in blockers.iter() {
                let name = view
                    .card_name(blocker_id)
                    .unwrap_or_else(|| format!("Card {:?}", blocker_id));

                // Show which attacker(s) this blocker is blocking
                let attacker_names: Vec<String> = blocking_attackers
                    .iter()
                    .map(|&att_id| view.card_name(att_id).unwrap_or_else(|| format!("Card {:?}", att_id)))
                    .collect();

                let blocking_desc = if attacker_names.len() == 1 {
                    attacker_names[0].clone()
                } else {
                    attacker_names.join(", ")
                };

                lines.push(Line::from(vec![
                    Span::raw("  ← "),
                    Span::styled(name, Style::default().fg(Color::White)),
                    Span::raw(format!(" blocking {}", blocking_desc)),
                ]));
            }
        }

        let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
        f.render_widget(paragraph, area);
    }

    /// Draw the log view
    fn draw_log_view(&self, f: &mut Frame, area: Rect, view: &GameStateView) {
        // Get logs from the game logger
        let logs = view.logger().logs();

        // Take the last N logs that fit in the area, in chronological order (oldest to newest)
        // so the most recent messages appear at the bottom
        let available_lines = area.height.saturating_sub(2) as usize; // Account for borders
        let start_idx = logs.len().saturating_sub(available_lines);

        let items: Vec<ListItem> = logs
            .iter()
            .skip(start_idx)
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

        let block = Block::default()
            .borders(Borders::ALL)
            .title(title)
            .border_style(Style::default().fg(border_color));
        let inner_area = block.inner(area);
        f.render_widget(block, area);

        // Reserve 1 line at bottom for status bar
        let status_height = 1;
        let content_height = inner_area.height.saturating_sub(status_height);

        let content_area = Rect {
            x: inner_area.x,
            y: inner_area.y,
            width: inner_area.width,
            height: content_height,
        };

        let status_area = Rect {
            x: inner_area.x,
            y: inner_area.y + content_height,
            width: inner_area.width,
            height: status_height,
        };

        // Draw main content (prompt and choices)
        if let Some(prompt) = current_prompt {
            // Show prompt text
            let prompt_text = Text::from(prompt);
            let prompt_height = 3; // Reserve lines for prompt
            let prompt_area = Rect {
                x: content_area.x,
                y: content_area.y,
                width: content_area.width,
                height: prompt_height.min(content_area.height),
            };
            let paragraph = Paragraph::new(prompt_text)
                .wrap(Wrap { trim: true })
                .style(Style::default().fg(Color::Cyan));
            f.render_widget(paragraph, prompt_area);

            // Show choices below prompt
            if !choices.is_empty() {
                let choices_area = Rect {
                    x: content_area.x,
                    y: content_area.y + prompt_height,
                    width: content_area.width,
                    height: content_area.height.saturating_sub(prompt_height),
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
            f.render_widget(paragraph, content_area);
        }

        // Draw status bar at the bottom
        let action_count = view.action_count();
        let status_text = if let Some(ref msg) = self.state.rewind_message {
            format!("{} actions in game | {}", action_count, msg)
        } else {
            format!("{} actions in game", action_count)
        };

        let status_paragraph = Paragraph::new(status_text).style(Style::default().fg(Color::DarkGray));
        f.render_widget(status_paragraph, status_area);
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

        // Determine if we need to split into two lines
        // Need at least a few spaces between stats and phase info
        const MIN_SPACING: u16 = 3;
        let fits_on_one_line = stats_len + phase_len + MIN_SPACING <= inner_width;

        let text = if fits_on_one_line {
            // Single line with spacing
            let padding = inner_width.saturating_sub(stats_len + phase_len);
            let mut line_spans = vec![Span::raw(stats_text)];
            line_spans.push(Span::raw(" ".repeat(padding as usize)));
            line_spans.extend(phase_spans);
            Text::from(Line::from(line_spans))
        } else {
            // Two lines: stats on first line, turn info on second line
            let stats_line = Line::from(Span::raw(stats_text));
            let phase_line = Line::from(phase_spans);
            Text::from(vec![stats_line, phase_line])
        };

        // Bold the entire text if this is the active player
        let base_style = if is_active {
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };

        let paragraph = Paragraph::new(text).style(base_style).block(
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

        // Group cards: lands, creatures, artifacts, enchantments
        let (lands, creatures, artifacts, enchantments): (Vec<_>, Vec<_>, Vec<_>, Vec<_>) = player_cards.iter().fold(
            (Vec::new(), Vec::new(), Vec::new(), Vec::new()),
            |(mut lands, mut creatures, mut artifacts, mut enchantments), &card_id| {
                if let Some(card) = view.get_card(card_id) {
                    if card.is_land() {
                        lands.push(card_id);
                    } else if card.is_creature() {
                        creatures.push(card_id);
                    } else if card.is_artifact() {
                        artifacts.push(card_id);
                    } else if card.is_enchantment() {
                        enchantments.push(card_id);
                    }
                    // Note: Some cards might not fit any category (e.g., planeswalkers)
                    // They will be omitted for now
                }
                (lands, creatures, artifacts, enchantments)
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
        if !artifacts.is_empty() {
            card_groups.push((artifacts.clone(), "Artifacts"));
        }
        if !enchantments.is_empty() {
            card_groups.push((enchantments.clone(), "Enchants"));
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

        if !artifacts.is_empty() {
            y_offset += self.render_card_group(
                f,
                inner_area,
                y_offset,
                view,
                &artifacts,
                "Artifacts",
                Color::Cyan,
                card_width,
                card_height,
            );
        }

        if !enchantments.is_empty() {
            self.render_card_group(
                f,
                inner_area,
                y_offset,
                view,
                &enchantments,
                "Enchants",
                Color::Magenta,
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
    const MAX_CARD_HEIGHT: u16 = 15; // Prevent cards from getting too large
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
        Self::get_dimensions_for_tapped_state(is_tapped, base_width, base_height)
    }

    /// Get dimensions based on tapped state
    /// Tapped entities are rendered wider and shorter to simulate 90-degree rotation
    fn get_dimensions_for_tapped_state(is_tapped: bool, base_width: u16, base_height: u16) -> (u16, u16) {
        if is_tapped {
            // Tapped cards should be WIDER and SHORTER to simulate horizontal rotation
            // We exaggerate the dimensions to make the rotation effect clear
            // width becomes roughly 1.5x the original dimensions, height becomes ~60%
            let tapped_width = (base_width * 3 / 2).max(base_width);
            let tapped_height = (base_height * 3 / 5).max(4); // Minimum height of 4
            (tapped_width, tapped_height)
        } else {
            (base_width, base_height)
        }
    }

    /// Get dimensions for an entity (handles visual stacking diagonal offsets)
    fn get_entity_dimensions(entity: &Entity, view: &GameStateView, base_width: u16, base_height: u16) -> (u16, u16) {
        match entity {
            Entity::SingleCard { .. } | Entity::SimpleStack { .. } => {
                let is_tapped = entity.is_tapped(view);
                Self::get_dimensions_for_tapped_state(is_tapped, base_width, base_height)
            }
            Entity::VisualStack {
                card_ids, tapped_count, ..
            } => {
                // Visual stacks need extra space for diagonal offsets
                const DIAGONAL_OFFSET: u16 = 1; // chars per card in stack
                let stack_depth = card_ids.len() as u16;
                let offset_total = (stack_depth.saturating_sub(1)) * DIAGONAL_OFFSET;

                // Determine if we need tapped dimensions for the visible top cards
                let any_tapped = *tapped_count > 0;
                let (base_w, base_h) = Self::get_dimensions_for_tapped_state(any_tapped, base_width, base_height);

                (base_w + offset_total, base_h + offset_total)
            }
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

                // Stop if we've reached the maximum height
                if next_height > Self::MAX_CARD_HEIGHT {
                    break;
                }

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

        // Group cards into entities
        let entities = self.group_cards_into_entities(cards, view);

        // Pre-calculate rows to enable centering
        let mut rows: Vec<Vec<(&Entity, u16, u16)>> = Vec::new();
        let mut current_row: Vec<(&Entity, u16, u16)> = Vec::new();
        let mut current_row_width = 0u16;

        for entity in &entities {
            let (card_w, card_h) = Self::get_entity_dimensions(entity, view, card_width, card_height);

            let entity_width_with_spacing = card_w + Self::CARD_SPACING;

            // Check if entity fits on current row
            if !current_row.is_empty() && current_row_width + card_w > area.width {
                // Start new row
                rows.push(current_row);
                current_row = Vec::new();
                current_row_width = 0;
            }

            current_row.push((entity, card_w, card_h));
            current_row_width += entity_width_with_spacing;
        }

        // Add last row if not empty
        if !current_row.is_empty() {
            rows.push(current_row);
        }

        // Render rows with centering
        let mut current_y = area.y + y_offset + rendered_height;

        for row in &rows {
            if row.is_empty() {
                continue;
            }

            // Calculate total width of this row (without trailing spacing)
            let row_width: u16 =
                row.iter().map(|(_, w, _)| w).sum::<u16>() + (row.len().saturating_sub(1) as u16 * Self::CARD_SPACING);

            // Calculate row height (max height in row)
            let row_height: u16 = row.iter().map(|(_, _, h)| *h).max().unwrap_or(0);

            // Check if we have vertical space
            if current_y + row_height > area.y + area.height {
                break; // No more vertical space
            }

            // Center the row
            let x_offset = (area.width.saturating_sub(row_width)) / 2;
            let mut current_x = area.x + x_offset;

            // Render entities in this row
            for (entity, card_w, card_h) in row {
                let entity_area = Rect {
                    x: current_x,
                    y: current_y,
                    width: *card_w,
                    height: *card_h,
                };
                self.render_entity(f, entity_area, view, entity);

                current_x += card_w + Self::CARD_SPACING;
            }

            current_y += row_height + Self::CARD_SPACING;
        }

        // Total height used by this group
        if !rows.is_empty() {
            rendered_height = current_y - (area.y + y_offset);
        }

        rendered_height
    }

    /// Render a visual stack with diagonal offsets
    fn render_visual_stack(&mut self, f: &mut Frame, area: Rect, view: &GameStateView, entity: &Entity) {
        let Entity::VisualStack {
            card_ids, tapped_count, ..
        } = entity
        else {
            return;
        };

        const DIAGONAL_OFFSET: u16 = 1;
        let stack_depth = card_ids.len();
        let card_id = card_ids[0]; // Representative card
        let card = view.get_card(card_id);

        // Check if this card is currently selected
        let is_selected =
            Some(card_id) == self.state.selected_card_in_your_bf || Some(card_id) == self.state.selected_card_in_opp_bf;

        // Determine border color from card colors
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

        // Render stacked cards from back to front (bottom-left to top-right)
        for i in 0..stack_depth {
            let offset = i as u16 * DIAGONAL_OFFSET;

            // Card area with diagonal offset
            let card_area = Rect {
                x: area.x + offset,
                y: area.y + offset,
                width: area
                    .width
                    .saturating_sub(offset + DIAGONAL_OFFSET * (stack_depth - i - 1) as u16),
                height: area
                    .height
                    .saturating_sub(offset + DIAGONAL_OFFSET * (stack_depth - i - 1) as u16),
            };

            // For all but the topmost card, render only the border
            if i < stack_depth - 1 {
                let border_style = Style::default().fg(border_color);
                let block = Block::default().borders(Borders::ALL).border_style(border_style);
                f.render_widget(block, card_area);
            } else {
                // Render the top card with full content
                // Determine if the top cards are tapped
                let is_tapped = *tapped_count > 0;

                let border_style = if is_selected {
                    Style::default().fg(border_color).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(border_color)
                };

                let text_style = if is_tapped {
                    Style::default().fg(Color::Gray)
                } else {
                    Style::default().fg(Color::White)
                };

                // Build simple content for the top card
                let name = entity.display_name(view);
                let cost_str = card.as_ref().map(|c| c.mana_cost.to_string()).unwrap_or_default();

                let content_width = card_area.width.saturating_sub(2) as usize;
                let mut lines = Vec::new();

                // Title line with count prefix
                if !cost_str.is_empty() && name.len() + cost_str.len() < content_width {
                    let padding = content_width.saturating_sub(name.len() + cost_str.len());
                    lines.push(Line::from(vec![
                        Span::styled(name.clone(), text_style),
                        Span::raw(" ".repeat(padding)),
                        Span::raw(cost_str),
                    ]));
                } else {
                    lines.push(Line::from(Span::styled(name, text_style)));
                }

                let block = Block::default().borders(Borders::ALL).border_style(border_style);
                let paragraph = Paragraph::new(Text::from(lines)).block(block);
                f.render_widget(paragraph, card_area);
            }
        }
    }

    /// Render a single entity as a box with priority-based content layout
    fn render_entity(&mut self, f: &mut Frame, area: Rect, view: &GameStateView, entity: &Entity) {
        // Track entity position for mouse hit testing
        self.state.entity_positions.push(EntityPosition {
            entity: entity.clone(),
            area,
        });

        // Dispatch to visual stack renderer if applicable
        if matches!(entity, Entity::VisualStack { .. }) {
            self.render_visual_stack(f, area, view, entity);
            return;
        }

        let card_id = entity.representative_card();
        let name = entity.display_name(view);
        let is_tapped = entity.is_tapped(view);
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

        // Check if name starts with multiplier (e.g., "3x Island")
        // If so, split it and colorize the multiplier in cyan
        let (multiplier_prefix, card_name_part) = if let Some(x_pos) = name.find("x ") {
            let prefix = &name[..x_pos + 1]; // "3x"
            let rest = &name[x_pos + 1..]; // " Island"
                                           // Check if prefix is all digits followed by 'x'
            if prefix.len() >= 2 && prefix[..prefix.len() - 1].chars().all(|c| c.is_ascii_digit()) {
                (Some(prefix.to_string()), rest.to_string())
            } else {
                (None, name.clone())
            }
        } else {
            (None, name.clone())
        };

        // Strategy: Try to fit name + cost on one line
        // If name would be truncated and we have vertical space, use two lines instead
        let name_and_cost_fit = name.len() + cost_len < content_width;
        let name_fits_alone = name.len() <= content_width;
        let have_vertical_space = content_height >= 3; // Need at least 3 lines (name, cost, something else)

        if !cost_str.is_empty() && name_and_cost_fit {
            // Both fit on one line - ideal case
            let padding = content_width.saturating_sub(name.len() + cost_len);
            let mut title_spans = vec![];
            if let Some(mult) = multiplier_prefix.as_ref() {
                title_spans.push(Span::styled(mult.clone(), Style::default().fg(Color::Cyan)));
                title_spans.push(Span::styled(card_name_part.clone(), title_style));
            } else {
                title_spans.push(Span::styled(name.clone(), title_style));
            }
            title_spans.push(Span::raw(" ".repeat(padding)));
            title_spans.push(Span::raw(cost_str.clone()));
            lines.push(Line::from(title_spans));
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
            if let Some(mult) = multiplier_prefix.as_ref() {
                let card_name_truncated = if card_name_part.len() > content_width.saturating_sub(mult.len()) {
                    if content_width.saturating_sub(mult.len()) <= 5 {
                        card_name_part
                            .chars()
                            .take(content_width.saturating_sub(mult.len()))
                            .collect::<String>()
                    } else {
                        format!(
                            "{}..",
                            card_name_part
                                .chars()
                                .take(content_width.saturating_sub(mult.len() + 2))
                                .collect::<String>()
                        )
                    }
                } else {
                    card_name_part.clone()
                };
                lines.push(Line::from(vec![
                    Span::styled(mult.clone(), Style::default().fg(Color::Cyan)),
                    Span::styled(card_name_truncated, title_style),
                ]));
            } else {
                lines.push(Line::from(Span::styled(display_name, title_style)));
            }
            lines.push(Line::from(cost_str.clone()));
        } else if !cost_str.is_empty() && !name_and_cost_fit && name_fits_alone && have_vertical_space {
            // Name fits, cost doesn't fit on same line, use two lines
            if let Some(mult) = multiplier_prefix.as_ref() {
                lines.push(Line::from(vec![
                    Span::styled(mult.clone(), Style::default().fg(Color::Cyan)),
                    Span::styled(card_name_part.clone(), title_style),
                ]));
            } else {
                lines.push(Line::from(Span::styled(name.clone(), title_style)));
            }
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
            if let Some(mult) = multiplier_prefix {
                let card_name_truncated = if card_name_part.len() > content_width.saturating_sub(mult.len()) {
                    if content_width.saturating_sub(mult.len()) <= 5 {
                        card_name_part
                            .chars()
                            .take(content_width.saturating_sub(mult.len()))
                            .collect::<String>()
                    } else {
                        format!(
                            "{}..",
                            card_name_part
                                .chars()
                                .take(content_width.saturating_sub(mult.len() + 2))
                                .collect::<String>()
                        )
                    }
                } else {
                    card_name_part
                };
                lines.push(Line::from(vec![
                    Span::styled(mult, Style::default().fg(Color::Cyan)),
                    Span::styled(card_name_truncated, title_style),
                ]));
            } else {
                lines.push(Line::from(Span::styled(display_name, title_style)));
            }
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
                    // Handle both actual newlines '\n' and literal string "\\n"
                    let text_with_newlines = card.text.replace("\\n", "\n");
                    for text_line in text_with_newlines.split('\n') {
                        lines.push(Line::from(text_line.to_string()));
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
    fn draw_stack(&self, f: &mut Frame, area: Rect, view: &GameStateView) {
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

        // Display actual stack contents from game state
        let stack = view.stack();
        if stack.is_empty() {
            let text = Text::from("(Stack empty)");
            let paragraph = Paragraph::new(text).style(Style::default().fg(Color::DarkGray));
            f.render_widget(paragraph, inner_area);
        } else {
            // Display stack from bottom to top
            let mut lines = Vec::new();
            for (idx, &card_id) in stack.iter().enumerate() {
                let card_name = view.card_name(card_id).unwrap_or_else(|| format!("Card {:?}", card_id));

                // Stack position indicator (0 = bottom, N-1 = top)
                let position = if idx == stack.len() - 1 {
                    "TOP"
                } else if idx == 0 {
                    "BOT"
                } else {
                    "   "
                };

                // Highlight the top of stack
                let style = if idx == stack.len() - 1 {
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                };

                lines.push(Line::from(vec![
                    Span::styled(format!("[{}] ", position), Style::default().fg(Color::Cyan)),
                    Span::styled(card_name, style),
                ]));
            }

            let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
            f.render_widget(paragraph, inner_area);
        }
    }

    /// Wait for user input and update highlighted choice
    fn wait_for_choice_input(&mut self, num_choices: usize, view: &GameStateView) -> io::Result<InputAction> {
        // Set up signal handlers for suspend/resume
        let sigtstp_flag = Arc::new(AtomicBool::new(false));
        let sigcont_flag = Arc::new(AtomicBool::new(false));

        // Register SIGTSTP (Ctrl-Z) handler
        let _sigtstp_handle = signal_flag::register(SIGTSTP, Arc::clone(&sigtstp_flag)).map_err(io::Error::other)?;

        // Register SIGCONT (resume) handler
        let _sigcont_handle = signal_flag::register(SIGCONT, Arc::clone(&sigcont_flag)).map_err(io::Error::other)?;

        loop {
            // Check for suspend signal (Ctrl-Z)
            if sigtstp_flag.swap(false, Ordering::Relaxed) {
                // Disable raw mode and leave alternate screen
                disable_raw_mode()?;
                execute!(io::stdout(), LeaveAlternateScreen)?;

                // Send SIGSTOP to ourselves to actually suspend
                #[cfg(unix)]
                unsafe {
                    libc::raise(libc::SIGSTOP);
                }

                // When we resume (SIGCONT received), we'll continue here
            }

            // Check for resume signal
            if sigcont_flag.swap(false, Ordering::Relaxed) {
                // Re-enable raw mode and re-enter alternate screen
                enable_raw_mode()?;
                execute!(io::stdout(), EnterAlternateScreen)?;

                // Return Continue to force a redraw
                return Ok(InputAction::Continue);
            }

            if event::poll(std::time::Duration::from_millis(100))? {
                let event = event::read()?;
                match event {
                    Event::Mouse(mouse_event) => {
                        if let MouseEventKind::Down(MouseButton::Left) = mouse_event.kind {
                            let (x, y) = (mouse_event.column, mouse_event.row);

                            // Check if Actions pane was clicked
                            if let Some(actions_area) = self.state.actions_pane_area {
                                if x >= actions_area.x
                                    && x < actions_area.x + actions_area.width
                                    && y >= actions_area.y
                                    && y < actions_area.y + actions_area.height
                                {
                                    self.state.focused_pane = FocusedPane::Actions;
                                    return Ok(InputAction::Continue); // Redraw with new focus
                                }
                            }

                            // Check if Hand pane was clicked
                            if let Some(hand_area) = self.state.hand_pane_area {
                                if x >= hand_area.x
                                    && x < hand_area.x + hand_area.width
                                    && y >= hand_area.y
                                    && y < hand_area.y + hand_area.height
                                {
                                    self.state.focused_pane = FocusedPane::Hand;
                                    // Initialize selection to first card if hand not empty
                                    let hand = view.hand();
                                    if !hand.is_empty() && self.state.selected_card_in_hand.is_none() {
                                        self.state.selected_card_in_hand = Some(0);
                                        self.state.selected_card_id = Some(hand[0]);
                                    }
                                    return Ok(InputAction::Continue); // Redraw with new focus
                                }
                            }

                            // Check if any entity was clicked
                            for entity_pos in &self.state.entity_positions {
                                if x >= entity_pos.area.x
                                    && x < entity_pos.area.x + entity_pos.area.width
                                    && y >= entity_pos.area.y
                                    && y < entity_pos.area.y + entity_pos.area.height
                                {
                                    // Entity clicked! Select its representative card and show details
                                    let representative = entity_pos.entity.representative_card();
                                    self.state.selected_card_id = Some(representative);

                                    // Update battlefield selection if it's in a battlefield
                                    if let Some(card) = view.get_card(representative) {
                                        if card.controller == view.player_id() {
                                            self.state.selected_card_in_your_bf = Some(representative);
                                            self.state.focused_pane = FocusedPane::YourBattlefield;
                                        } else {
                                            self.state.selected_card_in_opp_bf = Some(representative);
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
                                // In Actions pane, select the highlighted choice
                                if self.state.focused_pane == FocusedPane::Actions {
                                    return Ok(InputAction::Select(self.state.highlighted_choice));
                                }

                                // In other panes, Enter selects a card to view in Card Details
                                match self.state.focused_pane {
                                    FocusedPane::Hand => {
                                        if let Some(idx) = self.state.selected_card_in_hand {
                                            let hand = view.hand();
                                            if idx < hand.len() {
                                                self.state.selected_card_id = Some(hand[idx]);
                                            }
                                        }
                                    }
                                    FocusedPane::YourBattlefield => {
                                        if let Some(card_id) = self.state.selected_card_in_your_bf {
                                            self.state.selected_card_id = Some(card_id);
                                        }
                                    }
                                    FocusedPane::OpponentBattlefield => {
                                        if let Some(card_id) = self.state.selected_card_in_opp_bf {
                                            self.state.selected_card_id = Some(card_id);
                                        }
                                    }
                                    FocusedPane::Stack => {
                                        // Select top of stack (most recent spell)
                                        let stack = view.stack();
                                        if !stack.is_empty() {
                                            self.state.selected_card_id = Some(stack[stack.len() - 1]);
                                        }
                                    }
                                    FocusedPane::Info | FocusedPane::Actions => {
                                        // Info pane doesn't have cards to select
                                        // Actions already handled above
                                    }
                                }

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
                                // Ctrl-Z is now handled by SIGTSTP signal handler above
                                // No action needed here - the signal handler will suspend the process
                                return Ok(InputAction::Continue);
                            }
                            KeyCode::Char('Z') => {
                                // Shift+Z: Undo the most recent action
                                return Ok(InputAction::Undo);
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
                    Event::Resize(_, _) => {
                        // Terminal was resized - trigger a redraw
                        return Ok(InputAction::Continue);
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
                InputAction::Undo => {
                    // TODO: Implement undo functionality
                    //
                    // Architecture challenge: prompt_for_choice only has &GameStateView (read-only).
                    // To implement undo, we need mutable access to GameState to call:
                    //   - game.undo() -> Result<Option<usize>> to get prior_log_size
                    //   - game.logger.truncate_to(prior_log_size)
                    //   - Set rewind message in self.state
                    //
                    // Possible solutions:
                    // 1. Modify PlayerController trait to support undo operations
                    // 2. Add a mutable GameState parameter to prompt_for_choice
                    // 3. Implement undo at the game loop level instead of controller level
                    //
                    // For now, just continue the loop (ignore the undo request)
                    self.state.rewind_message = Some("Undo not yet implemented".to_string());
                    continue;
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

        // Log the choice
        if let Some(ability) = &result {
            let choice_description = match ability {
                SpellAbility::PlayLand { card_id } => {
                    let name = view.card_name(*card_id).unwrap_or_default();
                    format!("play land: {}", name)
                }
                SpellAbility::CastSpell { card_id } => {
                    let name = view.card_name(*card_id).unwrap_or_default();
                    format!("cast spell: {}", name)
                }
                SpellAbility::ActivateAbility { card_id, .. } => {
                    let name = view.card_name(*card_id).unwrap_or_default();
                    format!("activate: {}", name)
                }
            };
            view.logger()
                .controller_choice("TUI", &format!("{} chose {}", player_name, choice_description));
        } else {
            view.logger()
                .controller_choice("TUI", &format!("{} chose to pass priority", player_name));
        }

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

        // Log the choice
        if targets.is_empty() {
            view.logger().controller_choice("TUI", "chose no target");
        } else {
            let target_names: Vec<String> = targets
                .iter()
                .map(|&card_id| view.card_name(card_id).unwrap_or_else(|| format!("{:?}", card_id)))
                .collect();
            view.logger()
                .controller_choice("TUI", &format!("chose target {}", target_names.join(", ")));
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

        // Log the choice
        if attackers.is_empty() {
            view.logger().controller_choice(
                "TUI",
                &format!(
                    "chose not to attack with {} available creatures",
                    available_creatures.len()
                ),
            );
        } else {
            view.logger().controller_choice(
                "TUI",
                &format!(
                    "chose {} attackers from {} available creatures",
                    attackers.len(),
                    available_creatures.len()
                ),
            );
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

        // Log the choice
        if blocks.is_empty() {
            view.logger().controller_choice(
                "TUI",
                &format!(
                    "chose not to block (no favorable blocks among {} blockers vs {} attackers)",
                    available_blockers.len(),
                    attackers.len()
                ),
            );
        } else {
            view.logger().controller_choice(
                "TUI",
                &format!("chose {} blockers for {} attackers", blocks.len(), attackers.len()),
            );
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
