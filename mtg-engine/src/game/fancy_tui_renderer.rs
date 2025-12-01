//! Fancy TUI renderer - shared rendering logic for ratatui interfaces
//!
//! This module provides the shared UI rendering code for the fancy TUI controller,
//! usable with both native (crossterm) and WASM (egui_ratatui) backends.
//!
//! ## Architecture
//!
//! The rendering is split from the controller to allow sharing:
//! - `FancyTuiRenderer`: Holds UI state and provides rendering methods
//! - `FancyTuiController` (native): Uses this with crossterm + blocking event loop
//! - `WasmFancyTui` (wasm): Uses this with egui_ratatui + browser event loop
//!
//! ## Design Principles
//!
//! - All rendering methods take `&mut Frame` from ratatui (backend-agnostic)
//! - No crossterm/terminal-specific code in this module
//! - State management is kept simple for serialization across JS boundary (WASM)

use crate::core::{CardId, PlayerId};
use crate::game::controller::GameStateView;
use crate::game::Step;
use crate::game::VerbosityLevel;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Tabs, Wrap},
    Frame,
};
use smallvec::SmallVec;
use std::collections::HashMap;

/// Tab indices for left panels (Combat|Log only)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// Trait for entities that can be rendered on the battlefield
pub trait BattlefieldEntity {
    /// Get all card IDs represented by this entity
    fn card_ids(&self) -> &[CardId];

    /// Get a representative card ID for rendering details
    fn representative_card(&self) -> CardId;

    /// Get the count of cards in this entity (1 for single cards, N for stacks)
    fn count(&self) -> usize;

    /// Get the display name for this entity
    fn display_name(&self, view: &GameStateView) -> String;

    /// Check if this entity is tapped
    fn is_tapped(&self, view: &GameStateView) -> bool;

    /// Check if any card in this entity matches the given card_id
    fn contains_card(&self, card_id: CardId) -> bool;
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

/// UI state for the fancy TUI renderer
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

impl Default for FancyTuiState {
    fn default() -> Self {
        Self::new()
    }
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

/// Fancy TUI renderer - provides all rendering methods
///
/// This struct holds UI state and rendering configuration.
/// It can be used with any ratatui backend.
pub struct FancyTuiRenderer {
    /// The player this renderer is for
    pub player_id: PlayerId,
    /// UI state
    pub state: FancyTuiState,
    /// Whether to use visual stacking (diagonal offsets) or simple stacking
    pub visual_stacks: bool,
}

impl FancyTuiRenderer {
    // Minimum acceptable widths for each pane (in terminal columns)
    pub const MIN_WIDTH_INFO_PANE: u16 = 40; // Combat/Log pane (left column top)
    pub const MIN_WIDTH_ACTIONS_PANE: u16 = 40; // Prompt/Actions pane (left column bottom)
    pub const MIN_WIDTH_CARD_DETAILS: u16 = 30; // Card details pane (right column top)
    pub const MIN_WIDTH_HAND: u16 = 30; // Hand pane (right column middle)
    pub const MIN_WIDTH_STACK: u16 = 30; // Stack pane (right column bottom)
    pub const MIN_WIDTH_BATTLEFIELD: u16 = 60; // Battlefield pane (middle column)

    // Default column percentages
    pub const DEFAULT_LEFT_COLUMN_PCT: u16 = 25;
    pub const DEFAULT_MIDDLE_COLUMN_PCT: u16 = 50;
    pub const DEFAULT_RIGHT_COLUMN_PCT: u16 = 25;

    // Boosted left column percentage (20% increase)
    pub const BOOSTED_LEFT_COLUMN_PCT: u16 = 30; // 25 * 1.2 = 30

    /// Create a new renderer for a player
    pub fn new(player_id: PlayerId, visual_stacks: bool) -> Self {
        FancyTuiRenderer {
            player_id,
            state: FancyTuiState::new(),
            visual_stacks,
        }
    }

