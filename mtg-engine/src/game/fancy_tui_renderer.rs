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
    /// A card in hand - used for mouse click detection
    HandCard {
        card_id: CardId,
        index: usize, // Index in hand for selection
    },
    /// A card in graveyard - used for mouse click detection
    GraveyardCard {
        card_id: CardId,
        index: usize,    // Index in graveyard for selection
        owner: PlayerId, // Whose graveyard this is in
    },
}

/// Item in battlefield word-wrap layout - either a section label or a card entity
#[derive(Debug, Clone)]
enum BattlefieldItem {
    /// Section label (e.g., "Lands:", "Creatures:")
    Label {
        text: String,
        color: Color,
        force_newline_before: bool,
        #[allow(dead_code)] // Reserved for future use (e.g., showing counts in header)
        entity_count: usize,
    },
    /// Card entity (single card or stack)
    Card { entity: Entity },
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
            Entity::HandCard { card_id, .. } => std::slice::from_ref(card_id),
            Entity::GraveyardCard { card_id, .. } => std::slice::from_ref(card_id),
        }
    }

    fn representative_card(&self) -> CardId {
        match self {
            Entity::SingleCard { card_id } => *card_id,
            Entity::SimpleStack { card_ids, .. } => card_ids[0],
            Entity::VisualStack { card_ids, .. } => card_ids[0],
            Entity::HandCard { card_id, .. } => *card_id,
            Entity::GraveyardCard { card_id, .. } => *card_id,
        }
    }

    fn count(&self) -> usize {
        match self {
            Entity::SingleCard { .. } => 1,
            Entity::SimpleStack { card_ids, .. } => card_ids.len(),
            Entity::VisualStack { card_ids, .. } => card_ids.len(),
            Entity::HandCard { .. } => 1,
            Entity::GraveyardCard { .. } => 1,
        }
    }

    fn display_name(&self, view: &GameStateView) -> String {
        // Note: Stack counts (2X:, 3X:) are now rendered in the header row above the card,
        // so we don't include them in the display name anymore.
        match self {
            Entity::SingleCard { card_id } => view.card_name(*card_id).unwrap_or_else(|| format!("{:?}", card_id)),
            Entity::SimpleStack { card_name, .. } => card_name.clone(),
            Entity::VisualStack { card_name, .. } => card_name.clone(),
            Entity::HandCard { card_id, .. } => view.card_name(*card_id).unwrap_or_else(|| format!("{:?}", card_id)),
            Entity::GraveyardCard { card_id, .. } => {
                view.card_name(*card_id).unwrap_or_else(|| format!("{:?}", card_id))
            }
        }
    }

    fn is_tapped(&self, view: &GameStateView) -> bool {
        match self {
            Entity::SingleCard { card_id } => view.is_tapped(*card_id),
            Entity::SimpleStack { is_tapped, .. } => *is_tapped,
            Entity::VisualStack { tapped_count, .. } => *tapped_count > 0,
            Entity::HandCard { .. } => false,      // Hand cards are never tapped
            Entity::GraveyardCard { .. } => false, // Graveyard cards are never tapped
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

    // Card size constants
    // Default size maintains MTG card aspect ratio: width/height = 10/7 ≈ 1.43
    // This accounts for terminal character aspect (~2:1) to create visually proportional cards
    pub const DEFAULT_CARD_WIDTH: u16 = 10;
    pub const DEFAULT_CARD_HEIGHT: u16 = 7;
    pub const MIN_CARD_WIDTH: u16 = 5;
    pub const MIN_CARD_HEIGHT: u16 = 4;
    pub const MAX_CARD_HEIGHT: u16 = 15; // Prevent cards from getting too large
    pub const CARD_SPACING: u16 = 1;
    const DIAGONAL_OFFSET: u16 = 1; // chars per card in stack
    /// Maximum number of cards in a visual stack (prevents huge stacks of 5+ lands)
    const MAX_VISUAL_STACK_SIZE: usize = 4;

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

    /// Calculate required height for player info bar based on available width.
    /// Returns 3 if content fits on one line, 4 if it needs two lines.
    /// Height includes 2 for borders (top + bottom) plus content lines.
    fn calculate_info_bar_height(available_width: u16) -> u16 {
        // Account for borders and padding in the Paragraph widget
        let inner_width = available_width.saturating_sub(4);

        // Estimate max lengths for both parts of the status line:
        // Left: "Opp: 20 life | Hand: 7 | GY: 99 | Lib: 99" (~42 chars)
        // Right: "Turn: 99 (99) | UP UK DR M1 BC DA DB CD EC M2 ET" (~48 chars)
        const STATS_MAX_LEN: u16 = 42;
        const PHASE_MAX_LEN: u16 = 48;
        const MIN_SPACING: u16 = 3;

        let needs_two_lines = STATS_MAX_LEN + PHASE_MAX_LEN + MIN_SPACING > inner_width;

        if needs_two_lines {
            4 // 2 borders + 2 content lines
        } else {
            3 // 2 borders + 1 content line
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
        let construct_entities = |card_name: String, card_ids: SmallVec<[CardId; 8]>| -> Vec<Entity> {
            if self.visual_stacks {
                // Visual stacking: separate tapped/untapped, then chunk by MAX_VISUAL_STACK_SIZE
                let (tapped, untapped): (SmallVec<[CardId; 8]>, SmallVec<[CardId; 8]>) =
                    card_ids.into_iter().partition(|&id| view.is_tapped(id));

                let mut result = Vec::new();

                // Helper to create visual stacks from a group, chunking if needed
                let mut add_visual_stacks = |cards: SmallVec<[CardId; 8]>, is_tapped: bool, name: &str| {
                    for chunk in cards.chunks(Self::MAX_VISUAL_STACK_SIZE) {
                        if chunk.len() > 1 {
                            result.push(Entity::VisualStack {
                                card_ids: chunk.iter().copied().collect(),
                                card_name: name.to_string(),
                                tapped_count: if is_tapped { chunk.len() } else { 0 },
                            });
                        } else if chunk.len() == 1 {
                            result.push(Entity::SingleCard { card_id: chunk[0] });
                        }
                    }
                };

                // Create visual stacks for untapped cards (chunked)
                add_visual_stacks(untapped, false, &card_name);

                // Create visual stacks for tapped cards (chunked)
                add_visual_stacks(tapped, true, &card_name);

                result
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
        // Note: HandCard is not used in battlefield grouping, but we handle it for completeness
        entities.sort_by_key(|e| match e {
            Entity::SingleCard { card_id } => (0, *card_id),
            Entity::VisualStack { card_ids, .. } => (1, card_ids[0]),
            Entity::SimpleStack { card_ids, .. } => (1, card_ids[0]),
            Entity::HandCard { card_id, .. } => (2, *card_id), // Shouldn't appear in battlefield
            Entity::GraveyardCard { card_id, .. } => (3, *card_id), // Shouldn't appear in battlefield
        });

        entities
    }

    /// Compute card width from height while maintaining the default aspect ratio
    /// This is the centralized function for all aspect ratio calculations
    pub fn compute_width_from_height(height: u16) -> u16 {
        ((height as f32 * Self::DEFAULT_CARD_WIDTH as f32) / Self::DEFAULT_CARD_HEIGHT as f32).round() as u16
    }

    /// Get card dimensions based on tapped state and base size
    /// Tapped cards swap width and height to simulate 90-degree rotation
    pub fn get_card_dimensions_with_size(
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
    pub fn get_dimensions_for_tapped_state(is_tapped: bool, base_width: u16, base_height: u16) -> (u16, u16) {
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
    pub fn get_entity_dimensions(
        entity: &Entity,
        view: &GameStateView,
        base_width: u16,
        base_height: u16,
    ) -> (u16, u16) {
        match entity {
            Entity::SingleCard { .. } | Entity::SimpleStack { .. } => {
                let is_tapped = entity.is_tapped(view);
                Self::get_dimensions_for_tapped_state(is_tapped, base_width, base_height)
            }
            Entity::VisualStack {
                card_ids, tapped_count, ..
            } => {
                // Visual stacks need extra space for diagonal offsets
                let stack_depth = card_ids.len() as u16;
                let offset_total = (stack_depth.saturating_sub(1)) * Self::DIAGONAL_OFFSET;

                // Determine if we need tapped dimensions for the visible top cards
                let any_tapped = *tapped_count > 0;
                let (base_w, base_h) = Self::get_dimensions_for_tapped_state(any_tapped, base_width, base_height);

                (base_w + offset_total, base_h + offset_total)
            }
            Entity::HandCard { .. } | Entity::GraveyardCard { .. } => {
                // Hand/graveyard cards use base dimensions (never tapped, no stacking)
                (base_width, base_height)
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
    pub fn calculate_optimal_card_size(
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
            // Default fits, try to increase size
            let mut best_height = Self::DEFAULT_CARD_HEIGHT;

            for h in (Self::DEFAULT_CARD_HEIGHT + 1)..=Self::MAX_CARD_HEIGHT {
                let w = Self::compute_width_from_height(h);
                if Self::test_card_size_fits(area, card_groups, view, w, h) {
                    best_height = h;
                } else {
                    break;
                }
            }

            let best_width = Self::compute_width_from_height(best_height);
            return (best_width, best_height);
        }

        // Default doesn't fit, try to shrink
        for h in (Self::MIN_CARD_HEIGHT..Self::DEFAULT_CARD_HEIGHT).rev() {
            let w = Self::compute_width_from_height(h).max(Self::MIN_CARD_WIDTH);
            if Self::test_card_size_fits(area, card_groups, view, w, h) {
                return (w, h);
            }
        }

        // Return minimum size as fallback
        (Self::MIN_CARD_WIDTH, Self::MIN_CARD_HEIGHT)
    }

    /// Map card color to ratatui color
    fn card_color_to_ratatui(color: &crate::core::Color) -> Color {
        match color {
            crate::core::Color::Red => Color::Red,
            crate::core::Color::Green => Color::Green,
            crate::core::Color::Blue => Color::Blue,
            crate::core::Color::White => Color::White,
            crate::core::Color::Black => Color::DarkGray,
            crate::core::Color::Colorless => Color::Gray,
        }
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

        // Middle column: Player info bars + battlefields
        // Layout: Opponent info bar, Opponent battlefield, Your battlefield, Your info bar
        // Calculate info bar height based on whether status fits on one line
        let info_bar_height = Self::calculate_info_bar_height(main_chunks[1].width);
        let middle_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(info_bar_height), // Opponent info header
                Constraint::Percentage(45),          // Opponent battlefield
                Constraint::Percentage(45),          // Your battlefield
                Constraint::Length(info_bar_height), // Your info footer
            ])
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

        // Draw battlefields with player info bars
        if let Some(opp_id) = opponent_id {
            self.draw_player_info(f, middle_chunks[0], view, opp_id);
            self.draw_battlefield(f, middle_chunks[1], view, opp_id, "Opponent");
        }
        self.draw_battlefield(f, middle_chunks[2], view, view.player_id(), "You");
        self.draw_player_info(f, middle_chunks[3], view, view.player_id());

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

    /// Render battlefield with inline section labels using word-wrap model.
    ///
    /// Treats the battlefield as a continuous "sentence" that word-wraps:
    /// - Section labels (e.g., "Lands:") are inline items that always render
    /// - Cards flow after their section label
    /// - Content wraps to next row when it doesn't fit
    /// - A header row at the top shows all section labels with counts
    fn render_battlefield_inline(
        &mut self,
        f: &mut Frame,
        area: Rect,
        view: &GameStateView,
        sections: &[(Vec<CardId>, &str, Color, bool)], // (cards, label, color, try_newline_before)
    ) {
        if sections.is_empty() || area.height == 0 || area.width == 0 {
            return;
        }

        // Build flat list of items to render
        let mut items: Vec<BattlefieldItem> = Vec::new();
        for (cards, label, color, force_newline) in sections {
            if cards.is_empty() {
                continue;
            }

            // Group cards into visual entities (stacks)
            let entities = self.group_cards_into_entities(cards, view);

            // Add section label item
            items.push(BattlefieldItem::Label {
                text: (*label).to_string(),
                color: *color,
                force_newline_before: *force_newline,
                entity_count: entities.len(),
            });

            // Add card items
            for entity in entities {
                items.push(BattlefieldItem::Card { entity });
            }
        }

        if items.is_empty() {
            return;
        }

        // Calculate optimal card size for this layout
        let (card_width, card_height) = self.calculate_wordwrap_card_size(area, view, &items);

        // Render using word-wrap model
        self.render_wordwrap_battlefield(f, area, view, &items, card_width, card_height);
    }

    /// Calculate optimal card size for word-wrap battlefield layout.
    /// Each row of cards has a 1-line header above it for labels.
    fn calculate_wordwrap_card_size(&self, area: Rect, view: &GameStateView, items: &[BattlefieldItem]) -> (u16, u16) {
        // Try default size first
        if self.test_wordwrap_layout_fits(area, view, items, Self::DEFAULT_CARD_WIDTH, Self::DEFAULT_CARD_HEIGHT) {
            // Default fits, try to increase
            let mut best_height = Self::DEFAULT_CARD_HEIGHT;
            for h in (Self::DEFAULT_CARD_HEIGHT + 1)..=Self::MAX_CARD_HEIGHT {
                let w = Self::compute_width_from_height(h);
                if self.test_wordwrap_layout_fits(area, view, items, w, h) {
                    best_height = h;
                } else {
                    break;
                }
            }
            return (Self::compute_width_from_height(best_height), best_height);
        }

        // Default doesn't fit, try to shrink
        for h in (Self::MIN_CARD_HEIGHT..Self::DEFAULT_CARD_HEIGHT).rev() {
            let w = Self::compute_width_from_height(h).max(Self::MIN_CARD_WIDTH);
            if self.test_wordwrap_layout_fits(area, view, items, w, h) {
                return (w, h);
            }
        }

        // Return minimum as fallback
        (Self::MIN_CARD_WIDTH, Self::MIN_CARD_HEIGHT)
    }

    /// Test if word-wrap layout fits in area.
    /// Each row of cards has a 1-line header above it (row_unit = 1 + card_height).
    fn test_wordwrap_layout_fits(
        &self,
        area: Rect,
        view: &GameStateView,
        items: &[BattlefieldItem],
        card_width: u16,
        card_height: u16,
    ) -> bool {
        if area.height == 0 || area.width == 0 {
            return items.is_empty();
        }

        // Each row takes: 1 (header) + card_height + CARD_SPACING (between rows)
        let row_unit = 1 + card_height;

        let mut y_offset = 0u16;
        let mut current_x = 0u16;

        for item in items {
            match item {
                BattlefieldItem::Label { force_newline_before, .. } => {
                    // Handle forced newlines between sections
                    if *force_newline_before && current_x > 0 {
                        y_offset += row_unit + Self::CARD_SPACING;
                        current_x = 0;
                    }
                }
                BattlefieldItem::Card { entity } => {
                    let (card_w, _card_h) = Self::get_entity_dimensions(entity, view, card_width, card_height);

                    // Check if card fits on current row
                    if current_x > 0 && current_x + card_w > area.width {
                        // Wrap to next row
                        y_offset += row_unit + Self::CARD_SPACING;
                        current_x = 0;
                    }

                    // Check if this row fits (header + card)
                    if y_offset + row_unit > area.height {
                        return false;
                    }

                    current_x += card_w + Self::CARD_SPACING;
                }
            }
        }

        true
    }

    /// Render battlefield using word-wrap model with per-row headers.
    /// Each row of cards has a 1-line header above it where section labels and stack counts appear.
    fn render_wordwrap_battlefield(
        &mut self,
        f: &mut Frame,
        area: Rect,
        view: &GameStateView,
        items: &[BattlefieldItem],
        card_width: u16,
        card_height: u16,
    ) {
        if area.height == 0 || area.width == 0 {
            return;
        }

        // Each row takes: 1 (header) + card_height
        let row_unit = 1 + card_height;

        // First pass: compute card positions, label positions, and stack count positions
        // Labels appear directly above the first card of their section
        // Stack counts (2X:, 3X:, etc.) appear above each stacked entity
        let mut card_positions: Vec<(u16, u16, u16, u16)> = Vec::new(); // (x, y, w, h) for each card
        let mut label_positions: Vec<(u16, u16, String, Color)> = Vec::new(); // (x, y, text, color)
        let mut stack_count_positions: Vec<(u16, u16, usize)> = Vec::new(); // (x, y, count) for stacks

        let mut y_offset = 0u16;
        let mut current_x = 0u16;
        let mut pending_label: Option<(String, Color)> = None;

        for item in items {
            match item {
                BattlefieldItem::Label {
                    text,
                    color,
                    force_newline_before,
                    ..
                } => {
                    // Handle forced newlines between sections
                    if *force_newline_before && current_x > 0 {
                        y_offset += row_unit + Self::CARD_SPACING;
                        current_x = 0;
                    }
                    // Store label to render above next card
                    pending_label = Some((format!("{}:", text), *color));
                }
                BattlefieldItem::Card { entity } => {
                    let (card_w, card_h) = Self::get_entity_dimensions(entity, view, card_width, card_height);

                    // Check if card fits on current row
                    if current_x > 0 && current_x + card_w > area.width {
                        // Wrap to next row
                        y_offset += row_unit + Self::CARD_SPACING;
                        current_x = 0;
                    }

                    // If there's a pending label, place it above this card
                    if let Some((label_text, label_color)) = pending_label.take() {
                        // Label goes in header row (y_offset), card goes below (y_offset + 1)
                        label_positions.push((current_x, y_offset, label_text, label_color));
                    }

                    // Check if this entity is a stack with count > 1
                    let stack_count = entity.count();
                    if stack_count > 1 {
                        stack_count_positions.push((current_x, y_offset, stack_count));
                    }

                    // Card position: below the header line
                    let card_y = y_offset + 1;
                    if card_y + card_h <= area.height {
                        card_positions.push((current_x, card_y, card_w, card_h));
                    }

                    current_x += card_w + Self::CARD_SPACING;
                }
            }
        }

        // Second pass: render labels in header rows
        for (x, y, text, color) in &label_positions {
            if *y < area.height {
                let label_area = Rect {
                    x: area.x + x,
                    y: area.y + y,
                    width: text.len() as u16,
                    height: 1,
                };
                let styled_label = Span::styled(text.clone(), Style::default().fg(*color).add_modifier(Modifier::BOLD));
                f.render_widget(Paragraph::new(Line::from(styled_label)), label_area);
            }
        }

        // Third pass: render stack counts in header rows (rendered AFTER labels so they overwrite if needed)
        for (x, y, count) in &stack_count_positions {
            if *y < area.height {
                let count_text = format!("{}X:", count);
                let count_area = Rect {
                    x: area.x + x,
                    y: area.y + y,
                    width: count_text.len() as u16,
                    height: 1,
                };
                let styled_count =
                    Span::styled(count_text, Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD));
                f.render_widget(Paragraph::new(Line::from(styled_count)), count_area);
            }
        }

        // Fourth pass: render cards
        let mut card_idx = 0;
        for item in items {
            if let BattlefieldItem::Card { entity } = item {
                if card_idx < card_positions.len() {
                    let (x, y, w, h) = card_positions[card_idx];
                    let entity_area = Rect {
                        x: area.x + x,
                        y: area.y + y,
                        width: w,
                        height: h,
                    };
                    self.render_entity(f, entity_area, view, entity);
                    card_idx += 1;
                }
            }
        }
    }

    /// Render a group of cards (lands, creatures, others) with dynamic packing
    #[allow(clippy::too_many_arguments)]
    pub fn render_card_group(
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
        use ratatui::text::Text;

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

    /// Draw a single battlefield with ASCII card boxes
    fn draw_battlefield(&mut self, f: &mut Frame, area: Rect, view: &GameStateView, owner_id: PlayerId, _title: &str) {
        use ratatui::text::Text;

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

        // Build ordered sections: lands at bottom for player, top for opponent
        // Section format: (cards, label, color, force_newline_before)
        let sections: Vec<(Vec<CardId>, &str, Color, bool)> = if is_player_bf {
            // Player battlefield: non-lands first, then lands
            let mut secs = Vec::new();
            if !creatures.is_empty() {
                secs.push((creatures, "Creatures", Color::Red, false));
            }
            if !artifacts.is_empty() {
                secs.push((artifacts, "Artifacts", Color::Cyan, false));
            }
            if !enchantments.is_empty() {
                secs.push((enchantments, "Enchants", Color::Magenta, false));
            }
            if !lands.is_empty() {
                // Try to force a newline before lands (will be evaluated during rendering)
                secs.push((lands, "Lands", Color::Green, true));
            }
            secs
        } else {
            // Opponent battlefield: lands first, then non-lands
            let mut secs = Vec::new();
            if !lands.is_empty() {
                secs.push((lands, "Lands", Color::Green, false));
            }
            if !creatures.is_empty() {
                // Try to force a newline after lands (before creatures)
                secs.push((creatures, "Creatures", Color::Red, !secs.is_empty()));
            }
            if !artifacts.is_empty() {
                secs.push((artifacts, "Artifacts", Color::Cyan, false));
            }
            if !enchantments.is_empty() {
                secs.push((enchantments, "Enchants", Color::Magenta, false));
            }
            secs
        };

        // Render battlefield with inline section labels
        self.render_battlefield_inline(f, inner_area, view, &sections);

        // Render graveyard overlay in bottom-right corner
        self.render_graveyard_overlay(f, inner_area, view, owner_id);
    }

    /// Render graveyard as a simple text list in the bottom-right corner of the battlefield
    /// Each card name is clickable to show card details
    fn render_graveyard_overlay(&mut self, f: &mut Frame, area: Rect, view: &GameStateView, owner_id: PlayerId) {
        let graveyard = view.player_graveyard(owner_id);
        if graveyard.is_empty() {
            return;
        }

        // Build list of (card_id, name) pairs
        let card_entries: Vec<(CardId, String)> = graveyard
            .iter()
            .map(|&card_id| {
                (
                    card_id,
                    view.card_name(card_id).unwrap_or_else(|| "Unknown".to_string()),
                )
            })
            .collect();

        // Calculate required width (longest name or header)
        let header = "Graveyard:";
        let max_name_len = card_entries.iter().map(|(_, n)| n.len()).max().unwrap_or(0);
        let content_width = max_name_len.max(header.len()) as u16;

        // Calculate required height: header + cards
        let box_height = (1 + card_entries.len()) as u16;

        // Position in bottom-right corner
        if area.width < content_width || area.height < box_height {
            return; // Not enough space
        }

        let x_start = area.x + area.width - content_width;
        let y_start = area.y + area.height - box_height;

        let style = Style::default().fg(Color::DarkGray);

        // Header line: "Graveyard:"
        let header_area = Rect {
            x: x_start,
            y: y_start,
            width: content_width,
            height: 1,
        };
        f.render_widget(Paragraph::new(header).style(style), header_area);

        // Card name lines (clickable)
        for (i, (card_id, name)) in card_entries.iter().enumerate() {
            let card_area = Rect {
                x: x_start,
                y: y_start + 1 + i as u16,
                width: content_width,
                height: 1,
            };
            f.render_widget(Paragraph::new(name.as_str()).style(style), card_area);

            // Record entity position for click detection
            self.state.entity_positions.push(EntityPosition {
                entity: Entity::GraveyardCard {
                    card_id: *card_id,
                    index: i,
                    owner: owner_id,
                },
                area: card_area,
            });
        }
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
    fn draw_hand(&mut self, f: &mut Frame, area: Rect, view: &GameStateView) {
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

        // Track entity positions for each hand card (for mouse click detection)
        // Each list item is 1 row tall, positioned starting from inner_area.y
        for (i, &card_id) in hand.iter().enumerate() {
            let card_area = Rect {
                x: inner_area.x,
                y: inner_area.y + i as u16,
                width: inner_area.width,
                height: 1,
            };
            // Only track if within visible area
            if card_area.y < inner_area.y + inner_area.height {
                self.state.entity_positions.push(EntityPosition {
                    entity: Entity::HandCard { card_id, index: i },
                    area: card_area,
                });
            }
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

    /// Draw player info bar (life, zones, etc.)
    pub fn draw_player_info(&self, f: &mut Frame, area: Rect, view: &GameStateView, player_id: PlayerId) {
        use ratatui::text::Text;

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

    /// Render a visual stack with diagonal offsets
    fn render_visual_stack(&mut self, f: &mut Frame, area: Rect, view: &GameStateView, entity: &Entity) {
        use ratatui::text::Text;

        let Entity::VisualStack {
            card_ids, tapped_count, ..
        } = entity
        else {
            return;
        };

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
                1 => Self::card_color_to_ratatui(&card.colors[0]),
                _ => Color::Yellow,
            }
        } else {
            Color::Gray
        };

        // Render stacked cards from back to front (bottom-left to top-right)
        for i in 0..stack_depth {
            let offset = i as u16 * Self::DIAGONAL_OFFSET;

            // Card area with diagonal offset
            let card_area = Rect {
                x: area.x + offset,
                y: area.y + offset,
                width: area
                    .width
                    .saturating_sub(offset + Self::DIAGONAL_OFFSET * (stack_depth - i - 1) as u16),
                height: area
                    .height
                    .saturating_sub(offset + Self::DIAGONAL_OFFSET * (stack_depth - i - 1) as u16),
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
    pub fn render_entity(&mut self, f: &mut Frame, area: Rect, view: &GameStateView, entity: &Entity) {
        use ratatui::text::Text;

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
                1 => Self::card_color_to_ratatui(&card.colors[0]),
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
                .map(|c| format!("{}/{}", c.current_power(), c.current_toughness()))
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
}
