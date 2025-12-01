//! Shared TUI rendering logic for Fancy TUI modes
//!
//! This module provides a renderer that can work with any ratatui backend,
//! allowing both interactive terminal rendering (CrosstermBackend) and
//! screenshot capture (TestBackend) to share the same rendering code.

use crate::core::{CardId, PlayerId};
use crate::game::controller::GameStateView;
use crate::game::Step;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, List, ListItem, Paragraph, Tabs, Wrap},
    Frame,
};
use smallvec::SmallVec;
use std::collections::HashMap;

/// Tab indices for left panels (Combat|Log only)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum LeftTab {
    Combat = 0,
    Log = 1,
}

/// Currently focused pane for keyboard navigation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusedPane {
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

/// Context for what kind of choice is being made
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChoiceContext {
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

/// Trait for entities that can be rendered on the battlefield
pub trait BattlefieldEntity {
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
pub enum Entity {
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
pub struct EntityPosition {
    pub entity: Entity,
    pub area: Rect,
}

/// Application state for the fancy TUI
pub struct FancyTuiState {
    /// Currently selected left tab
    pub left_tab: LeftTab,
    /// Currently highlighted choice index (if in choice mode)
    pub highlighted_choice: usize,
    /// Currently selected card for details view
    pub selected_card_id: Option<CardId>,
    /// Whether logger was configured for memory-only mode
    pub logger_memory_mode_enabled: bool,
    /// Currently focused pane
    pub focused_pane: FocusedPane,
    /// Selected card index in hand (for navigation)
    pub selected_card_in_hand: Option<usize>,
    /// Selected card in your battlefield (for navigation)
    pub selected_card_in_your_bf: Option<CardId>,
    /// Selected card in opponent battlefield (for navigation)
    pub selected_card_in_opp_bf: Option<CardId>,
    /// Entity positions for mouse hit testing (cleared and rebuilt each frame)
    pub entity_positions: Vec<EntityPosition>,
    /// Cards that can currently be chosen (for highlighting)
    pub valid_choices: Vec<CardId>,
    /// What kind of choice is being made
    pub choice_context: ChoiceContext,
    /// Actions pane area (for mouse click detection)
    pub actions_pane_area: Option<Rect>,
    /// Hand pane area (for mouse click detection)
    pub hand_pane_area: Option<Rect>,
    /// Rewind message to display after undo operation
    pub rewind_message: Option<String>,
}

impl FancyTuiState {
    pub fn new() -> Self {
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

impl Default for FancyTuiState {
    fn default() -> Self {
        Self::new()
    }
}

/// Shared TUI renderer that works with any ratatui backend
pub struct TuiRenderer {
    /// TUI state
    state: FancyTuiState,
    /// Whether to use visual stacking (diagonal offsets) or simple stacking
    visual_stacks: bool,
}

impl TuiRenderer {
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

    // Card size constants
    const DEFAULT_CARD_WIDTH: u16 = 10;
    const DEFAULT_CARD_HEIGHT: u16 = 7;
    const MIN_CARD_WIDTH: u16 = 5;
    const MIN_CARD_HEIGHT: u16 = 4;
    const MAX_CARD_HEIGHT: u16 = 15;
    const CARD_SPACING: u16 = 1;

    /// Create a new TUI renderer
    pub fn new(visual_stacks: bool) -> Self {
        TuiRenderer {
            state: FancyTuiState::new(),
            visual_stacks,
        }
    }

    /// Get mutable reference to state (for controllers to update)
    pub fn state_mut(&mut self) -> &mut FancyTuiState {
        &mut self.state
    }

    /// Get reference to state
    pub fn state(&self) -> &FancyTuiState {
        &self.state
    }

    /// Get abbreviated phase name for display
    fn step_abbrev(step: Step) -> &'static str {
        match step {
            Step::Untap => "UP",
            Step::Upkeep => "UK",
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

    /// Compute card width from height while maintaining the default aspect ratio
    fn compute_width_from_height(height: u16) -> u16 {
        ((height as f32 * Self::DEFAULT_CARD_WIDTH as f32) / Self::DEFAULT_CARD_HEIGHT as f32).round() as u16
    }

    /// Get card dimensions based on tapped state and base size
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
    fn get_dimensions_for_tapped_state(is_tapped: bool, base_width: u16, base_height: u16) -> (u16, u16) {
        if is_tapped {
            let tapped_width = (base_width * 3 / 2).max(base_width);
            let tapped_height = (base_height * 3 / 5).max(4);
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
                const DIAGONAL_OFFSET: u16 = 1;
                let stack_depth = card_ids.len() as u16;
                let offset_total = (stack_depth.saturating_sub(1)) * DIAGONAL_OFFSET;

                let any_tapped = *tapped_count > 0;
                let (base_w, base_h) = Self::get_dimensions_for_tapped_state(any_tapped, base_width, base_height);

                (base_w + offset_total, base_h + offset_total)
            }
        }
    }

    /// Test if all cards fit in the battlefield area with given card size
    fn test_card_size_fits(
        area: Rect,
        card_groups: &[(Vec<CardId>, &str)],
        view: &GameStateView,
        card_width: u16,
        card_height: u16,
    ) -> bool {
        let mut y_offset = 0u16;

        for (cards, _label) in card_groups {
            if y_offset >= area.height {
                return false;
            }

            y_offset += 1;

            let mut current_x = 0u16;
            let mut row_height = 0u16;

            for &card_id in cards {
                let (card_w, card_h) = Self::get_card_dimensions_with_size(view, card_id, card_width, card_height);

                if current_x + card_w > area.width && current_x > 0 {
                    current_x = 0;
                    y_offset += row_height + Self::CARD_SPACING;
                    row_height = 0;

                    if y_offset >= area.height {
                        return false;
                    }
                }

                if y_offset + card_h > area.height {
                    return false;
                }

                current_x += card_w + Self::CARD_SPACING;
                row_height = row_height.max(card_h);
            }

            if current_x > 0 {
                y_offset += row_height;
            }
        }

        true
    }

    /// Calculate optimal card size for battlefield
    fn calculate_optimal_card_size(
        area: Rect,
        card_groups: &[(Vec<CardId>, &str)],
        view: &GameStateView,
    ) -> (u16, u16) {
        if Self::test_card_size_fits(
            area,
            card_groups,
            view,
            Self::DEFAULT_CARD_WIDTH,
            Self::DEFAULT_CARD_HEIGHT,
        ) {
            let mut height = Self::DEFAULT_CARD_HEIGHT;
            let mut width = Self::DEFAULT_CARD_WIDTH;

            loop {
                let next_height = height + 1;

                if next_height > Self::MAX_CARD_HEIGHT {
                    break;
                }

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
            let mut height = Self::DEFAULT_CARD_HEIGHT;
            let mut width = Self::DEFAULT_CARD_WIDTH;

            while !Self::test_card_size_fits(area, card_groups, view, width, height) && height > Self::MIN_CARD_HEIGHT {
                height -= 1;
                width = Self::compute_width_from_height(height).max(Self::MIN_CARD_WIDTH);
            }

            (width.max(Self::MIN_CARD_WIDTH), height.max(Self::MIN_CARD_HEIGHT))
        }
    }

    /// Calculate maximum mana production from battlefield
    fn calculate_max_mana(view: &GameStateView) -> (u8, u8, u8, u8, u8, u8, u8) {
        view.max_mana_capacity()
    }

    /// Group cards into entities for rendering
    fn group_cards_into_entities(&self, cards: &[CardId], view: &GameStateView) -> Vec<Entity> {
        let mut groups: HashMap<String, SmallVec<[CardId; 8]>> = HashMap::new();

        for &card_id in cards {
            let name = view.card_name(card_id).unwrap_or_else(|| format!("{:?}", card_id));
            groups.entry(name).or_default().push(card_id);
        }

        let construct_entities = |card_name: String, mut card_ids: SmallVec<[CardId; 8]>| -> Vec<Entity> {
            if self.visual_stacks {
                if card_ids.len() > 1 {
                    card_ids.sort_by_key(|&id| view.is_tapped(id));

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
                let (tapped, untapped): (SmallVec<[CardId; 8]>, SmallVec<[CardId; 8]>) =
                    card_ids.into_iter().partition(|&id| view.is_tapped(id));

                let mut result = Vec::new();

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

        let mut entities: Vec<Entity> = groups
            .into_iter()
            .flat_map(|(card_name, card_ids)| construct_entities(card_name, card_ids))
            .collect();

        entities.sort_by_key(|e| match e {
            Entity::SingleCard { card_id } => (0, *card_id),
            Entity::VisualStack { card_ids, .. } => (1, card_ids[0]),
            Entity::SimpleStack { card_ids, .. } => (1, card_ids[0]),
        });

        entities
    }

    /// Draw the complete UI
    pub fn draw_ui(
        &mut self,
        f: &mut Frame<'_>,
        view: &GameStateView,
        current_prompt: Option<&str>,
        choices: &[(String, bool)],
    ) {
        // Clear entity positions and pane areas from previous frame
        self.state.entity_positions.clear();
        self.state.actions_pane_area = None;
        self.state.hand_pane_area = None;

        let total_width = f.area().width;

        let boosted_left_width = (total_width * Self::BOOSTED_LEFT_COLUMN_PCT) / 100;
        let boosted_middle_width = (total_width * (Self::DEFAULT_MIDDLE_COLUMN_PCT - 5)) / 100;
        let boosted_right_width = total_width.saturating_sub(boosted_left_width + boosted_middle_width);

        let can_boost = boosted_left_width >= Self::MIN_WIDTH_INFO_PANE
            && boosted_left_width >= Self::MIN_WIDTH_ACTIONS_PANE
            && boosted_middle_width >= Self::MIN_WIDTH_BATTLEFIELD
            && boosted_right_width >= Self::MIN_WIDTH_CARD_DETAILS
            && boosted_right_width >= Self::MIN_WIDTH_HAND
            && boosted_right_width >= Self::MIN_WIDTH_STACK;

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

        let main_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(left_pct),
                Constraint::Percentage(middle_pct),
                Constraint::Percentage(right_pct),
            ])
            .split(f.area());

        let left_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
            .split(main_chunks[0]);

        let right_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(33),
                Constraint::Percentage(33),
                Constraint::Percentage(34),
            ])
            .split(main_chunks[2]);

        self.draw_left_tabs(f, left_chunks[0], view);
        self.draw_prompt(f, left_chunks[1], view, current_prompt, choices);
        self.state.actions_pane_area = Some(left_chunks[1]);

        self.draw_battlefields(f, main_chunks[1], view);
        self.draw_card_details(f, right_chunks[0], view);
        self.draw_stack(f, right_chunks[1], view);
        self.draw_hand(f, right_chunks[2], view);
        self.state.hand_pane_area = Some(right_chunks[2]);
    }

    fn draw_left_tabs(&self, f: &mut Frame<'_>, area: Rect, view: &GameStateView) {
        let is_focused = self.state.focused_pane == FocusedPane::Info;
        let border_color = if is_focused { Color::White } else { Color::Gray };

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

    fn draw_combat_view(&self, f: &mut Frame<'_>, area: Rect, view: &GameStateView) {
        let combat = view.combat();

        if !combat.combat_active {
            let text = Text::from("(No combat)");
            let paragraph = Paragraph::new(text).wrap(Wrap { trim: true });
            f.render_widget(paragraph, area);
            return;
        }

        let mut lines = Vec::new();

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

    fn draw_log_view(&self, f: &mut Frame<'_>, area: Rect, view: &GameStateView) {
        let logs = view.logger().logs();

        let available_lines = area.height.saturating_sub(2) as usize;
        let start_idx = logs.len().saturating_sub(available_lines);

        let max_line_number = logs.len();
        let line_number_width = if max_line_number > 0 {
            max_line_number.to_string().len()
        } else {
            1
        };

        let items: Vec<ListItem> = logs
            .iter()
            .skip(start_idx)
            .enumerate()
            .map(|(idx, entry)| {
                let line_number = start_idx + idx + 1;
                let line_num_str = format!("{:>width$} ", line_number, width = line_number_width);
                let line = Line::from(vec![
                    Span::styled(line_num_str, Style::default().fg(Color::DarkGray)),
                    Span::raw(&entry.message),
                ]);
                ListItem::new(line)
            })
            .collect();

        let list = List::new(items);
        f.render_widget(list, area);
    }

    #[allow(clippy::too_many_lines)]
    fn draw_prompt(
        &mut self,
        f: &mut Frame<'_>,
        area: Rect,
        view: &GameStateView,
        current_prompt: Option<&str>,
        choices: &[(String, bool)],
    ) {
        let is_focused = self.state.focused_pane == FocusedPane::Actions;
        let border_color = if is_focused { Color::White } else { Color::Gray };

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

        if let Some(prompt) = current_prompt {
            let prompt_text = Text::from(prompt);
            let prompt_height = 3;
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

        let action_count = view.action_count();
        let choice_count = view.choice_count();
        let status_text = if let Some(ref msg) = self.state.rewind_message {
            format!("{} actions, {} choices in game | {}", action_count, choice_count, msg)
        } else {
            format!("{} actions, {} choices in game", action_count, choice_count)
        };

        let status_paragraph = Paragraph::new(status_text).style(Style::default().fg(Color::DarkGray));
        f.render_widget(status_paragraph, status_area);
    }

    fn draw_battlefields(&mut self, f: &mut Frame<'_>, area: Rect, view: &GameStateView) {
        let bf_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(3),
                Constraint::Percentage(45),
                Constraint::Percentage(45),
                Constraint::Min(3),
            ])
            .split(area);

        let opponent_id = view.opponents().next();
        if let Some(opp_id) = opponent_id {
            self.draw_player_info(f, bf_chunks[0], view, opp_id);
            self.draw_battlefield(f, bf_chunks[1], view, opp_id, "Opponent Battlefield");
        }

        self.draw_battlefield(f, bf_chunks[2], view, view.player_id(), "Your Battlefield");
        self.draw_player_info(f, bf_chunks[3], view, view.player_id());
    }

    #[allow(clippy::too_many_lines)]
    fn draw_player_info(&self, f: &mut Frame<'_>, area: Rect, view: &GameStateView, player_id: PlayerId) {
        let life = view.player_life(player_id);
        let hand_size = view.player_hand(player_id).len();
        let graveyard_size = view.player_graveyard(player_id).len();
        let library_size = view.player_library(player_id).len();

        let player_label = if player_id == view.player_id() { "You" } else { "Opp" };

        let stats_text = format!(
            "{}: {} life | Hand: {} | GY: {} | Lib: {}",
            player_label, life, hand_size, graveyard_size, library_size
        );

        let turn_number = view.turn_number();
        let current_step = view.current_step();
        let active_player = view.active_player();
        let is_active = player_id == active_player;

        let is_first_player = (active_player == player_id) == (turn_number % 2 == 1);
        let player_turn = if is_first_player {
            turn_number.div_ceil(2)
        } else {
            turn_number / 2
        };

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

        let turn_display = if is_active {
            player_turn.to_string()
        } else {
            "_".to_string()
        };

        let mut phase_spans = vec![Span::raw(format!("Turn: {} ({}) | ", turn_display, turn_number))];

        for (i, step) in all_steps.iter().enumerate() {
            let abbrev = Self::step_abbrev(*step);
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

        let inner_width = area.width.saturating_sub(4);
        let stats_len = stats_text.len() as u16;
        let phase_text_plain = format!(
            "Turn: {} ({}) | UP UK DR M1 BC DA DB CD EC M2 ET",
            turn_display, turn_number
        );
        let phase_len = phase_text_plain.len() as u16;

        const MIN_SPACING: u16 = 3;
        let fits_on_one_line = stats_len + phase_len + MIN_SPACING <= inner_width;

        let text = if fits_on_one_line {
            let padding = inner_width.saturating_sub(stats_len + phase_len);
            let mut line_spans = vec![Span::raw(stats_text)];
            line_spans.push(Span::raw(" ".repeat(padding as usize)));
            line_spans.extend(phase_spans);
            Text::from(Line::from(line_spans))
        } else {
            let stats_line = Line::from(Span::raw(stats_text));
            let phase_line = Line::from(phase_spans);
            Text::from(vec![stats_line, phase_line])
        };

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

    #[allow(clippy::too_many_lines)]
    fn draw_battlefield(
        &mut self,
        f: &mut Frame<'_>,
        area: Rect,
        view: &GameStateView,
        owner_id: PlayerId,
        _title: &str,
    ) {
        let battlefield = view.battlefield();

        log::debug!(target: "tui", "draw_battlefield for player {}: battlefield has {} cards total",
            owner_id.as_u32(), battlefield.len());

        let player_cards: Vec<CardId> = battlefield
            .iter()
            .filter(|&&card_id| {
                let matches = view
                    .get_card(card_id)
                    .map(|c| {
                        let is_match = c.controller == owner_id;
                        if is_match || c.name.as_str().contains("Peter Porker") {
                            log::debug!(target: "tui", "  Card {} (id={}): controller={} owner_id={} match={}",
                                c.name, card_id.as_u32(), c.controller.as_u32(), owner_id.as_u32(), is_match);
                        }
                        is_match
                    })
                    .unwrap_or(false);
                matches
            })
            .copied()
            .collect();

        log::debug!(target: "tui", "After filtering: player {} has {} cards",
            owner_id.as_u32(), player_cards.len());

        let (lands, creatures, artifacts, enchantments): (Vec<_>, Vec<_>, Vec<_>, Vec<_>) = player_cards.iter().fold(
            (Vec::new(), Vec::new(), Vec::new(), Vec::new()),
            |(mut lands, mut creatures, mut artifacts, mut enchantments), &card_id| {
                if let Some(card) = view.get_card(card_id) {
                    log::debug!(target: "tui", "Categorizing card {} (id={}): land={} creature={} artifact={} enchant={}",
                        card.name, card_id.as_u32(), card.is_land(), card.is_creature(), card.is_artifact(), card.is_enchantment());

                    if card.is_land() {
                        lands.push(card_id);
                    } else if card.is_creature() {
                        creatures.push(card_id);
                    } else if card.is_artifact() {
                        artifacts.push(card_id);
                    } else if card.is_enchantment() {
                        enchantments.push(card_id);
                    }
                }
                (lands, creatures, artifacts, enchantments)
            },
        );

        log::debug!(target: "tui", "After categorization: player {} has {} lands, {} creatures, {} artifacts, {} enchantments",
            owner_id.as_u32(), lands.len(), creatures.len(), artifacts.len(), enchantments.len());

        let is_player_bf = owner_id == view.player_id();
        let is_focused = if is_player_bf {
            self.state.focused_pane == FocusedPane::YourBattlefield
        } else {
            self.state.focused_pane == FocusedPane::OpponentBattlefield
        };
        let border_color = if is_focused { Color::White } else { Color::Gray };

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

        let (card_width, card_height) = Self::calculate_optimal_card_size(inner_area, &card_groups, view);

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

    #[allow(clippy::too_many_arguments)]
    fn render_card_group(
        &mut self,
        f: &mut Frame<'_>,
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

        let entities = self.group_cards_into_entities(cards, view);

        // Check if there's enough space for at least one row of cards
        let remaining_height = area.height.saturating_sub(y_offset + 1); // -1 for label
        let first_row_height = entities
            .first()
            .map(|e| Self::get_entity_dimensions(e, view, card_width, card_height).1)
            .unwrap_or(card_height);

        // If no space for cards, render compact text-only summary
        if remaining_height < first_row_height {
            return self.render_compact_card_group(f, area, y_offset, view, cards, label, color);
        }

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

        let mut rows: Vec<Vec<(&Entity, u16, u16)>> = Vec::new();
        let mut current_row: Vec<(&Entity, u16, u16)> = Vec::new();
        let mut current_row_width = 0u16;

        for entity in &entities {
            let (card_w, card_h) = Self::get_entity_dimensions(entity, view, card_width, card_height);

            let entity_width_with_spacing = card_w + Self::CARD_SPACING;

            if !current_row.is_empty() && current_row_width + card_w > area.width {
                rows.push(current_row);
                current_row = Vec::new();
                current_row_width = 0;
            }

            current_row.push((entity, card_w, card_h));
            current_row_width += entity_width_with_spacing;
        }

        if !current_row.is_empty() {
            rows.push(current_row);
        }

        let mut current_y = area.y + y_offset + rendered_height;

        for row in &rows {
            if row.is_empty() {
                continue;
            }

            let row_width: u16 =
                row.iter().map(|(_, w, _)| w).sum::<u16>() + (row.len().saturating_sub(1) as u16 * Self::CARD_SPACING);

            let row_height: u16 = row.iter().map(|(_, _, h)| *h).max().unwrap_or(0);

            if current_y + row_height > area.y + area.height {
                break;
            }

            let x_offset = (area.width.saturating_sub(row_width)) / 2;
            let mut current_x = area.x + x_offset;

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

        if !rows.is_empty() {
            rendered_height = current_y - (area.y + y_offset);
        }

        rendered_height
    }

    /// Render a compact text-only summary of cards when there's no space for card graphics
    #[allow(clippy::too_many_arguments)]
    fn render_compact_card_group(
        &self,
        f: &mut Frame<'_>,
        area: Rect,
        y_offset: u16,
        view: &GameStateView,
        cards: &[CardId],
        label: &str,
        color: Color,
    ) -> u16 {
        use std::collections::HashMap;

        // Count cards by name
        let mut card_counts: HashMap<String, usize> = HashMap::new();
        for &card_id in cards {
            let name = view.card_name(card_id).unwrap_or_else(|| format!("{:?}", card_id));
            *card_counts.entry(name).or_insert(0) += 1;
        }

        // Build compact summary: "Creatures: Spider-Ham x1, Bear x2"
        let mut summary_parts: Vec<String> = card_counts
            .iter()
            .map(|(name, count)| {
                // Truncate long names to fit
                let short_name = if name.len() > 12 {
                    format!("{}..", &name[..10])
                } else {
                    name.clone()
                };
                if *count > 1 {
                    format!("{}x{}", short_name, count)
                } else {
                    short_name
                }
            })
            .collect();
        summary_parts.sort(); // Consistent ordering

        let summary = summary_parts.join(", ");
        let full_text = format!("{}: {}", label, summary);

        // Truncate if too long for available width
        let display_text = if full_text.len() > area.width as usize {
            format!("{}...", &full_text[..area.width.saturating_sub(3) as usize])
        } else {
            full_text
        };

        let compact_area = Rect {
            x: area.x,
            y: area.y + y_offset,
            width: area.width,
            height: 1,
        };

        let text = Text::from(Span::styled(display_text, Style::default().fg(color)));
        f.render_widget(Paragraph::new(text), compact_area);

        1 // Only takes 1 line of height
    }

    fn render_visual_stack(&mut self, f: &mut Frame<'_>, area: Rect, view: &GameStateView, entity: &Entity) {
        let Entity::VisualStack {
            card_ids, tapped_count, ..
        } = entity
        else {
            return;
        };

        const DIAGONAL_OFFSET: u16 = 1;
        let stack_depth = card_ids.len();
        let card_id = card_ids[0];
        let card = view.get_card(card_id);

        let is_selected =
            Some(card_id) == self.state.selected_card_in_your_bf || Some(card_id) == self.state.selected_card_in_opp_bf;

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

        for i in 0..stack_depth {
            let offset = i as u16 * DIAGONAL_OFFSET;

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

            if i < stack_depth - 1 {
                let border_style = Style::default().fg(border_color);
                let block = Block::default().borders(Borders::ALL).border_style(border_style);
                f.render_widget(block, card_area);
            } else {
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

                let name = entity.display_name(view);
                let cost_str = card.as_ref().map(|c| c.mana_cost.to_string()).unwrap_or_default();

                let content_width = card_area.width.saturating_sub(2) as usize;
                let mut lines = Vec::new();

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

    #[allow(clippy::too_many_lines)]
    fn render_entity(&mut self, f: &mut Frame<'_>, area: Rect, view: &GameStateView, entity: &Entity) {
        self.state.entity_positions.push(EntityPosition {
            entity: entity.clone(),
            area,
        });

        if matches!(entity, Entity::VisualStack { .. }) {
            self.render_visual_stack(f, area, view, entity);
            return;
        }

        let card_id = entity.representative_card();
        let name = entity.display_name(view);
        let is_tapped = entity.is_tapped(view);
        let card = view.get_card(card_id);

        let is_selected =
            Some(card_id) == self.state.selected_card_in_your_bf || Some(card_id) == self.state.selected_card_in_opp_bf;

        let content_width = area.width.saturating_sub(2) as usize;
        let content_height = area.height.saturating_sub(2);

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

        let is_valid_choice = self.state.valid_choices.contains(&card_id);
        let has_choice_context = self.state.choice_context != ChoiceContext::None;

        let text_style = if has_choice_context {
            if is_valid_choice {
                Style::default().fg(Color::White)
            } else {
                Style::default().fg(Color::DarkGray)
            }
        } else if is_tapped {
            Style::default().fg(Color::Gray)
        } else {
            Style::default().fg(Color::White)
        };

        let mut lines = Vec::new();

        let cost_str = card.as_ref().map(|c| c.mana_cost.to_string()).unwrap_or_default();
        let cost_len = cost_str.len();

        let title_style = if is_selected {
            Style::default()
                .add_modifier(Modifier::BOLD)
                .add_modifier(Modifier::UNDERLINED)
        } else {
            Style::default()
        };

        let (multiplier_prefix, card_name_part) = if let Some(x_pos) = name.find("x ") {
            let prefix = &name[..x_pos + 1];
            let rest = &name[x_pos + 1..];
            if prefix.len() >= 2 && prefix[..prefix.len() - 1].chars().all(|c| c.is_ascii_digit()) {
                (Some(prefix.to_string()), rest.to_string())
            } else {
                (None, name.clone())
            }
        } else {
            (None, name.clone())
        };

        let name_and_cost_fit = name.len() + cost_len < content_width;
        let name_fits_alone = name.len() <= content_width;
        let have_vertical_space = content_height >= 3;

        if !cost_str.is_empty() && name_and_cost_fit {
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

        if is_tapped && lines.len() < content_height as usize {
            let tapped_text = if content_width >= 12 {
                "[TAPPED]"
            } else if content_width >= 3 {
                "[T]"
            } else {
                "T"
            };
            lines.push(Line::from(tapped_text));
        }

        let is_creature = card.as_ref().map(|c| c.is_creature()).unwrap_or(false);
        let pt_str = if is_creature {
            card.as_ref()
                .map(|c| format!("{}/{}", c.current_power(), c.current_toughness()))
                .unwrap_or_default()
        } else {
            String::new()
        };

        let reserve_last_line_for_pt = is_creature && !pt_str.is_empty() && pt_str.len() <= content_width;

        let max_total_lines = if reserve_last_line_for_pt {
            content_height.saturating_sub(1) as usize
        } else {
            content_height as usize
        };

        if let Some(card) = card.as_ref() {
            if !card.text.is_empty() && lines.len() < max_total_lines {
                let available_lines = max_total_lines.saturating_sub(lines.len());
                let desc_lines = card.text.split('\n').collect::<Vec<_>>();

                for (i, desc_line) in desc_lines.iter().enumerate().take(available_lines) {
                    if i == available_lines - 1 && (i < desc_lines.len() - 1 || desc_line.len() > content_width) {
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

        while lines.len() < max_total_lines {
            lines.push(Line::from(""));
        }

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

    fn draw_card_details(&self, f: &mut Frame<'_>, area: Rect, view: &GameStateView) {
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
                        card.current_power(),
                        card.current_toughness()
                    )));
                }

                if !card.text.is_empty() {
                    lines.push(Line::from(""));
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

        let text = Text::from("(No card selected)");
        let paragraph = Paragraph::new(text).style(Style::default().fg(Color::DarkGray));
        f.render_widget(paragraph, inner_area);
    }

    fn draw_hand(&self, f: &mut Frame<'_>, area: Rect, view: &GameStateView) {
        let hand = view.hand();

        let is_focused = self.state.focused_pane == FocusedPane::Hand;
        let border_color = if is_focused { Color::White } else { Color::Gray };

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

        let (total, w, u, b, r, g, c) = Self::calculate_max_mana(view);
        let mana_text = format!("Max Mana: {} ~= {}W {}U {}B {}R {}G {}C", total, w, u, b, r, g, c);
        let mana_paragraph = Paragraph::new(mana_text).style(Style::default().fg(Color::Cyan));
        f.render_widget(mana_paragraph, mana_area);
    }

    fn draw_stack(&self, f: &mut Frame<'_>, area: Rect, view: &GameStateView) {
        let is_focused = self.state.focused_pane == FocusedPane::Stack;
        let border_color = if is_focused { Color::White } else { Color::Gray };

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

        let stack = view.stack();
        if stack.is_empty() {
            let text = Text::from("(Stack empty)");
            let paragraph = Paragraph::new(text).style(Style::default().fg(Color::DarkGray));
            f.render_widget(paragraph, inner_area);
        } else {
            let mut lines = Vec::new();
            for (idx, &card_id) in stack.iter().enumerate() {
                let card_name = view.card_name(card_id).unwrap_or_else(|| format!("Card {:?}", card_id));

                let position = if idx == stack.len() - 1 {
                    "TOP"
                } else if idx == 0 {
                    "BOT"
                } else {
                    "   "
                };

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
}