    /// Get abbreviated phase name for display
    pub fn step_abbrev(step: Step) -> &'static str {
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

    /// Get all cards for a battlefield in display order (lands, creatures, others)
    pub fn get_battlefield_cards_in_order(view: &GameStateView, owner_id: PlayerId) -> Vec<CardId> {
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
    pub fn group_cards_into_entities(&self, cards: &[CardId], view: &GameStateView) -> Vec<Entity> {
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
    ///
    /// This is the main rendering entry point. It draws all panels and updates
    /// hit-testing state for mouse interactions.
    pub fn draw_ui(
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

        // Main layout: three columns
        let main_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(left_pct),
                Constraint::Percentage(middle_pct),
                Constraint::Percentage(right_pct),
            ])
            .split(f.area());

        // Left column: Info tabs (Combat/Log) on top, Actions/Prompt on bottom
        let left_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(main_chunks[0]);

        // Middle column: Opponent battlefield on top, your battlefield on bottom
        let middle_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(main_chunks[1]);

        // Right column: Card details, Hand, Stack
        let right_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(35), // Card details
                Constraint::Percentage(35), // Hand
                Constraint::Percentage(30), // Stack
            ])
            .split(main_chunks[2]);

        // Draw all panels
        self.draw_left_tabs(f, left_chunks[0], view);
        self.draw_prompt(f, left_chunks[1], view, current_prompt, choices);
        self.state.actions_pane_area = Some(left_chunks[1]);

        // Get opponent ID
        let opponent_id = view.opponents().next();

        // Draw battlefields
        if let Some(opp_id) = opponent_id {
            self.draw_battlefield(f, middle_chunks[0], view, opp_id, "Opponent");
        }
        self.draw_battlefield(f, middle_chunks[1], view, view.player_id(), "You");

        // Draw right column
        self.draw_card_details(f, right_chunks[0], view);
        self.draw_hand(f, right_chunks[1], view);
        self.state.hand_pane_area = Some(right_chunks[1]);
        self.draw_stack(f, right_chunks[2], view);
    }

    /// Draw the left column tabs (Combat/Log)
    fn draw_left_tabs(&self, f: &mut Frame, area: Rect, view: &GameStateView) {
        let is_focused = self.state.focused_pane == FocusedPane::Info;

        // Create tab titles with highlighting for selected tab
        let titles: Vec<Line> = ["Combat", "Log"]
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let style = if i == self.state.left_tab as usize {
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Gray)
                };
                Line::from(Span::styled(*t, style))
            })
            .collect();

        let tabs = Tabs::new(titles)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(if is_focused {
                        " Info (I) [FOCUSED] "
                    } else {
                        " Info (I) "
                    })
                    .border_style(if is_focused {
                        Style::default().fg(Color::Cyan)
                    } else {
                        Style::default()
                    }),
            )
            .select(self.state.left_tab as usize)
            .highlight_style(Style::default().fg(Color::Yellow));

        f.render_widget(tabs, area);

        // Draw content area below tabs
        let content_area = Rect {
            x: area.x + 1,
            y: area.y + 2,
            width: area.width.saturating_sub(2),
            height: area.height.saturating_sub(3),
        };

        match self.state.left_tab {
            LeftTab::Combat => self.draw_combat_view(f, content_area, view),
            LeftTab::Log => self.draw_log_view(f, content_area, view),
        }
    }

    /// Draw the combat view panel
    fn draw_combat_view(&self, f: &mut Frame, area: Rect, view: &GameStateView) {
        let combat = view.combat();

        let mut lines = Vec::new();

        // Show phase info
        let step_abbrev = Self::step_abbrev(view.current_step());
        lines.push(Line::from(vec![
            Span::raw("Phase: "),
            Span::styled(format!("{:?}", view.current_step()), Style::default().fg(Color::Yellow)),
            Span::raw(format!(" ({})", step_abbrev)),
        ]));

        lines.push(Line::from(""));

        if combat.combat_active {
            // Show attacking creatures
            if !combat.attackers.is_empty() {
                lines.push(Line::from(Span::styled(
                    format!("Attackers ({}):", combat.attackers.len()),
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                )));

                for &attacker_id in combat.attackers.keys() {
                    let name = view
                        .card_name(attacker_id)
                        .unwrap_or_else(|| format!("{:?}", attacker_id));

                    // Get P/T
                    let pt = if let Some(card) = view.get_card(attacker_id) {
                        let power = view
                            .get_effective_power(attacker_id)
                            .unwrap_or(card.current_power() as i32);
                        let toughness = view
                            .get_effective_toughness(attacker_id)
                            .unwrap_or(card.current_toughness() as i32);
                        format!(" {}/{}", power, toughness)
                    } else {
                        String::new()
                    };

                    // Check for blockers from attacker_blockers map
                    let blockers = combat.attacker_blockers.get(&attacker_id);

                    if blockers.is_none_or(|b| b.is_empty()) {
                        lines.push(Line::from(vec![
                            Span::raw("  "),
                            Span::styled(name, Style::default().fg(Color::Red)),
                            Span::styled(pt, Style::default().fg(Color::Gray)),
                            Span::styled(" (unblocked)", Style::default().fg(Color::DarkGray)),
                        ]));
                    } else {
                        let blocker_names: Vec<String> = blockers
                            .unwrap()
                            .iter()
                            .map(|&b| view.card_name(b).unwrap_or_else(|| format!("{:?}", b)))
                            .collect();
                        lines.push(Line::from(vec![
                            Span::raw("  "),
                            Span::styled(name, Style::default().fg(Color::Red)),
                            Span::styled(pt, Style::default().fg(Color::Gray)),
                            Span::styled(
                                format!(" <- {}", blocker_names.join(", ")),
                                Style::default().fg(Color::Blue),
                            ),
                        ]));
                    }
                }
            } else {
                lines.push(Line::from(Span::styled(
                    "No attackers",
                    Style::default().fg(Color::DarkGray),
                )));
            }

            lines.push(Line::from(""));

            // Show defending creatures that are blocking
            if !combat.blockers.is_empty() {
                lines.push(Line::from(Span::styled(
                    format!("Blockers ({}):", combat.blockers.len()),
                    Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD),
                )));

                for (&blocker_id, blocked_attackers) in &combat.blockers {
                    let name = view
                        .card_name(blocker_id)
                        .unwrap_or_else(|| format!("{:?}", blocker_id));

                    // Get what it's blocking
                    let blocking_names: Vec<String> = blocked_attackers
                        .iter()
                        .map(|&a| view.card_name(a).unwrap_or_else(|| format!("{:?}", a)))
                        .collect();

                    let pt = if let Some(card) = view.get_card(blocker_id) {
                        let power = view
                            .get_effective_power(blocker_id)
                            .unwrap_or(card.current_power() as i32);
                        let toughness = view
                            .get_effective_toughness(blocker_id)
                            .unwrap_or(card.current_toughness() as i32);
                        format!(" {}/{}", power, toughness)
                    } else {
                        String::new()
                    };

                    if !blocking_names.is_empty() {
                        lines.push(Line::from(vec![
                            Span::raw("  "),
                            Span::styled(name, Style::default().fg(Color::Blue)),
                            Span::styled(pt, Style::default().fg(Color::Gray)),
                            Span::styled(
                                format!(" -> {}", blocking_names.join(", ")),
                                Style::default().fg(Color::Red),
                            ),
                        ]));
                    }
                }
            }
        } else {
            lines.push(Line::from(Span::styled(
                "No combat in progress",
                Style::default().fg(Color::DarkGray),
            )));
        }

        let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
        f.render_widget(paragraph, area);
    }

    /// Draw the game log view panel
    fn draw_log_view(&self, f: &mut Frame, area: Rect, view: &GameStateView) {
        let logs = view.logger().logs();

        // Show last N log entries that fit in the area
        let max_lines = area.height as usize;
        let start_idx = logs.len().saturating_sub(max_lines);

        let log_lines: Vec<ListItem> = logs[start_idx..]
            .iter()
            .map(|entry| {
                // Color based on verbosity level
                let style = match entry.level {
                    VerbosityLevel::Silent => Style::default().fg(Color::DarkGray),
                    VerbosityLevel::Minimal => Style::default().fg(Color::Gray),
                    VerbosityLevel::Normal => Style::default().fg(Color::White),
                    VerbosityLevel::Verbose => Style::default().fg(Color::Yellow),
                };
                ListItem::new(Line::from(Span::styled(&entry.message, style)))
            })
            .collect();

        let log_list = List::new(log_lines);
        f.render_widget(log_list, area);
    }

    /// Draw the prompt/actions panel
    fn draw_prompt(
        &self,
        f: &mut Frame,
        area: Rect,
        view: &GameStateView,
        current_prompt: Option<&str>,
        choices: &[(String, bool)],
    ) {
        let is_focused = self.state.focused_pane == FocusedPane::Actions;

        let block = Block::default()
            .borders(Borders::ALL)
            .title(if is_focused {
                " Actions (A) [FOCUSED] "
            } else {
                " Actions (A) "
            })
            .border_style(if is_focused {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default()
            });

        let inner_area = block.inner(area);
        f.render_widget(block, area);

        let mut lines = Vec::new();

        // Show turn info
        let turn_info = format!(
            "Turn {} - {} ({}'s turn)",
            view.turn_number(),
            Self::step_abbrev(view.current_step()),
            view.get_player_name_by_id(view.active_player())
        );
        lines.push(Line::from(Span::styled(turn_info, Style::default().fg(Color::Cyan))));

        // Show player info line
        let player_name = view.player_name();
        let player_life = view.player_life(view.player_id());
        let player_info = format!("{}: {} life", player_name, player_life);
        lines.push(Line::from(Span::styled(player_info, Style::default().fg(Color::Green))));

        lines.push(Line::from(""));

        // Show prompt if present
        if let Some(prompt) = current_prompt {
            lines.push(Line::from(Span::styled(
                prompt,
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(""));
        }

        // Show rewind message if present
        if let Some(ref msg) = self.state.rewind_message {
            lines.push(Line::from(Span::styled(
                msg.as_str(),
                Style::default().fg(Color::Magenta),
            )));
            lines.push(Line::from(""));
        }

        // Show choices
        for (text, is_highlighted) in choices {
            let style = if *is_highlighted {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            lines.push(Line::from(Span::styled(text.as_str(), style)));
        }

        // Show navigation hints at the bottom
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Up/Down: select | Enter: confirm | Esc: pass | Z: undo | R: random",
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(Span::styled(
            "H/I/Y/O/A/S: focus panes | Tab: cycle tabs",
            Style::default().fg(Color::DarkGray),
        )));

        let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
        f.render_widget(paragraph, inner_area);
    }

    // NOTE: The remaining rendering methods (draw_battlefield, draw_card_details,
    // draw_hand, draw_stack, render_entity, render_visual_stack, render_card_group)
    // will be migrated from fancy_tui_controller.rs in subsequent updates.
    // This is a partial extraction to establish the architecture.

    /// Draw a player's battlefield
    fn draw_battlefield(&mut self, f: &mut Frame, area: Rect, view: &GameStateView, owner_id: PlayerId, title: &str) {
        let is_yours = owner_id == view.player_id();
        let is_focused = if is_yours {
            self.state.focused_pane == FocusedPane::YourBattlefield
        } else {
            self.state.focused_pane == FocusedPane::OpponentBattlefield
        };

        let pane_key = if is_yours { "Y" } else { "O" };
        let title_text = if is_focused {
            format!(" {} ({}) [FOCUSED] ", title, pane_key)
        } else {
            format!(" {} ({}) ", title, pane_key)
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .title(title_text)
            .border_style(if is_focused {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default()
            });

        let inner_area = block.inner(area);
        f.render_widget(block, area);

        // Get cards and group into entities
        let cards = Self::get_battlefield_cards_in_order(view, owner_id);
        let entities = self.group_cards_into_entities(&cards, view);

        if entities.is_empty() {
            let empty_msg = Paragraph::new("(empty)")
                .style(Style::default().fg(Color::DarkGray))
                .alignment(Alignment::Center);
            f.render_widget(empty_msg, inner_area);
            return;
        }

        // Simple rendering: list entities vertically
        let items: Vec<ListItem> = entities
            .iter()
            .map(|entity| {
                let name = entity.display_name(view);
                let is_valid_choice = entity.card_ids().iter().any(|id| self.state.valid_choices.contains(id));
                let is_selected = if is_yours {
                    self.state
                        .selected_card_in_your_bf
                        .is_some_and(|sel| entity.card_ids().contains(&sel))
                } else {
                    self.state
                        .selected_card_in_opp_bf
                        .is_some_and(|sel| entity.card_ids().contains(&sel))
                };

                let style = if is_selected {
                    Style::default().fg(Color::Black).bg(Color::Cyan)
                } else if is_valid_choice {
                    Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
                } else if entity.is_tapped(view) {
                    Style::default().fg(Color::DarkGray)
                } else {
                    Style::default().fg(Color::White)
                };

                let tapped_marker = if entity.is_tapped(view) { " [T]" } else { "" };
                ListItem::new(Line::from(Span::styled(format!("{}{}", name, tapped_marker), style)))
            })
            .collect();

        // Track entity positions for mouse hit testing
        let list_area = inner_area;
        for (i, entity) in entities.iter().enumerate() {
            if (i as u16) < list_area.height {
                let entity_area = Rect {
                    x: list_area.x,
                    y: list_area.y + i as u16,
                    width: list_area.width,
                    height: 1,
                };
                self.state.entity_positions.push(EntityPosition {
                    entity: entity.clone(),
                    area: entity_area,
                });
            }
        }

        let list = List::new(items);
        f.render_widget(list, inner_area);
    }

    /// Draw card details panel
    fn draw_card_details(&self, f: &mut Frame, area: Rect, view: &GameStateView) {
        let block = Block::default().borders(Borders::ALL).title(" Card Details ");

        let inner_area = block.inner(area);
        f.render_widget(block, area);

        if let Some(card_id) = self.state.selected_card_id {
            if let Some(card) = view.get_card(card_id) {
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

                // Type line (constructed from types and subtypes)
                let type_names: Vec<String> = card.types.iter().map(|t| format!("{:?}", t)).collect();
                let subtype_names: Vec<String> = card.subtypes.iter().map(|s| format!("{:?}", s)).collect();
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
                if card.is_creature() {
                    let power = view.get_effective_power(card_id).unwrap_or(card.current_power() as i32);
                    let toughness = view
                        .get_effective_toughness(card_id)
                        .unwrap_or(card.current_toughness() as i32);
                    lines.push(Line::from(vec![
                        Span::raw("P/T: "),
                        Span::styled(format!("{}/{}", power, toughness), Style::default().fg(Color::Green)),
                    ]));
                }

                // Status
                let mut status_parts = Vec::new();
                if card.tapped {
                    status_parts.push("Tapped");
                }
                // Show summoning sickness for creatures that entered this turn
                if card.is_creature() && card.turn_entered_battlefield == Some(view.turn_number()) {
                    status_parts.push("Summoning Sick");
                }
                if !status_parts.is_empty() {
                    lines.push(Line::from(vec![
                        Span::raw("Status: "),
                        Span::styled(status_parts.join(", "), Style::default().fg(Color::Gray)),
                    ]));
                }

                // Oracle text (if any)
                if !card.text.is_empty() {
                    lines.push(Line::from(""));
                    lines.push(Line::from(Span::styled(
                        card.text.clone(),
                        Style::default().fg(Color::White),
                    )));
                }

                let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
                f.render_widget(paragraph, inner_area);
            }
        } else {
            let hint = Paragraph::new("Click a card or use arrow keys to view details")
                .style(Style::default().fg(Color::DarkGray))
                .wrap(Wrap { trim: false });
            f.render_widget(hint, inner_area);
        }
    }

    /// Draw the hand panel
    fn draw_hand(&self, f: &mut Frame, area: Rect, view: &GameStateView) {
        let is_focused = self.state.focused_pane == FocusedPane::Hand;

        let hand = view.hand();
        let title = if is_focused {
            format!(" Hand ({}) (H) [FOCUSED] ", hand.len())
        } else {
            format!(" Hand ({}) (H) ", hand.len())
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .title(title)
            .border_style(if is_focused {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default()
            });

        let inner_area = block.inner(area);
        f.render_widget(block, area);

        if hand.is_empty() {
            let empty_msg = Paragraph::new("(empty)")
                .style(Style::default().fg(Color::DarkGray))
                .alignment(Alignment::Center);
            f.render_widget(empty_msg, inner_area);
            return;
        }

        let items: Vec<ListItem> = hand
            .iter()
            .enumerate()
            .map(|(i, &card_id)| {
                let name = view.card_name(card_id).unwrap_or_else(|| format!("{:?}", card_id));
                let is_valid_choice = self.state.valid_choices.contains(&card_id);
                let is_selected = self.state.selected_card_in_hand == Some(i);

                let style = if is_selected {
                    Style::default().fg(Color::Black).bg(Color::Cyan)
                } else if is_valid_choice {
                    Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                };

                // Show mana cost
                let cost = view
                    .get_card(card_id)
                    .map(|c| format!("{}", c.mana_cost))
                    .unwrap_or_default();

                let text = if cost.is_empty() {
                    name
                } else {
                    format!("{} {}", name, cost)
                };

                ListItem::new(Line::from(Span::styled(text, style)))
            })
            .collect();

        let list = List::new(items);
        f.render_widget(list, inner_area);
    }

    /// Draw the stack panel
    fn draw_stack(&self, f: &mut Frame, area: Rect, view: &GameStateView) {
        let is_focused = self.state.focused_pane == FocusedPane::Stack;

        let stack = view.stack();
        let title = if is_focused {
            format!(" Stack ({}) (S) [FOCUSED] ", stack.len())
        } else {
            format!(" Stack ({}) (S) ", stack.len())
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .title(title)
            .border_style(if is_focused {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default()
            });

        let inner_area = block.inner(area);
        f.render_widget(block, area);

        if stack.is_empty() {
            let empty_msg = Paragraph::new("(empty)")
                .style(Style::default().fg(Color::DarkGray))
                .alignment(Alignment::Center);
            f.render_widget(empty_msg, inner_area);
            return;
        }

        // Stack is displayed bottom-up (most recent on top)
        let items: Vec<ListItem> = stack
            .iter()
            .rev()
            .enumerate()
            .map(|(i, &card_id)| {
                let name = view.card_name(card_id).unwrap_or_else(|| format!("{:?}", card_id));
                let controller = view.get_card(card_id).map(|c| c.controller);
                let owner_marker = if controller == Some(view.player_id()) {
                    "(yours)"
                } else {
                    "(theirs)"
                };

                let style = if i == 0 {
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                };

                ListItem::new(Line::from(vec![
                    Span::styled(name, style),
                    Span::styled(format!(" {}", owner_marker), Style::default().fg(Color::DarkGray)),
                ]))
            })
            .collect();

        let list = List::new(items);
        f.render_widget(list, inner_area);
    }
}
