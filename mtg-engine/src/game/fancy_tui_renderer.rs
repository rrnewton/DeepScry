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
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
    Frame,
};
use smallvec::SmallVec;
use std::collections::HashMap;

/// Get style for log message based on content patterns
///
/// This provides content-aware coloring for the log pane, making it easier
/// to scan for important events like combat, damage, and turn transitions.
fn style_for_log_content(message: &str, level: VerbosityLevel) -> Style {
    // Turn headers: yellow, bold, underlined
    if message.contains(">>> Turn") || message.contains("<<<< ") {
        return Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
    }

    // Step headers: cyan
    if message.starts_with("--- ") && message.ends_with(" ---") {
        return Style::default().fg(Color::Cyan);
    }

    // Combat events: magenta
    if message.contains("attacks") || message.contains("blocks") {
        return Style::default().fg(Color::Magenta);
    }

    // Damage/life loss: red bold
    if (message.contains("damage") && message.contains("life:"))
        || (message.contains("takes") && message.contains("damage"))
    {
        return Style::default().fg(Color::Red).add_modifier(Modifier::BOLD);
    }

    // Life gain: green
    if message.contains("gains") && message.contains("life") {
        return Style::default().fg(Color::Green);
    }

    // Resolution: green
    if message.contains("resolves") {
        return Style::default().fg(Color::Green);
    }

    // Mana tapping: dark gray
    if (message.contains("Tap ") && message.contains("for {"))
        || (message.contains("taps") && message.contains("for {"))
    {
        return Style::default().fg(Color::DarkGray);
    }

    // Target selection: dark gray (auxiliary info)
    if message.starts_with("  → targeting") {
        return Style::default().fg(Color::DarkGray);
    }

    // Choice markers: cyan dim
    if message.starts_with("<Choice>") {
        return Style::default().fg(Color::Cyan).add_modifier(Modifier::DIM);
    }

    // Player-based coloring
    if message.starts_with("Player1") || message.contains(" Player1 ") {
        return Style::default().fg(Color::Blue);
    }
    if message.starts_with("Player2") || message.contains(" Player2 ") {
        return Style::default().fg(Color::Red);
    }

    // Default: use verbosity-based coloring
    match level {
        VerbosityLevel::Silent => Style::default().fg(Color::DarkGray),
        VerbosityLevel::Minimal => Style::default().fg(Color::Gray),
        VerbosityLevel::Normal => Style::default().fg(Color::White),
        VerbosityLevel::Verbose => Style::default().fg(Color::Yellow),
    }
}

/// Currently focused pane for keyboard navigation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusedPane {
    /// (H)and pane
    Hand,
    /// (L)og pane
    Log,
    /// (Y)our battlefield
    YourBattlefield,
    /// (O)pponent battlefield
    OpponentBattlefield,
    /// (A)ctions pane (Prompt + Stack)
    Actions,
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

/// A single wrapped line from a log entry
#[derive(Debug, Clone)]
pub struct WrappedLogLine {
    /// Index of the original log entry this came from
    pub original_idx: usize,
    /// Style for this line
    pub style: Style,
    /// The text content of this wrapped line
    pub text: String,
}

/// Prefix for log lines that should be filtered from TUI display
/// These lines are useful for debugging/accounting but clutter the game history
const LOG_FILTER_PREFIX: &str = "<Choice>";

/// Cache for wrapped log lines - enables efficient scrolling with word wrap
#[derive(Debug, Clone, Default)]
pub struct LogWrapCache {
    /// All wrapped lines (flattened from original log entries)
    pub lines: Vec<WrappedLogLine>,
    /// For each original log entry, the index of its first wrapped line in `lines`
    /// line_starts[i] = first index in `lines` for original log entry i
    pub line_starts: Vec<usize>,
    /// Width used to compute this cache (0 = invalid/needs rebuild)
    pub width: u16,
    /// Number of original log entries processed into this cache
    pub processed_count: usize,
}

impl LogWrapCache {
    /// Check if cache needs full rebuild (width changed or empty)
    pub fn needs_rebuild(&self, current_width: u16) -> bool {
        self.width == 0 || self.width != current_width
    }

    /// Check if cache needs incremental update (new log entries)
    pub fn needs_update(&self, total_log_entries: usize) -> bool {
        self.processed_count < total_log_entries
    }

    /// Clear the cache (forces rebuild on next render)
    pub fn invalidate(&mut self) {
        self.lines.clear();
        self.line_starts.clear();
        self.width = 0;
        self.processed_count = 0;
    }

    /// Wrap a single log message into multiple lines
    fn wrap_message(message: &str, width: usize) -> Vec<String> {
        if width == 0 {
            return vec![message.to_string()];
        }

        let mut result = Vec::new();
        let mut remaining = message;

        while !remaining.is_empty() {
            if remaining.len() <= width {
                result.push(remaining.to_string());
                break;
            }

            // Find a good break point (prefer word boundaries)
            let break_at = remaining[..width]
                .rfind(' ')
                .filter(|&pos| pos > width / 2) // Don't break too early
                .unwrap_or(width);

            result.push(remaining[..break_at].to_string());
            remaining = remaining[break_at..].trim_start();
        }

        if result.is_empty() {
            result.push(String::new());
        }

        result
    }

    /// Rebuild entire cache from scratch
    pub fn rebuild(&mut self, logs: &[crate::game::logger::LogEntry], width: u16) {
        self.lines.clear();
        self.line_starts.clear();
        self.width = width;
        self.processed_count = 0;

        let wrap_width = width.saturating_sub(1) as usize;

        for (idx, entry) in logs.iter().enumerate() {
            // Filter out <Choice> lines - they clutter the game history
            if entry.message.starts_with(LOG_FILTER_PREFIX) {
                self.line_starts.push(self.lines.len()); // Still track for index mapping
                continue;
            }

            self.line_starts.push(self.lines.len());

            // Use content-aware coloring
            let style = style_for_log_content(&entry.message, entry.level);

            let wrapped = Self::wrap_message(&entry.message, wrap_width);
            for text in wrapped {
                self.lines.push(WrappedLogLine {
                    original_idx: idx,
                    style,
                    text,
                });
            }
        }

        self.processed_count = logs.len();
    }

    /// Incrementally add new log entries to cache
    pub fn update(&mut self, logs: &[crate::game::logger::LogEntry], width: u16) {
        // If width changed, need full rebuild
        if self.width != width {
            self.rebuild(logs, width);
            return;
        }

        let wrap_width = width.saturating_sub(1) as usize;

        for (idx, entry) in logs.iter().enumerate().skip(self.processed_count) {
            // Filter out <Choice> lines - they clutter the game history
            if entry.message.starts_with(LOG_FILTER_PREFIX) {
                self.line_starts.push(self.lines.len()); // Still track for index mapping
                continue;
            }

            self.line_starts.push(self.lines.len());

            // Use content-aware coloring
            let style = style_for_log_content(&entry.message, entry.level);

            let wrapped = Self::wrap_message(&entry.message, wrap_width);
            for text in wrapped {
                self.lines.push(WrappedLogLine {
                    original_idx: idx,
                    style,
                    text,
                });
            }
        }

        self.processed_count = logs.len();
    }

    /// Map an unwrapped log index + offset to a wrapped line offset
    /// Returns the offset from the end in wrapped lines
    pub fn unwrapped_to_wrapped_offset(
        &self,
        unwrapped_offset: usize,
        total_unwrapped: usize,
        visible_lines: usize,
    ) -> usize {
        if unwrapped_offset == 0 {
            return 0; // Follow mode stays at bottom
        }

        // Find which original log entry is at the top of the visible area
        let end_idx = total_unwrapped.saturating_sub(unwrapped_offset);
        let first_visible_idx = end_idx.saturating_sub(visible_lines);

        if first_visible_idx >= self.line_starts.len() {
            return 0;
        }

        // Get the wrapped line index for that original entry
        let wrapped_line_idx = self.line_starts[first_visible_idx];
        let total_wrapped = self.lines.len();

        // Calculate offset from end in wrapped lines
        total_wrapped.saturating_sub(wrapped_line_idx + visible_lines)
    }

    /// Map a wrapped line offset to an unwrapped log offset
    /// Returns the offset from the end in original log entries
    pub fn wrapped_to_unwrapped_offset(
        &self,
        wrapped_offset: usize,
        total_unwrapped: usize,
        visible_lines: usize,
    ) -> usize {
        if wrapped_offset == 0 {
            return 0; // Follow mode stays at bottom
        }

        let total_wrapped = self.lines.len();
        let end_idx = total_wrapped.saturating_sub(wrapped_offset);
        let first_visible_wrapped = end_idx.saturating_sub(visible_lines);

        if first_visible_wrapped >= self.lines.len() {
            return 0;
        }

        // Get the original log entry for this wrapped line
        let original_idx = self.lines[first_visible_wrapped].original_idx;

        // Calculate offset from end in original entries
        total_unwrapped.saturating_sub(original_idx + visible_lines)
    }
}

/// Item in battlefield word-wrap layout - either a section label or a card entity
#[derive(Debug, Clone)]
enum BattlefieldItem {
    /// Section label (e.g., "Lands:", "Creatures:")
    Label {
        text: String,
        color: Color,
        #[allow(dead_code)] // Reserved for future use (section-aware line breaking)
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
        // Note: Stack counts (2X, 3X) are rendered at the upper-right of the card header,
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
    /// MIN bounds (cell-based, for TUI text rendering)
    pub area: Rect,
    /// MAX bounds (pixel-based, for image layout spacing)
    /// If None, defaults to area converted to pixels
    pub layout_area_px: Option<LayoutAreaPx>,
}

/// Pixel-based layout area for image positioning
/// Uses f32 for sub-pixel precision (no rounding waste)
#[derive(Debug, Clone, Copy)]
pub struct LayoutAreaPx {
    pub x_px: f32,
    pub y_px: f32,
    pub width_px: f32,
    pub height_px: f32,
}

/// Bounding boxes for card layout
///
/// MIN is in cells (for TUI text), MAX is in pixels (for images).
/// In CLI mode, MAX equals MIN converted to pixels.
/// In GUI mode, MAX may be larger to achieve correct MTG card aspect ratio.
#[derive(Debug, Clone, Copy)]
pub struct CardBounds {
    /// Cell dimensions for TUI text rendering
    pub min_width: u16,
    pub min_height: u16,
    /// Pixel dimensions for layout spacing (precise, no cell-rounding waste)
    pub max_width_px: f32,
    pub max_height_px: f32,
}

impl CardBounds {
    /// MTG card aspect ratio: 63mm × 88mm = 0.716 (portrait, width/height)
    pub const MTG_ASPECT_RATIO: f32 = 63.0 / 88.0;
    /// MTG tapped card aspect ratio: 88mm × 63mm = 1.397 (landscape, width/height)
    pub const MTG_TAPPED_RATIO: f32 = 88.0 / 63.0;

    /// Create bounds for CLI mode (MAX = MIN in pixels)
    pub fn for_cli(min_w: u16, min_h: u16, cell_w_px: f32, cell_h_px: f32) -> Self {
        Self {
            min_width: min_w,
            min_height: min_h,
            max_width_px: f32::from(min_w) * cell_w_px,
            max_height_px: f32::from(min_h) * cell_h_px,
        }
    }

    /// Create bounds for GUI mode (untapped card, portrait orientation)
    ///
    /// Expands one dimension to achieve MTG 63:88 aspect ratio.
    /// The MIN cell bounds are preserved, MAX is expanded as needed.
    pub fn for_gui(min_w: u16, min_h: u16, cell_w_px: f32, cell_h_px: f32) -> Self {
        let min_w_px = f32::from(min_w) * cell_w_px;
        let min_h_px = f32::from(min_h) * cell_h_px;

        // Current aspect ratio vs target (portrait: 63/88 ≈ 0.716)
        let current_ratio = min_w_px / min_h_px;

        let (max_w_px, max_h_px) = if current_ratio > Self::MTG_ASPECT_RATIO {
            // Too wide: expand height
            let expanded_h = min_w_px / Self::MTG_ASPECT_RATIO;
            (min_w_px, expanded_h)
        } else {
            // Too tall: expand width
            let expanded_w = min_h_px * Self::MTG_ASPECT_RATIO;
            (expanded_w, min_h_px)
        };

        Self {
            min_width: min_w,
            min_height: min_h,
            max_width_px: max_w_px,
            max_height_px: max_h_px,
        }
    }

    /// Create bounds for GUI mode (tapped card, landscape orientation)
    ///
    /// Uses MTG 88:63 aspect ratio (tapped = rotated 90°).
    pub fn for_gui_tapped(min_w: u16, min_h: u16, cell_w_px: f32, cell_h_px: f32) -> Self {
        let min_w_px = f32::from(min_w) * cell_w_px;
        let min_h_px = f32::from(min_h) * cell_h_px;

        // Current aspect ratio vs target (landscape: 88/63 ≈ 1.397)
        let current_ratio = min_w_px / min_h_px;

        let (max_w_px, max_h_px) = if current_ratio > Self::MTG_TAPPED_RATIO {
            // Too wide: expand height
            let expanded_h = min_w_px / Self::MTG_TAPPED_RATIO;
            (min_w_px, expanded_h)
        } else {
            // Too tall: expand width
            let expanded_w = min_h_px * Self::MTG_TAPPED_RATIO;
            (expanded_w, min_h_px)
        };

        Self {
            min_width: min_w,
            min_height: min_h,
            max_width_px: max_w_px,
            max_height_px: max_h_px,
        }
    }
}

/// Runtime configuration for rendering
///
/// Initialized from compile-time feature (`wasm-tui` → gui_mode=true)
/// and runtime cell dimension measurements from JavaScript.
#[derive(Debug, Clone, Copy)]
pub struct RenderConfig {
    /// Whether we're in GUI mode (images enabled)
    /// Set by wasm-tui feature at compile time
    pub gui_mode: bool,
    /// Cell width in pixels (measured from browser font metrics)
    pub cell_width_px: f32,
    /// Cell height in pixels (measured from browser font metrics)
    pub cell_height_px: f32,
    /// Background color for panes in GUI mode (None = transparent/default)
    /// In CLI mode this is always None to preserve terminal background
    pub pane_bg_color: Option<Color>,
}

impl Default for RenderConfig {
    fn default() -> Self {
        Self {
            gui_mode: false,
            cell_width_px: 10.0,
            cell_height_px: 20.0,
            pane_bg_color: None,
        }
    }
}

impl RenderConfig {
    /// Create config for CLI mode (no image layout expansion)
    pub fn cli() -> Self {
        Self {
            gui_mode: false,
            cell_width_px: 10.0, // RatZilla default
            cell_height_px: 20.0,
            pane_bg_color: None, // Preserve terminal background
        }
    }

    /// Create config for GUI mode with specified cell dimensions
    pub fn gui(cell_width_px: f32, cell_height_px: f32) -> Self {
        Self {
            gui_mode: true,
            cell_width_px,
            cell_height_px,
            // Dark grey background so black card borders are visible
            // #1c1c1c = RGB(28, 28, 28)
            pane_bg_color: Some(Color::Rgb(28, 28, 28)),
        }
    }
}

/// UI state for the fancy TUI renderer
pub struct FancyTuiState {
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
    /// Card Details pane area (for image overlay positioning)
    pub card_details_pane_area: Option<Rect>,
    /// Log pane area (for mouse click detection and scroll wheel)
    pub log_pane_area: Option<Rect>,
    /// Rewind message to display after undo operation
    pub rewind_message: Option<String>,
    /// Log scroll offset (0 = follow mode, showing latest; >0 = scrolled up by N lines)
    /// In unwrapped mode: counts original log entries from end
    /// In wrapped mode: counts wrapped lines from end
    pub log_scroll_offset: usize,
    /// Horizontal scroll offset for log pane (characters from left)
    pub log_horizontal_offset: usize,
    /// Whether to wrap lines in the log pane
    pub log_wrap_lines: bool,
    /// Actual visible lines in log pane (updated during render)
    pub log_visible_lines: usize,
    /// Cache of wrapped log lines for efficient scrolling
    pub log_wrap_cache: LogWrapCache,
    /// Buffer for multi-digit choice input (>10 choices)
    pub digit_buffer: String,
}

impl Default for FancyTuiState {
    fn default() -> Self {
        Self::new()
    }
}

impl FancyTuiState {
    pub fn new() -> Self {
        Self {
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
            card_details_pane_area: None,
            log_pane_area: None,
            rewind_message: None,
            log_scroll_offset: 0,     // 0 = follow mode (show latest)
            log_horizontal_offset: 0, // 0 = no horizontal scroll
            log_wrap_lines: false,    // Default: no line wrapping (truncate)
            log_visible_lines: 20,    // Default estimate, updated during render
            log_wrap_cache: LogWrapCache::default(),
            digit_buffer: String::new(),
        }
    }

    /// Scroll log up by one line (away from latest)
    pub fn log_scroll_up(&mut self, total_lines: usize, visible_lines: usize) {
        // Can only scroll up if there are more lines than visible
        let max_offset = total_lines.saturating_sub(visible_lines);
        if self.log_scroll_offset < max_offset {
            self.log_scroll_offset += 1;
        }
    }

    /// Scroll log down by one line (toward latest)
    pub fn log_scroll_down(&mut self) {
        if self.log_scroll_offset > 0 {
            self.log_scroll_offset -= 1;
        }
    }

    /// Scroll log up by a page
    pub fn log_page_up(&mut self, total_lines: usize, visible_lines: usize) {
        let max_offset = total_lines.saturating_sub(visible_lines);
        let page_size = visible_lines.saturating_sub(1).max(1);
        self.log_scroll_offset = (self.log_scroll_offset + page_size).min(max_offset);
    }

    /// Scroll log down by a page
    pub fn log_page_down(&mut self, visible_lines: usize) {
        let page_size = visible_lines.saturating_sub(1).max(1);
        self.log_scroll_offset = self.log_scroll_offset.saturating_sub(page_size);
    }

    /// Scroll to beginning of log (oldest messages)
    pub fn log_scroll_home(&mut self, total_lines: usize, visible_lines: usize) {
        self.log_scroll_offset = total_lines.saturating_sub(visible_lines);
    }

    /// Scroll to end of log (latest messages, follow mode)
    pub fn log_scroll_end(&mut self) {
        self.log_scroll_offset = 0;
    }

    /// Scroll log left (toward beginning of lines)
    pub fn log_scroll_left(&mut self) {
        // Scroll 4 characters at a time for usability
        self.log_horizontal_offset = self.log_horizontal_offset.saturating_sub(4);
    }

    /// Scroll log right (toward end of lines)
    pub fn log_scroll_right(&mut self) {
        // Scroll 4 characters at a time, capped at reasonable max
        // (lines rarely exceed 200 chars)
        if self.log_horizontal_offset < 200 {
            self.log_horizontal_offset += 4;
        }
    }

    /// Reset horizontal scroll to leftmost position
    pub fn log_scroll_reset_horizontal(&mut self) {
        self.log_horizontal_offset = 0;
    }

    /// Toggle line wrapping in log, preserving scroll position
    /// - In follow mode (offset=0): stay at bottom
    /// - In scrollback mode: keep the same first visible line
    pub fn log_toggle_wrap(&mut self, total_unwrapped: usize) {
        let visible_lines = self.log_visible_lines;

        if self.log_wrap_lines {
            // Switching FROM wrapped TO unwrapped
            // Map the current wrapped offset to an unwrapped offset
            if self.log_scroll_offset > 0 {
                self.log_scroll_offset = self.log_wrap_cache.wrapped_to_unwrapped_offset(
                    self.log_scroll_offset,
                    total_unwrapped,
                    visible_lines,
                );
            }
        } else {
            // Switching FROM unwrapped TO wrapped
            // Map the current unwrapped offset to a wrapped offset
            if self.log_scroll_offset > 0 {
                self.log_scroll_offset = self.log_wrap_cache.unwrapped_to_wrapped_offset(
                    self.log_scroll_offset,
                    total_unwrapped,
                    visible_lines,
                );
            }
        }

        self.log_wrap_lines = !self.log_wrap_lines;
    }

    /// Scroll log to previous turn (Left arrow)
    /// Scrolls up until a new ">>> Turn" header appears at the top of the visible area
    pub fn log_scroll_prev_turn(&mut self, logs: &[crate::game::logger::LogEntry], visible_lines: usize) {
        if self.log_wrap_lines {
            self.log_scroll_prev_turn_wrapped(visible_lines);
        } else {
            self.log_scroll_prev_turn_unwrapped(logs, visible_lines);
        }
    }

    fn log_scroll_prev_turn_unwrapped(&mut self, logs: &[crate::game::logger::LogEntry], visible_lines: usize) {
        // Filter out <Choice> lines to match the display
        let filtered_logs: Vec<_> = logs
            .iter()
            .filter(|e| !e.message.starts_with(LOG_FILTER_PREFIX))
            .collect();

        let total_lines = filtered_logs.len();
        let max_offset = total_lines.saturating_sub(visible_lines);

        // Calculate current first visible line index
        let current_end = total_lines.saturating_sub(self.log_scroll_offset);
        let current_start = current_end.saturating_sub(visible_lines);

        // Find the next turn header above current_start
        // We need to find a turn header that would be at line 0 of the visible area
        for (idx, entry) in filtered_logs[..current_start].iter().enumerate().rev() {
            if entry.message.contains(">>> Turn") {
                // Found a turn header - calculate offset to put it at top
                // If this line is at index `idx`, we want:
                // start_idx = idx, so end_idx = idx + visible_lines
                // scroll_offset = total_lines - end_idx
                let new_end = (idx + visible_lines).min(total_lines);
                self.log_scroll_offset = total_lines.saturating_sub(new_end);
                self.log_scroll_offset = self.log_scroll_offset.min(max_offset);
                return;
            }
        }

        // No turn header found above - scroll to beginning
        self.log_scroll_offset = max_offset;
    }

    fn log_scroll_prev_turn_wrapped(&mut self, visible_lines: usize) {
        let total_lines = self.log_wrap_cache.lines.len();
        let max_offset = total_lines.saturating_sub(visible_lines);

        // Calculate current first visible line index
        let current_end = total_lines.saturating_sub(self.log_scroll_offset);
        let current_start = current_end.saturating_sub(visible_lines);

        // Find the next turn header above current_start in wrapped lines
        for idx in (0..current_start).rev() {
            if self.log_wrap_cache.lines[idx].text.contains(">>> Turn") {
                // Found a turn header - calculate offset to put it at top
                let new_end = (idx + visible_lines).min(total_lines);
                self.log_scroll_offset = total_lines.saturating_sub(new_end);
                self.log_scroll_offset = self.log_scroll_offset.min(max_offset);
                return;
            }
        }

        // No turn header found above - scroll to beginning
        self.log_scroll_offset = max_offset;
    }

    /// Scroll log to next turn (Right arrow)
    /// Scrolls down until the next ">>> Turn" header appears at the top of the visible area
    pub fn log_scroll_next_turn(&mut self, logs: &[crate::game::logger::LogEntry], visible_lines: usize) {
        if self.log_wrap_lines {
            self.log_scroll_next_turn_wrapped(visible_lines);
        } else {
            self.log_scroll_next_turn_unwrapped(logs, visible_lines);
        }
    }

    fn log_scroll_next_turn_unwrapped(&mut self, logs: &[crate::game::logger::LogEntry], visible_lines: usize) {
        // Filter out <Choice> lines to match the display
        let filtered_logs: Vec<_> = logs
            .iter()
            .filter(|e| !e.message.starts_with(LOG_FILTER_PREFIX))
            .collect();

        let total_lines = filtered_logs.len();

        // Calculate current first visible line index
        let current_end = total_lines.saturating_sub(self.log_scroll_offset);
        let current_start = current_end.saturating_sub(visible_lines);

        // Find the next turn header after current_start
        let search_start = (current_start + 1).min(total_lines);
        for (offset, entry) in filtered_logs[search_start..].iter().enumerate() {
            if entry.message.contains(">>> Turn") {
                // Found a turn header - calculate offset to put it at top
                let idx = search_start + offset;
                let new_end = (idx + visible_lines).min(total_lines);
                self.log_scroll_offset = total_lines.saturating_sub(new_end);
                return;
            }
        }

        // No turn header found below - scroll to end (follow mode)
        self.log_scroll_offset = 0;
    }

    fn log_scroll_next_turn_wrapped(&mut self, visible_lines: usize) {
        let total_lines = self.log_wrap_cache.lines.len();

        // Calculate current first visible line index
        let current_end = total_lines.saturating_sub(self.log_scroll_offset);
        let current_start = current_end.saturating_sub(visible_lines);

        // Find the next turn header after current_start in wrapped lines
        for idx in (current_start + 1)..total_lines {
            if self.log_wrap_cache.lines[idx].text.contains(">>> Turn") {
                // Found a turn header - calculate offset to put it at top
                let new_end = (idx + visible_lines).min(total_lines);
                self.log_scroll_offset = total_lines.saturating_sub(new_end);
                return;
            }
        }

        // No turn header found below - scroll to end (follow mode)
        self.log_scroll_offset = 0;
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
    /// Render configuration (GUI mode, cell dimensions)
    pub render_config: RenderConfig,
}

impl FancyTuiRenderer {
    // Minimum acceptable widths for each pane (in terminal columns)
    pub const MIN_WIDTH_LOG_PANE: u16 = 40; // Log pane (left column top)
    pub const MIN_WIDTH_ACTIONS_PANE: u16 = 40; // Prompt/Actions pane (left column bottom, includes stack)
    pub const MIN_WIDTH_CARD_DETAILS: u16 = 30; // Card details pane (right column top)
    pub const MIN_WIDTH_HAND: u16 = 30; // Hand pane (right column bottom)
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

    /// Create a new renderer for a player (CLI mode - no image layout expansion)
    pub fn new(player_id: PlayerId, visual_stacks: bool) -> Self {
        FancyTuiRenderer {
            player_id,
            state: FancyTuiState::new(),
            visual_stacks,
            render_config: RenderConfig::cli(),
        }
    }

    /// Create a new renderer for GUI mode with specified cell dimensions
    pub fn new_gui(player_id: PlayerId, visual_stacks: bool, cell_width_px: f32, cell_height_px: f32) -> Self {
        FancyTuiRenderer {
            player_id,
            state: FancyTuiState::new(),
            visual_stacks,
            render_config: RenderConfig::gui(cell_width_px, cell_height_px),
        }
    }

    /// Update cell dimensions (called when JavaScript measures actual font metrics)
    pub fn set_cell_dimensions(&mut self, cell_width_px: f32, cell_height_px: f32) {
        self.render_config.cell_width_px = cell_width_px;
        self.render_config.cell_height_px = cell_height_px;
    }

    /// Enable or disable GUI mode
    pub fn set_gui_mode(&mut self, enabled: bool) {
        self.render_config.gui_mode = enabled;
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
        // Format: Turn: <global_turn> (<player_turn>)
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

    /// Get all cards for a battlefield in display order
    /// Order: Planeswalkers → Creatures → Enchantments → Artifacts → Lands
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

        // Group cards: planeswalkers, creatures, enchantments, artifacts, lands
        let (planeswalkers, creatures, enchantments, artifacts, lands): (Vec<_>, Vec<_>, Vec<_>, Vec<_>, Vec<_>) =
            player_cards.iter().fold(
                (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new()),
                |(mut planeswalkers, mut creatures, mut enchantments, mut artifacts, mut lands), &card_id| {
                    if let Some(card) = view.get_card(card_id) {
                        // Check types in priority order (a card can have multiple types)
                        if card.is_planeswalker() {
                            planeswalkers.push(card_id);
                        } else if card.is_creature() {
                            creatures.push(card_id);
                        } else if card.is_enchantment() {
                            enchantments.push(card_id);
                        } else if card.is_artifact() {
                            artifacts.push(card_id);
                        } else if card.is_land() {
                            lands.push(card_id);
                        }
                    }
                    (planeswalkers, creatures, enchantments, artifacts, lands)
                },
            );

        // Concatenate in display order: PWs → Creatures → Enchants → Artifacts → Lands
        let mut result = Vec::new();
        result.extend(planeswalkers);
        result.extend(creatures);
        result.extend(enchantments);
        result.extend(artifacts);
        result.extend(lands);
        result
    }

    /// Get the hand sorted in display order: lands first, then by descending CMC.
    ///
    /// This must be used by both the renderer and event handler so that
    /// index-based hand navigation matches the visual display order.
    pub fn get_sorted_hand(view: &GameStateView) -> Vec<CardId> {
        let hand = view.hand();
        let mut sorted: Vec<CardId> = hand.to_vec();
        sorted.sort_by(|&a, &b| {
            let card_a = view.get_card(a);
            let card_b = view.get_card(b);

            // Lands first
            let a_is_land = card_a.map(|c| c.is_land()).unwrap_or(false);
            let b_is_land = card_b.map(|c| c.is_land()).unwrap_or(false);

            match (a_is_land, b_is_land) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => {
                    // Both lands or both non-lands: sort by descending CMC
                    let a_cmc = card_a.map(|c| c.mana_cost.cmc()).unwrap_or(0);
                    let b_cmc = card_b.map(|c| c.mana_cost.cmc()).unwrap_or(0);
                    b_cmc.cmp(&a_cmc) // Descending order
                }
            }
        });
        sorted
    }

    /// Group cards into battlefield entities
    ///
    /// Groups cards by name (and P/T for creatures), then uses a mode-specific constructor
    /// to create entities. Creatures with different power/toughness values are placed in
    /// separate stacks even if they have the same name.
    ///
    /// With visual_stacks=true: creates VisualStack entities with diagonal offsets
    /// With visual_stacks=false: creates separate SimpleStack entities for tapped/untapped
    pub fn group_cards_into_entities(&self, cards: &[CardId], view: &GameStateView) -> Vec<Entity> {
        // Group cards by name AND P/T (for creatures with differing stats)
        // Key format: "CardName" for non-creatures, "CardName\x00P/T" for creatures
        // This separates tokens/creatures that have different stats from the same source
        let mut groups: HashMap<String, (String, SmallVec<[CardId; 8]>)> = HashMap::new();

        for &card_id in cards {
            let name = view.card_name(card_id).unwrap_or_else(|| format!("{:?}", card_id));

            // For creatures, include P/T in the grouping key to separate cards with different stats
            let key = if let Some(card) = view.get_card(card_id) {
                if card.is_creature() {
                    let power = view
                        .get_effective_power(card_id)
                        .unwrap_or_else(|| i32::from(card.current_power()));
                    let toughness = view
                        .get_effective_toughness(card_id)
                        .unwrap_or_else(|| i32::from(card.current_toughness()));
                    format!("{}\x00{}/{}", name, power, toughness)
                } else {
                    name.clone()
                }
            } else {
                name.clone()
            };
            groups
                .entry(key)
                .or_insert_with(|| (name.clone(), SmallVec::new()))
                .1
                .push(card_id);
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
        // The key includes P/T for creatures but we use the actual card_name for display
        let mut entities: Vec<Entity> = groups
            .into_iter()
            .flat_map(|(_key, (card_name, card_ids))| construct_entities(card_name, card_ids))
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
        ((f32::from(height) * f32::from(Self::DEFAULT_CARD_WIDTH)) / f32::from(Self::DEFAULT_CARD_HEIGHT)).round()
            as u16
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

    /// Get LAYOUT dimensions for an entity (MAX bounds in cells).
    ///
    /// In GUI mode, this returns larger dimensions that respect MTG card aspect ratio.
    /// In CLI mode, this returns the same as `get_entity_dimensions()`.
    ///
    /// The layout dimensions are the card's "public" size - used for:
    /// - Spacing between cards
    /// - Row wrapping decisions
    /// - Hit-testing / click detection
    /// - Image overlay sizing
    ///
    /// The MIN dimensions (from `get_entity_dimensions`) are internal - used only for
    /// determining where TUI text is rendered within the larger layout box.
    pub fn get_entity_layout_dimensions(
        &self,
        entity: &Entity,
        view: &GameStateView,
        base_width: u16,
        base_height: u16,
    ) -> (u16, u16) {
        // Get MIN dimensions first (TUI text area)
        let (min_w, min_h) = Self::get_entity_dimensions(entity, view, base_width, base_height);

        if !self.render_config.gui_mode {
            // CLI mode: layout = min (no expansion)
            return (min_w, min_h);
        }

        // GUI mode: calculate MAX bounds with correct MTG aspect ratio
        let is_tapped = entity.is_tapped(view);
        let bounds = if is_tapped {
            CardBounds::for_gui_tapped(
                min_w,
                min_h,
                self.render_config.cell_width_px,
                self.render_config.cell_height_px,
            )
        } else {
            CardBounds::for_gui(
                min_w,
                min_h,
                self.render_config.cell_width_px,
                self.render_config.cell_height_px,
            )
        };

        // Convert MAX pixels back to cells (ceiling to ensure we don't undersell space)
        let layout_w = (bounds.max_width_px / self.render_config.cell_width_px).ceil() as u16;
        let layout_h = (bounds.max_height_px / self.render_config.cell_height_px).ceil() as u16;

        // Layout dimensions must be at least as large as MIN
        (layout_w.max(min_w), layout_h.max(min_h))
    }

    /// Get both MIN and LAYOUT dimensions for an entity.
    ///
    /// Returns ((min_w, min_h), (layout_w, layout_h)).
    /// - MIN: TUI text rendering area (internal)
    /// - LAYOUT: Card's public bounding box (spacing, hit-testing, images)
    pub fn get_entity_min_and_layout_dimensions(
        &self,
        entity: &Entity,
        view: &GameStateView,
        base_width: u16,
        base_height: u16,
    ) -> ((u16, u16), (u16, u16)) {
        let min_dims = Self::get_entity_dimensions(entity, view, base_width, base_height);
        let layout_dims = self.get_entity_layout_dimensions(entity, view, base_width, base_height);
        (min_dims, layout_dims)
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
        self.state.log_pane_area = None;

        // Calculate optimal column widths
        // Try to boost left column width by 20% if all panes remain above their minimums
        let total_width = f.area().width;

        // Calculate what the widths would be with boosted left column
        let boosted_left_width = (total_width * Self::BOOSTED_LEFT_COLUMN_PCT) / 100;
        let boosted_middle_width = (total_width * (Self::DEFAULT_MIDDLE_COLUMN_PCT - 5)) / 100; // Reduce middle by 5%
        let boosted_right_width = total_width.saturating_sub(boosted_left_width + boosted_middle_width);

        // Check if all panes would meet their minimum widths with boosted layout
        let can_boost = boosted_left_width >= Self::MIN_WIDTH_LOG_PANE
            && boosted_left_width >= Self::MIN_WIDTH_ACTIONS_PANE
            && boosted_middle_width >= Self::MIN_WIDTH_BATTLEFIELD
            && boosted_right_width >= Self::MIN_WIDTH_CARD_DETAILS
            && boosted_right_width >= Self::MIN_WIDTH_HAND;

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

        // Left column: Log on top, Actions/Prompt on bottom
        let left_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(main_chunks[0]);

        // Middle column: Player info bars + battlefields
        // Layout: Top half (opp info + opp battlefield), Bottom half (your battlefield + your info)
        // Split 50/50 to align with left/right column midpoint
        let middle_halves = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(main_chunks[1]);

        // Calculate info bar height based on whether status fits on one line
        let info_bar_height = Self::calculate_info_bar_height(main_chunks[1].width);

        // Top half: Opponent info bar at top, battlefield fills rest
        let top_half = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(info_bar_height), // Opponent info header (at top)
                Constraint::Min(0),                  // Opponent battlefield (fills remaining)
            ])
            .split(middle_halves[0]);

        // Bottom half: Battlefield fills most, your info bar at bottom
        let bottom_half = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(0),                  // Your battlefield (fills remaining)
                Constraint::Length(info_bar_height), // Your info footer (at bottom)
            ])
            .split(middle_halves[1]);

        // Combine into middle_chunks for compatibility with existing code
        let middle_chunks = [top_half[0], top_half[1], bottom_half[0], bottom_half[1]];

        // Right column: Card Details (top 50%), Hand+Stack (bottom 50%)
        // Matches left column layout: Log (top 50%), Actions (bottom 50%)
        // Card Details starts at row 0, aligned with Log pane
        let right_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(main_chunks[2]);

        // Draw all panels
        self.draw_log_pane(f, left_chunks[0], view);
        self.state.log_pane_area = Some(left_chunks[0]);
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
        // right_chunks[0] is Card Details (starts at row 0, aligned with Log pane)
        // right_chunks[1] is Hand+Stack (aligned with Actions pane)
        self.draw_card_details(f, right_chunks[0], view);
        self.draw_hand(f, right_chunks[1], view);
        self.state.hand_pane_area = Some(right_chunks[1]);
        self.state.card_details_pane_area = Some(right_chunks[0]);
    }

    /// Draw the Log pane (left column top)
    fn draw_log_pane(&mut self, f: &mut Frame, area: Rect, view: &GameStateView) {
        let is_focused = self.state.focused_pane == FocusedPane::Log;
        let border_color = if is_focused { Color::Cyan } else { Color::Reset };

        // Draw the block with title
        let block = Block::default()
            .borders(Borders::ALL)
            .title(if is_focused { " Log (I) [FOCUSED] " } else { " Log (I) " })
            .border_style(Style::default().fg(border_color));
        f.render_widget(block, area);

        // Content area inside the border
        let content_area = Rect {
            x: area.x + 1,
            y: area.y + 1,
            width: area.width.saturating_sub(2),
            height: area.height.saturating_sub(2),
        };

        self.draw_log_view(f, content_area, view);

        // Render log status in the title bar (right side of top border)
        let visible_lines = content_area.height as usize;
        let (total_lines, scroll_offset) = if self.state.log_wrap_lines {
            let total = self.state.log_wrap_cache.lines.len();
            let max_offset = total.saturating_sub(visible_lines);
            (total, self.state.log_scroll_offset.min(max_offset))
        } else {
            // Must use filtered count to match draw_log_view_unwrapped
            let logs = view.logger().logs();
            let filtered_count = logs
                .iter()
                .filter(|e| !e.message.starts_with(LOG_FILTER_PREFIX))
                .count();
            let max_offset = filtered_count.saturating_sub(visible_lines);
            (filtered_count, self.state.log_scroll_offset.min(max_offset))
        };
        let end_idx = total_lines.saturating_sub(scroll_offset);
        let start_idx = end_idx.saturating_sub(visible_lines);

        // Build status string
        let status_text = if scroll_offset == 0 {
            format!("{}-{}/{} [F]", start_idx + 1, end_idx, total_lines)
        } else {
            format!("{}-{}/{}", start_idx + 1, end_idx, total_lines)
        };
        let wrap_indicator = if self.state.log_wrap_lines { " [W]" } else { "" };
        let full_status = format!("{}{}", status_text, wrap_indicator);

        // Render status on the title bar line (y = area.y, right-aligned before border)
        let status_width = full_status.len() as u16;
        let title_len = if is_focused { 18 } else { 10 }; // " Log (I) [FOCUSED] " or " Log (I) "
        let available_width = area.width.saturating_sub(title_len + 2); // -2 for border corners
        if status_width <= available_width {
            let status_area = Rect {
                x: area.x + area.width - status_width - 1, // -1 for right border
                y: area.y,                                 // Title bar line
                width: status_width,
                height: 1,
            };
            let status_span = Span::styled(full_status, Style::default().fg(Color::DarkGray));
            f.render_widget(Paragraph::new(Line::from(status_span)), status_area);
        }

        // Draw scroll indicator on right border when in scrollback mode
        if scroll_offset > 0 && total_lines > visible_lines {
            self.draw_scroll_indicator(f, area, total_lines, visible_lines, start_idx, border_color);
        }
    }

    /// Draw scroll indicator on the right border of a pane
    /// Maps K border characters to N total lines, highlighting the portion representing visible lines
    fn draw_scroll_indicator(
        &self,
        f: &mut Frame,
        area: Rect,
        total_lines: usize,
        visible_lines: usize,
        start_idx: usize,
        border_color: Color,
    ) {
        // Right border characters (excluding corners): from y+1 to y+height-2
        let scroll_track_height = area.height.saturating_sub(2) as usize; // K characters
        if scroll_track_height == 0 || total_lines == 0 {
            return;
        }

        // Calculate thumb position and size
        // Thumb size (J) is proportional to visible_lines / total_lines
        // Thumb position maps start_idx to the track
        let thumb_size = ((visible_lines as f32 / total_lines as f32) * scroll_track_height as f32)
            .ceil()
            .max(1.0) as usize;
        let thumb_size = thumb_size.min(scroll_track_height);

        // Position: map start_idx (0..total_lines-visible_lines) to (0..scroll_track_height-thumb_size)
        let max_start = total_lines.saturating_sub(visible_lines);
        let max_thumb_pos = scroll_track_height.saturating_sub(thumb_size);
        let thumb_pos = if max_start > 0 {
            ((start_idx as f32 / max_start as f32) * max_thumb_pos as f32).round() as usize
        } else {
            0
        };

        // Draw each character on the right border
        let right_x = area.x + area.width - 1;
        for i in 0..scroll_track_height {
            let y = area.y + 1 + i as u16;
            let is_thumb = i >= thumb_pos && i < thumb_pos + thumb_size;

            let (ch, style) = if is_thumb {
                // Highlighted: use solid block character with border color
                ('█', Style::default().fg(border_color))
            } else {
                // Unhighlighted: normal border character
                ('│', Style::default().fg(border_color))
            };

            let cell_area = Rect {
                x: right_x,
                y,
                width: 1,
                height: 1,
            };
            f.render_widget(Paragraph::new(ch.to_string()).style(style), cell_area);
        }
    }

    /// Draw the game log view panel with scrolling support
    /// Status indicator is rendered in draw_log_pane on the title bar
    fn draw_log_view(&mut self, f: &mut Frame, area: Rect, view: &GameStateView) {
        let logs = view.logger().logs();
        let visible_lines = area.height as usize;

        // Store actual visible lines for turn navigation calculations
        self.state.log_visible_lines = visible_lines;

        if visible_lines == 0 || area.width < 10 {
            return;
        }

        if self.state.log_wrap_lines {
            // WRAPPED MODE: Use the wrap cache
            self.draw_log_view_wrapped(f, area, &logs);
        } else {
            // UNWRAPPED MODE: Original truncation logic
            self.draw_log_view_unwrapped(f, area, &logs);
        }
    }

    /// Draw log view in unwrapped mode (truncate long lines with ellipsis)
    fn draw_log_view_unwrapped(&mut self, f: &mut Frame, area: Rect, logs: &[crate::game::logger::LogEntry]) {
        // Filter out <Choice> lines for cleaner game history display
        let filtered_logs: Vec<_> = logs
            .iter()
            .filter(|e| !e.message.starts_with(LOG_FILTER_PREFIX))
            .collect();

        let total_lines = filtered_logs.len();
        let visible_lines = area.height as usize;

        // Clamp scroll offset FIRST - must happen before calculating indices
        let max_offset = total_lines.saturating_sub(visible_lines);
        if self.state.log_scroll_offset > max_offset {
            self.state.log_scroll_offset = max_offset;
        }

        // Calculate which lines to show based on scroll offset
        let end_idx = total_lines.saturating_sub(self.state.log_scroll_offset);
        let start_idx = end_idx.saturating_sub(visible_lines);

        // Build log lines with horizontal offset and truncation
        let h_offset = self.state.log_horizontal_offset;
        let log_lines: Vec<ListItem> = filtered_logs[start_idx..end_idx]
            .iter()
            .map(|entry| {
                // Use content-aware coloring
                let style = style_for_log_content(&entry.message, entry.level);
                let max_width = area.width.saturating_sub(1) as usize;

                // Apply horizontal offset first, then truncate
                let msg = &entry.message;
                let message = if h_offset >= msg.len() {
                    // Scrolled past end of line
                    String::new()
                } else {
                    let shifted = &msg[h_offset..];
                    if shifted.len() > max_width {
                        // Need left indicator if scrolled, right ellipsis if truncated
                        if h_offset > 0 {
                            format!("…{}…", &shifted[..max_width.saturating_sub(2)])
                        } else {
                            format!("{}…", &shifted[..max_width.saturating_sub(1)])
                        }
                    } else if h_offset > 0 {
                        // Show left indicator when scrolled
                        format!("…{}", shifted)
                    } else {
                        shifted.to_string()
                    }
                };
                ListItem::new(Line::from(Span::styled(message, style)))
            })
            .collect();

        let log_list = List::new(log_lines);
        f.render_widget(log_list, area);
    }

    /// Draw log view in wrapped mode (multi-line display for long messages)
    fn draw_log_view_wrapped(&mut self, f: &mut Frame, area: Rect, logs: &[crate::game::logger::LogEntry]) {
        let visible_lines = area.height as usize;

        // Update or rebuild cache as needed
        if self.state.log_wrap_cache.needs_rebuild(area.width) {
            self.state.log_wrap_cache.rebuild(logs, area.width);
        } else if self.state.log_wrap_cache.needs_update(logs.len()) {
            self.state.log_wrap_cache.update(logs, area.width);
        }

        let total_wrapped = self.state.log_wrap_cache.lines.len();

        // Clamp scroll offset for wrapped lines
        let max_offset = total_wrapped.saturating_sub(visible_lines);
        if self.state.log_scroll_offset > max_offset {
            self.state.log_scroll_offset = max_offset;
        }

        // Calculate which wrapped lines to show
        let end_idx = total_wrapped.saturating_sub(self.state.log_scroll_offset);
        let start_idx = end_idx.saturating_sub(visible_lines);

        // Build list items from wrapped cache
        let log_lines: Vec<ListItem> = self.state.log_wrap_cache.lines[start_idx..end_idx]
            .iter()
            .map(|wrapped| ListItem::new(Line::from(Span::styled(&wrapped.text, wrapped.style))))
            .collect();

        let log_list = List::new(log_lines);
        f.render_widget(log_list, area);
    }

    /// Draw the prompt/actions panel (now includes stack at bottom)
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

        // Calculate stack height: 1 line for empty, N+1 lines for N items on stack
        let stack = view.stack();
        let stack_height = if stack.is_empty() { 1 } else { stack.len() + 1 } as u16;

        // Split inner area: main content on top, stack at bottom
        let content_height = inner_area.height.saturating_sub(stack_height);
        let content_area = Rect {
            x: inner_area.x,
            y: inner_area.y,
            width: inner_area.width,
            height: content_height,
        };
        let stack_area = Rect {
            x: inner_area.x,
            y: inner_area.y + content_height,
            width: inner_area.width,
            height: stack_height,
        };

        // Render main content
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

        // Show digit buffer when non-empty (multi-digit input mode)
        if !self.state.digit_buffer.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                format!("Select: {}_ (Enter to confirm)", self.state.digit_buffer),
                Style::default().fg(Color::Cyan),
            )));
        }

        // Show navigation hints
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Up/Down: select | Enter: confirm | Esc: pass | Z: undo | R: random",
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(Span::styled(
            "H/I/Y/O/A: focus panes | Tab: cycle tabs",
            Style::default().fg(Color::DarkGray),
        )));

        let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
        f.render_widget(paragraph, content_area);

        // Render stack at bottom of Actions pane
        self.render_stack_inline(f, stack_area, view);
    }

    /// Render the stack as inline text at the bottom of the Actions pane
    fn render_stack_inline(&self, f: &mut Frame, area: Rect, view: &GameStateView) {
        let stack = view.stack();

        let mut lines = Vec::new();

        if stack.is_empty() {
            lines.push(Line::from(Span::styled(
                "Stack: (empty)",
                Style::default().fg(Color::DarkGray),
            )));
        } else {
            // Header line
            lines.push(Line::from(Span::styled(
                format!("Stack ({}):", stack.len()),
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            )));

            // Stack items (most recent first, displayed top-to-bottom)
            for (i, &card_id) in stack.iter().rev().enumerate() {
                let name = view.card_name(card_id).unwrap_or_else(|| format!("{:?}", card_id));
                let controller = view.get_card(card_id).map(|c| c.controller);
                let owner_marker = if controller == Some(view.player_id()) {
                    "(yours)"
                } else {
                    "(opp)"
                };

                // First item (top of stack, will resolve next) is highlighted
                let style = if i == 0 {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default().fg(Color::White)
                };

                let line_text = format!("  {} {}", name, owner_marker);
                lines.push(Line::from(Span::styled(line_text, style)));
            }
        }

        let paragraph = Paragraph::new(lines);
        f.render_widget(paragraph, area);
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
        graveyard_bounds: Option<Rect>,                // graveyard area to avoid collision
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

        // Calculate optimal card size and section breaks for this layout
        let (card_width, card_height, section_breaks) = self.calculate_wordwrap_card_size(area, view, &items);

        // Render using word-wrap model with section-aware breaks and centering
        self.render_wordwrap_battlefield(
            f,
            area,
            view,
            &items,
            card_width,
            card_height,
            &section_breaks,
            graveyard_bounds,
        );
    }

    /// Calculate optimal card size and section break indices for battlefield layout.
    ///
    /// Uses a smart algorithm that tries different row configurations (1-4 rows)
    /// to maximize card size. Prefers fewer rows when card sizes are equal.
    ///
    /// Returns: (card_width, card_height, section_break_indices)
    /// section_break_indices: indices into `sections` array where breaks should occur
    fn calculate_wordwrap_card_size(
        &self,
        area: Rect,
        view: &GameStateView,
        items: &[BattlefieldItem],
    ) -> (u16, u16, Vec<usize>) {
        if items.is_empty() || area.height == 0 || area.width == 0 {
            return (Self::MIN_CARD_WIDTH, Self::MIN_CARD_HEIGHT, Vec::new());
        }

        // Extract sections from items for section-aware layout
        let sections = Self::extract_sections(items);

        // Try different row counts and find the best card size for each
        let mut best_height = Self::MIN_CARD_HEIGHT;

        for target_rows in 1..=4 {
            let max_height = self.find_max_card_height_for_rows(area, view, items, &sections, target_rows);
            if max_height > best_height {
                best_height = max_height;
            }
        }

        let best_width = Self::compute_width_from_height(best_height);

        // Now find section breaks that work with this card size
        let section_breaks = self.compute_section_breaks(area, view, items, &sections, best_width, best_height);

        (best_width, best_height, section_breaks)
    }

    /// Compute which section indices should start on new rows.
    /// Returns indices into the sections array where breaks should occur.
    fn compute_section_breaks(
        &self,
        area: Rect,
        view: &GameStateView,
        items: &[BattlefieldItem],
        sections: &[(usize, usize)],
        card_width: u16,
        card_height: u16,
    ) -> Vec<usize> {
        let mut breaks = Vec::new();
        let mut current_x = 0u16;

        for (section_idx, &(start, end)) in sections.iter().enumerate() {
            // Calculate width and max height of this section using LAYOUT dimensions
            let mut section_width = 0u16;
            for item in &items[start..end] {
                if let BattlefieldItem::Card { entity } = item {
                    let (card_w, _) = self.get_entity_layout_dimensions(entity, view, card_width, card_height);
                    section_width += card_w + Self::CARD_SPACING;
                }
            }

            // Check if section fits on current row
            if current_x > 0 && current_x + section_width > area.width {
                // Need to break before this section
                if section_idx > 0 {
                    breaks.push(section_idx);
                    current_x = 0;
                }
            }

            // If section is too wide for a single row, we don't break at section level
            // (natural wrapping within the section will handle it)
            if section_width > area.width {
                // Reset since section will wrap internally
                current_x = section_width % area.width;
            } else {
                current_x += section_width;
            }
        }

        breaks
    }

    /// Extract section boundaries from items.
    /// Returns a list of (start_idx, end_idx) for each section.
    fn extract_sections(items: &[BattlefieldItem]) -> Vec<(usize, usize)> {
        let mut sections = Vec::new();
        let mut current_start: Option<usize> = None;

        for (i, item) in items.iter().enumerate() {
            match item {
                BattlefieldItem::Label { .. } => {
                    // End previous section if any
                    if let Some(start) = current_start {
                        sections.push((start, i));
                    }
                    current_start = Some(i);
                }
                BattlefieldItem::Card { .. } => {
                    // Continue current section
                }
            }
        }
        // Close final section
        if let Some(start) = current_start {
            sections.push((start, items.len()));
        }
        sections
    }

    /// Find the maximum card height that fits in the given number of rows.
    fn find_max_card_height_for_rows(
        &self,
        area: Rect,
        view: &GameStateView,
        items: &[BattlefieldItem],
        sections: &[(usize, usize)],
        target_rows: usize,
    ) -> u16 {
        // Binary search for the maximum card height that fits in target_rows
        let mut best_height = Self::MIN_CARD_HEIGHT;

        for h in Self::MIN_CARD_HEIGHT..=Self::MAX_CARD_HEIGHT {
            let w = Self::compute_width_from_height(h);
            let rows_used = self.count_rows_for_layout(area, view, items, sections, w, h, target_rows);
            if rows_used <= target_rows {
                best_height = h;
            } else {
                // Once we exceed target rows, larger cards won't fit either
                break;
            }
        }

        best_height
    }

    /// Count how many rows the layout uses with given card size.
    /// If target_rows > 1, tries to break at section boundaries when beneficial.
    #[allow(clippy::too_many_arguments)]
    fn count_rows_for_layout(
        &self,
        area: Rect,
        view: &GameStateView,
        items: &[BattlefieldItem],
        sections: &[(usize, usize)],
        card_width: u16,
        card_height: u16,
        target_rows: usize,
    ) -> usize {
        if area.width == 0 {
            return usize::MAX;
        }

        // First, try natural wrapping (no forced section breaks)
        let natural_rows = self.simulate_layout_rows(area, view, items, card_width, card_height, false);

        if target_rows == 1 || natural_rows <= target_rows {
            return natural_rows;
        }

        // For multi-row targets, try breaking at section boundaries
        // This treats sections like "words" - don't break mid-section
        if sections.len() > 1 && target_rows >= 2 {
            let section_rows = self.simulate_section_break_layout(area, view, items, sections, card_width, card_height);
            if section_rows <= target_rows {
                return section_rows;
            }
        }

        natural_rows
    }

    /// Simulate layout and count rows used (natural word-wrap, no forced breaks).
    /// Also tracks max entity height per row for accurate vertical space calculation.
    fn simulate_layout_rows(
        &self,
        area: Rect,
        view: &GameStateView,
        items: &[BattlefieldItem],
        card_width: u16,
        card_height: u16,
        _respect_force_newline: bool,
    ) -> usize {
        let mut rows = 1usize;
        let mut current_x = 0u16;
        let mut current_row_max_h = card_height;
        let mut total_height = 0u16; // Track total height needed

        for item in items {
            match item {
                BattlefieldItem::Label { .. } => {
                    // Labels don't take horizontal space in current implementation
                    // They appear in the header row above cards
                }
                BattlefieldItem::Card { entity } => {
                    // Use LAYOUT dimensions for spacing decisions
                    let (card_w, card_h) = self.get_entity_layout_dimensions(entity, view, card_width, card_height);

                    // Check if card fits on current row
                    if current_x > 0 && current_x + card_w > area.width {
                        // Finish current row
                        total_height += 1 + current_row_max_h + Self::CARD_SPACING; // header + max_h + spacing
                        rows += 1;
                        current_x = 0;
                        current_row_max_h = card_height;
                    }

                    current_row_max_h = current_row_max_h.max(card_h);
                    current_x += card_w + Self::CARD_SPACING;
                }
            }
        }

        // Add final row height
        total_height += 1 + current_row_max_h; // header + max_h (no trailing spacing)

        // Check if we have vertical space
        if total_height > area.height {
            usize::MAX // Doesn't fit
        } else {
            rows
        }
    }

    /// Simulate layout with section-aware line breaking.
    /// Tries to keep sections together, breaking only between sections.
    /// Tracks actual entity heights for accurate vertical space calculation.
    fn simulate_section_break_layout(
        &self,
        area: Rect,
        view: &GameStateView,
        items: &[BattlefieldItem],
        sections: &[(usize, usize)],
        card_width: u16,
        card_height: u16,
    ) -> usize {
        let mut rows = 1usize;
        let mut current_x = 0u16;
        let mut current_row_max_h = card_height;
        let mut total_height = 0u16;

        for (section_idx, &(start, end)) in sections.iter().enumerate() {
            // Calculate width and max height of this entire section using LAYOUT dimensions
            let mut section_width = 0u16;
            let mut section_max_h = card_height;
            for item in &items[start..end] {
                if let BattlefieldItem::Card { entity } = item {
                    let (card_w, card_h) = self.get_entity_layout_dimensions(entity, view, card_width, card_height);
                    section_width += card_w + Self::CARD_SPACING;
                    section_max_h = section_max_h.max(card_h);
                }
            }

            // Check if section fits on current row
            if current_x > 0 && current_x + section_width > area.width {
                // Break before this section - finish current row
                if section_idx > 0 {
                    total_height += 1 + current_row_max_h + Self::CARD_SPACING;
                    rows += 1;
                    current_x = 0;
                    current_row_max_h = card_height;
                }
            }

            // If section is too wide for a single row, fall back to card-by-card wrapping
            if section_width > area.width {
                for item in &items[start..end] {
                    if let BattlefieldItem::Card { entity } = item {
                        let (card_w, card_h) = self.get_entity_layout_dimensions(entity, view, card_width, card_height);
                        if current_x > 0 && current_x + card_w > area.width {
                            total_height += 1 + current_row_max_h + Self::CARD_SPACING;
                            rows += 1;
                            current_x = 0;
                            current_row_max_h = card_height;
                        }
                        current_row_max_h = current_row_max_h.max(card_h);
                        current_x += card_w + Self::CARD_SPACING;
                    }
                }
            } else {
                current_row_max_h = current_row_max_h.max(section_max_h);
                current_x += section_width;
            }
        }

        // Add final row height
        total_height += 1 + current_row_max_h;

        // Check if we have vertical space
        if total_height > area.height {
            usize::MAX
        } else {
            rows
        }
    }

    /// Render battlefield using word-wrap model with per-row headers.
    /// Each row of cards has a 1-line header above it where section labels and stack counts appear.
    /// The grid is centered horizontally, sliding left to avoid graveyard collision.
    /// Uses section_breaks to force line breaks at section boundaries where computed.
    #[allow(clippy::too_many_arguments)]
    fn render_wordwrap_battlefield(
        &mut self,
        f: &mut Frame,
        area: Rect,
        view: &GameStateView,
        items: &[BattlefieldItem],
        card_width: u16,
        card_height: u16,
        section_breaks: &[usize],       // Section indices that should start on new rows
        graveyard_bounds: Option<Rect>, // bottom-right graveyard area to avoid
    ) {
        if area.height == 0 || area.width == 0 {
            return;
        }

        // Extract sections so we can track which section we're in
        let sections = Self::extract_sections(items);
        let section_break_set: std::collections::HashSet<usize> = section_breaks.iter().copied().collect();

        // Precompute which sections can fit on a single row vs need internal wrapping
        // Use LAYOUT dimensions for width calculations
        let mut section_fits_on_row: Vec<bool> = Vec::new();
        for &(start, end) in &sections {
            let mut section_width = 0u16;
            for item in &items[start..end] {
                if let BattlefieldItem::Card { entity } = item {
                    let (card_w, _) = self.get_entity_layout_dimensions(entity, view, card_width, card_height);
                    section_width += card_w + Self::CARD_SPACING;
                }
            }
            section_fits_on_row.push(section_width <= area.width);
        }

        // First pass: compute layout positions relative to (0, 0)
        // Track max entity height per row for proper row advancement
        let mut card_positions: Vec<(u16, u16, u16, u16)> = Vec::new(); // (x, y, w, h) for each card
        let mut card_section_idx: Vec<usize> = Vec::new(); // section index for each card (for spacing)
        let mut card_label_idx: Vec<Option<usize>> = Vec::new(); // label index for each card (if it has one)
        let mut card_stack_idx: Vec<Option<usize>> = Vec::new(); // stack_count index for each card (if it's a stack)
        let mut label_positions: Vec<(u16, u16, String, Color)> = Vec::new(); // (x, y, text, color)
        let mut stack_count_positions: Vec<(u16, u16, u16, usize)> = Vec::new(); // (x, y, card_w, count) for stacks

        let mut y_offset = 0u16;
        let mut current_x = 0u16;
        let mut current_row_max_h = card_height; // Track tallest entity in current row
        let mut pending_label: Option<(String, Color)> = None;
        let mut current_section_idx = 0usize;

        for (item_idx, item) in items.iter().enumerate() {
            // Check if we're entering a new section that should break
            if let Some(&(section_start, _)) = sections.get(current_section_idx + 1) {
                if item_idx >= section_start {
                    current_section_idx += 1;
                    // Check if this section should start on new row
                    if section_break_set.contains(&current_section_idx) && current_x > 0 {
                        y_offset += 1 + current_row_max_h + Self::CARD_SPACING;
                        current_x = 0;
                        current_row_max_h = card_height;
                    }
                }
            }

            match item {
                BattlefieldItem::Label { text, color, .. } => {
                    // Store label to render above next card
                    pending_label = Some((format!("{}:", text), *color));
                }
                BattlefieldItem::Card { entity } => {
                    // Use LAYOUT dimensions for spacing and positioning
                    // This is the card's public bounding box
                    let (card_w, card_h) = self.get_entity_layout_dimensions(entity, view, card_width, card_height);

                    // Check if card fits on current row (width check)
                    // In section-aware mode, only wrap mid-section if section is too wide to fit
                    let section_can_fit = section_fits_on_row.get(current_section_idx).copied().unwrap_or(false);
                    // Wrap if: position is non-zero AND card doesn't fit AND
                    // (section is too wide for a row, OR no section breaks computed)
                    let should_wrap = current_x > 0
                        && current_x + card_w > area.width
                        && (!section_can_fit || section_breaks.is_empty());

                    if should_wrap {
                        // Wrap to next row: advance by header + max height of current row
                        y_offset += 1 + current_row_max_h + Self::CARD_SPACING;
                        current_x = 0;
                        current_row_max_h = card_height;
                    }

                    // Update max height for this row
                    current_row_max_h = current_row_max_h.max(card_h);

                    // If there's a pending label, place it above this card
                    let label_idx = if let Some((label_text, label_color)) = pending_label.take() {
                        let idx = label_positions.len();
                        label_positions.push((current_x, y_offset, label_text, label_color));
                        Some(idx)
                    } else {
                        None
                    };

                    // Check if this entity is a stack with count > 1
                    let stack_idx = if entity.count() > 1 {
                        let idx = stack_count_positions.len();
                        stack_count_positions.push((current_x, y_offset, card_w, entity.count()));
                        Some(idx)
                    } else {
                        None
                    };

                    // Card position uses LAYOUT dimensions (public bounding box)
                    let card_y = y_offset + 1;
                    // Always add card position - ratatui will handle clipping if needed.
                    // The test_wordwrap_layout_fits function should have already ensured
                    // proper card sizing, but we render regardless for robustness.
                    card_positions.push((current_x, card_y, card_w, card_h));
                    card_section_idx.push(current_section_idx);
                    card_label_idx.push(label_idx);
                    card_stack_idx.push(stack_idx);

                    current_x += card_w + Self::CARD_SPACING;
                }
            }
        }

        // Calculate bounding box of the card grid (before redistribution)
        let mut grid_width = 0u16;
        let mut grid_height = 0u16;
        for &(x, y, w, h) in &card_positions {
            grid_width = grid_width.max(x + w);
            grid_height = grid_height.max(y + h);
        }

        // Horizontal redistribution: spread cards with padding on edges and between cards
        // Gap weights: 1.0 for left edge, right edge, and same-section gaps
        //              SECTION_GAP_MULTIPLIER for gaps between different sections
        const SECTION_GAP_MULTIPLIER: f32 = 1.5; // Easily adjustable section separator weight
        const MIN_EDGE_PADDING: u16 = 1;

        let extra_space = area
            .width
            .saturating_sub(grid_width)
            .saturating_sub(MIN_EDGE_PADDING * 2);

        if extra_space > 2 && !card_positions.is_empty() {
            // Group cards by row (same y value) and track their indices
            let mut rows: Vec<Vec<usize>> = Vec::new();
            let mut current_row_y: Option<u16> = None;

            for (idx, &(_, y, _, _)) in card_positions.iter().enumerate() {
                if current_row_y != Some(y) {
                    rows.push(Vec::new());
                    current_row_y = Some(y);
                }
                if let Some(row) = rows.last_mut() {
                    row.push(idx);
                }
            }

            // For each row, calculate and apply extra spacing
            for row_indices in &rows {
                if row_indices.is_empty() {
                    continue;
                }

                // Calculate weighted gap count including edges:
                // - 1.0 for left edge padding
                // - 1.0 for right edge padding
                // - 1.0 for gaps between cards in same section
                // - SECTION_GAP_MULTIPLIER for gaps between sections
                let mut total_gap_weight: f32 = 2.0; // Left edge (1.0) + right edge (1.0)

                for i in 1..row_indices.len() {
                    let prev_section = card_section_idx[row_indices[i - 1]];
                    let curr_section = card_section_idx[row_indices[i]];
                    if curr_section != prev_section {
                        total_gap_weight += SECTION_GAP_MULTIPLIER;
                    } else {
                        total_gap_weight += 1.0;
                    }
                }

                // Calculate row width and extra space for this row
                let first_idx = row_indices[0];
                let last_idx = row_indices[row_indices.len() - 1];
                let (first_x, _, _, _) = card_positions[first_idx];
                let (last_x, _, last_w, _) = card_positions[last_idx];
                let row_width = last_x + last_w - first_x;
                let row_extra = area
                    .width
                    .saturating_sub(row_width)
                    .saturating_sub(MIN_EDGE_PADDING * 2);

                if row_extra <= 2 {
                    continue; // Not enough extra space for this row
                }

                // Calculate extra spacing per weight unit
                let extra_per_unit = f32::from(row_extra) / total_gap_weight;
                if extra_per_unit < 0.5 {
                    continue; // Not enough to make a difference
                }

                // Start with left edge padding (1.0 weight)
                let left_edge_extra = extra_per_unit.round() as u16;

                // Apply cumulative offset to ALL cards in this row (including first)
                // First card gets left edge padding, subsequent cards get cumulative
                let mut cumulative_extra = left_edge_extra;

                // Update first card position with left edge padding
                let first_card_idx = row_indices[0];
                card_positions[first_card_idx].0 += cumulative_extra;
                if let Some(label_idx) = card_label_idx[first_card_idx] {
                    label_positions[label_idx].0 += cumulative_extra;
                }
                if let Some(stack_idx) = card_stack_idx[first_card_idx] {
                    stack_count_positions[stack_idx].0 += cumulative_extra;
                }

                // Apply spacing to remaining cards
                for i in 1..row_indices.len() {
                    let prev_section = card_section_idx[row_indices[i - 1]];
                    let curr_section = card_section_idx[row_indices[i]];
                    let gap_weight = if curr_section != prev_section {
                        SECTION_GAP_MULTIPLIER
                    } else {
                        1.0
                    };
                    cumulative_extra += (extra_per_unit * gap_weight).round() as u16;

                    // Update card position
                    let card_idx = row_indices[i];
                    card_positions[card_idx].0 += cumulative_extra;

                    // Update associated label position if any
                    if let Some(label_idx) = card_label_idx[card_idx] {
                        label_positions[label_idx].0 += cumulative_extra;
                    }

                    // Update associated stack_count position if any
                    if let Some(stack_idx) = card_stack_idx[card_idx] {
                        stack_count_positions[stack_idx].0 += cumulative_extra;
                    }
                }
            }

            // Recalculate grid_width after redistribution
            grid_width = 0;
            for &(x, _, w, _) in &card_positions {
                grid_width = grid_width.max(x + w);
            }
        }

        // Include header row in height
        if !label_positions.is_empty() || !stack_count_positions.is_empty() {
            // The first row has y_offset=0, so card starts at y=1
            // We need to account for the header line at the top
            if grid_height > 0 {
                // grid_height already includes card_y which starts at 1
                // but we want the total from y=0
            }
        }

        // Compute centering offset, adjusting for graveyard collision
        let x_offset = if grid_width < area.width {
            let ideal_center = (area.width - grid_width) / 2;

            // Check for graveyard collision with last cards in each row
            if let Some(gy_bounds) = graveyard_bounds {
                // Find right-most cards that might collide with graveyard
                // Last card in last row and last card in second-to-last row
                let gy_left = gy_bounds.x.saturating_sub(area.x);
                let gy_top = gy_bounds.y.saturating_sub(area.y);

                // Find cards that would collide with graveyard after centering
                let mut max_safe_offset = ideal_center;

                for &(x, y, w, h) in &card_positions {
                    let card_right = x + w + ideal_center;
                    let card_bottom = y + h;

                    // Check if this card would overlap graveyard
                    if card_right > gy_left && card_bottom > gy_top {
                        // This card collides - compute how much we need to slide left
                        let needed_slide = card_right.saturating_sub(gy_left);
                        max_safe_offset = max_safe_offset.saturating_sub(needed_slide);
                    }
                }

                max_safe_offset
            } else {
                ideal_center
            }
        } else {
            0
        };

        // Render labels with offset
        for (x, y, text, color) in &label_positions {
            if *y < area.height {
                let label_area = Rect {
                    x: area.x + x + x_offset,
                    y: area.y + y,
                    width: text.len() as u16,
                    height: 1,
                };
                let styled_label = Span::styled(text.clone(), Style::default().fg(*color).add_modifier(Modifier::BOLD));
                f.render_widget(Paragraph::new(Line::from(styled_label)), label_area);
            }
        }

        // Render stack counts with offset
        for (x, y, card_w, count) in &stack_count_positions {
            if *y < area.height {
                let count_text = format!("{}X", count);
                let text_len = count_text.len() as u16;
                let count_x = x + card_w.saturating_sub(text_len);
                let count_area = Rect {
                    x: area.x + count_x + x_offset,
                    y: area.y + y,
                    width: text_len,
                    height: 1,
                };
                let styled_count = Span::styled(
                    count_text,
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                );
                f.render_widget(Paragraph::new(Line::from(styled_count)), count_area);
            }
        }

        // Render cards with offset, clipping to area bounds
        let mut card_idx = 0;
        for item in items {
            if let BattlefieldItem::Card { entity } = item {
                if card_idx < card_positions.len() {
                    // card_positions contains LAYOUT dimensions (MAX bounds)
                    let (x, y, layout_w, layout_h) = card_positions[card_idx];
                    let entity_x = area.x + x + x_offset;
                    let entity_y = area.y + y;

                    // Get MIN dimensions for text rendering
                    let (min_w, min_h) = Self::get_entity_dimensions(entity, view, card_width, card_height);

                    // Clip entity to not exceed area bounds
                    let max_w = area.x.saturating_add(area.width).saturating_sub(entity_x);
                    let max_h = area.y.saturating_add(area.height).saturating_sub(entity_y);

                    // Only render if entity has some visible area
                    if max_w > 0 && max_h > 0 {
                        // render_area uses MIN dimensions for text rendering
                        let render_area = Rect {
                            x: entity_x,
                            y: entity_y,
                            width: min_w.min(max_w),
                            height: min_h.min(max_h),
                        };
                        // layout_dimensions uses MAX for hit-testing (clipped to visible area)
                        let layout_dims = Some((layout_w.min(max_w), layout_h.min(max_h)));
                        self.render_entity(f, render_area, layout_dims, view, entity);
                    }
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
            // Use LAYOUT dimensions for spacing (card's public bounding box)
            let (card_w, card_h) = self.get_entity_layout_dimensions(entity, view, card_width, card_height);

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
            // (card_w, card_h) are LAYOUT dimensions (MAX bounds)
            for (entity, layout_w, layout_h) in row {
                // Get MIN dimensions for text rendering
                let (min_w, min_h) = Self::get_entity_dimensions(entity, view, card_width, card_height);

                // render_area uses MIN dimensions for text rendering
                let render_area = Rect {
                    x: current_x,
                    y: current_y,
                    width: min_w,
                    height: min_h,
                };
                // layout_dimensions uses MAX for hit-testing
                let layout_dims = Some((*layout_w, *layout_h));
                self.render_entity(f, render_area, layout_dims, view, entity);

                current_x += layout_w + Self::CARD_SPACING;
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

        // Group cards: planeswalkers, creatures, enchantments, artifacts, lands
        let (planeswalkers, creatures, enchantments, artifacts, lands): (Vec<_>, Vec<_>, Vec<_>, Vec<_>, Vec<_>) =
            player_cards.iter().fold(
                (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new()),
                |(mut planeswalkers, mut creatures, mut enchantments, mut artifacts, mut lands), &card_id| {
                    if let Some(card) = view.get_card(card_id) {
                        // Check types in priority order (a card can have multiple types)
                        if card.is_planeswalker() {
                            planeswalkers.push(card_id);
                        } else if card.is_creature() {
                            creatures.push(card_id);
                        } else if card.is_enchantment() {
                            enchantments.push(card_id);
                        } else if card.is_artifact() {
                            artifacts.push(card_id);
                        } else if card.is_land() {
                            lands.push(card_id);
                        }
                    }
                    (planeswalkers, creatures, enchantments, artifacts, lands)
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

        // Create block with optional background color for GUI mode
        let block_style = if let Some(bg_color) = self.render_config.pane_bg_color {
            Style::default().bg(bg_color)
        } else {
            Style::default()
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .title(title_line)
            .border_style(Style::default().fg(border_color))
            .style(block_style);
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

        // Build ordered sections for Z-order flow (upper-left to lower-right with wraps)
        // Player: Planeswalkers → Creatures → Enchantments → Artifacts → Lands
        // Opponent: reverse (Lands → Artifacts → Enchantments → Creatures → Planeswalkers)
        // Section format: (cards, label, color, force_newline_before)
        let sections: Vec<(Vec<CardId>, &str, Color, bool)> = if is_player_bf {
            // Player battlefield: important things first, lands last
            let mut secs = Vec::new();
            if !planeswalkers.is_empty() {
                secs.push((planeswalkers, "PWs", Color::LightYellow, false));
            }
            if !creatures.is_empty() {
                secs.push((creatures, "Creatures", Color::Red, false));
            }
            if !enchantments.is_empty() {
                secs.push((enchantments, "Enchants", Color::Magenta, false));
            }
            if !artifacts.is_empty() {
                secs.push((artifacts, "Artifacts", Color::Cyan, false));
            }
            if !lands.is_empty() {
                // Try to force a newline before lands (will be evaluated during rendering)
                secs.push((lands, "Lands", Color::Green, true));
            }
            secs
        } else {
            // Opponent battlefield: reversed order (lands first, PWs last)
            let mut secs = Vec::new();
            if !lands.is_empty() {
                secs.push((lands, "Lands", Color::Green, false));
            }
            if !artifacts.is_empty() {
                secs.push((artifacts, "Artifacts", Color::Cyan, !secs.is_empty()));
            }
            if !enchantments.is_empty() {
                secs.push((enchantments, "Enchants", Color::Magenta, false));
            }
            if !creatures.is_empty() {
                secs.push((creatures, "Creatures", Color::Red, false));
            }
            if !planeswalkers.is_empty() {
                secs.push((planeswalkers, "PWs", Color::LightYellow, false));
            }
            secs
        };

        // Compute graveyard bounds first so battlefield can avoid collision
        let graveyard_bounds = Self::compute_graveyard_bounds(inner_area, view, owner_id);

        // Render battlefield with inline section labels, centered and avoiding graveyard
        self.render_battlefield_inline(f, inner_area, view, &sections, graveyard_bounds);

        // Render graveyard overlay in bottom-right corner
        self.render_graveyard_overlay(f, inner_area, view, owner_id);

        // Render command zone overlay in bottom-left corner (Commander format)
        self.render_command_zone_overlay(f, inner_area, view, owner_id);
    }

    /// Compute the bounding box for graveyard overlay (without rendering)
    /// Returns None if graveyard is empty or doesn't fit
    fn compute_graveyard_bounds(area: Rect, view: &GameStateView, owner_id: PlayerId) -> Option<Rect> {
        let graveyard = view.player_graveyard(owner_id);
        if graveyard.is_empty() {
            return None;
        }

        // Calculate required width (longest name or header)
        let header = "Graveyard:";
        let max_name_len = graveyard
            .iter()
            .filter_map(|&card_id| view.card_name(card_id))
            .map(|n| n.len())
            .max()
            .unwrap_or(0);
        let content_width = max_name_len.max(header.len()) as u16;

        // Calculate required height: header + cards
        let box_height = (1 + graveyard.len()) as u16;

        // Check if it fits
        if area.width < content_width || area.height < box_height {
            return None;
        }

        let x_start = area.x + area.width - content_width;
        let y_start = area.y + area.height - box_height;

        Some(Rect {
            x: x_start,
            y: y_start,
            width: content_width,
            height: box_height,
        })
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
                layout_area_px: None,
            });
        }
    }

    /// Render command zone as a simple text overlay in the bottom-left corner of the battlefield
    /// Mirrors the graveyard overlay style but positioned on the opposite side
    fn render_command_zone_overlay(&mut self, f: &mut Frame, area: Rect, view: &GameStateView, owner_id: PlayerId) {
        let command_zone = view.player_command_zone(owner_id);
        if command_zone.is_empty() {
            return;
        }

        let card_entries: Vec<(CardId, String)> = command_zone
            .iter()
            .map(|&card_id| {
                (
                    card_id,
                    view.card_name(card_id).unwrap_or_else(|| "Unknown".to_string()),
                )
            })
            .collect();

        let header = "Command:";
        let max_name_len = card_entries.iter().map(|(_, n)| n.len()).max().unwrap_or(0);
        let content_width = max_name_len.max(header.len()) as u16;
        let box_height = (1 + card_entries.len()) as u16;

        if area.width < content_width || area.height < box_height {
            return;
        }

        // Position in bottom-left corner (opposite of graveyard)
        let x_start = area.x;
        let y_start = area.y + area.height - box_height;

        let style = Style::default().fg(Color::LightMagenta);

        let header_area = Rect {
            x: x_start,
            y: y_start,
            width: content_width,
            height: 1,
        };
        f.render_widget(
            ratatui::widgets::Paragraph::new(header).style(style.add_modifier(ratatui::style::Modifier::BOLD)),
            header_area,
        );

        for (i, (_card_id, name)) in card_entries.iter().enumerate() {
            let card_area = Rect {
                x: x_start,
                y: y_start + 1 + i as u16,
                width: content_width,
                height: 1,
            };
            f.render_widget(ratatui::widgets::Paragraph::new(name.as_str()).style(style), card_area);
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

                // Card name with ID (matches log format: "Island (35)")
                lines.push(Line::from(Span::styled(
                    format!("{} ({})", card.name, card_id.as_u32()),
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
                    let power = view
                        .get_effective_power(card_id)
                        .unwrap_or_else(|| i32::from(card.current_power()));
                    let toughness = view
                        .get_effective_toughness(card_id)
                        .unwrap_or_else(|| i32::from(card.current_toughness()));

                    // Get base/printed P/T to show when modified
                    let base_power = i32::from(card.base_power().unwrap_or(0));
                    let base_toughness = i32::from(card.base_toughness().unwrap_or(0));

                    let pt_display = if power != base_power || toughness != base_toughness {
                        format!("{}/{} (base {}/{})", power, toughness, base_power, base_toughness)
                    } else {
                        format!("{}/{}", power, toughness)
                    };

                    lines.push(Line::from(vec![
                        Span::raw("P/T: "),
                        Span::styled(pt_display, Style::default().fg(Color::Green)),
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
                    // Split oracle text on newlines - each line becomes a separate Line
                    for oracle_line in card.text.split('\n') {
                        lines.push(Line::from(Span::styled(
                            oracle_line.to_string(),
                            Style::default().fg(Color::White),
                        )));
                    }
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

        // Sort hand in display order (shared with event handler for consistent indexing)
        let sorted_hand = Self::get_sorted_hand(view);

        // Track entity positions for each hand card (for mouse click detection)
        // Each list item is 1 row tall, positioned starting from inner_area.y
        for (i, &card_id) in sorted_hand.iter().enumerate() {
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
                    layout_area_px: None,
                });
            }
        }

        let items: Vec<ListItem> = sorted_hand
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

    /// Draw player info bar (life, zones, etc.)
    pub fn draw_player_info(&self, f: &mut Frame, area: Rect, view: &GameStateView, player_id: PlayerId) {
        use ratatui::text::Text;

        let life = view.player_life(player_id);
        let hand_size = view.player_hand(player_id).len();
        let graveyard_size = view.player_graveyard(player_id).len();
        let library_size = view.player_library(player_id).len();

        // Get player name and determine if this is P1 or P2
        let player_name = view.get_player_name_by_id(player_id);
        let player_num = if player_id == view.player_id() { "P1" } else { "P2" };

        // Format player label: avoid redundant "P1 (P1)" - just show "P1" if name matches
        let player_label = if player_name == player_num {
            player_name
        } else {
            format!("{} ({})", player_name, player_num)
        };

        // Left side: player stats (battlefield title already shows ownership)
        let stats_text = format!(
            "{} | {} life | Hand: {} | GY: {} | Lib: {}",
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

        // Format: Turn: <global> (<player>) to match log format
        let mut phase_spans = vec![Span::raw(format!("Turn: {} ({}) | ", turn_number, turn_display))];

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
            turn_number, turn_display
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
                // First, clear the area to erase any underlying card borders showing through
                f.render_widget(Clear, card_area);

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
    ///
    /// Parameters:
    /// - `render_area`: MIN bounds (cells) where TUI text is actually rendered
    /// - `layout_dimensions`: Optional (layout_w, layout_h) in cells - MAX bounds for hit-testing/images
    ///   If None, uses render_area dimensions (CLI mode behavior)
    pub fn render_entity(
        &mut self,
        f: &mut Frame,
        render_area: Rect,
        layout_dimensions: Option<(u16, u16)>,
        view: &GameStateView,
        entity: &Entity,
    ) {
        use ratatui::text::Text;

        // Calculate layout area (MAX bounds) for hit-testing and image positioning
        // In GUI mode, this is larger than render_area to match MTG aspect ratio
        let (layout_w, layout_h) = layout_dimensions.unwrap_or((render_area.width, render_area.height));
        let layout_area = Rect {
            x: render_area.x,
            y: render_area.y,
            width: layout_w,
            height: layout_h,
        };

        // Calculate pixel-based layout area if in GUI mode
        let layout_area_px = if self.render_config.gui_mode {
            Some(LayoutAreaPx {
                x_px: f32::from(render_area.x) * self.render_config.cell_width_px,
                y_px: f32::from(render_area.y) * self.render_config.cell_height_px,
                width_px: f32::from(layout_w) * self.render_config.cell_width_px,
                height_px: f32::from(layout_h) * self.render_config.cell_height_px,
            })
        } else {
            None
        };

        // Track entity position for mouse hit testing
        // Uses MAX bounds (layout_area) as the public bounding box
        self.state.entity_positions.push(EntityPosition {
            entity: entity.clone(),
            area: layout_area, // MAX bounds - the public bounding box
            layout_area_px,
        });

        // Dispatch to visual stack renderer if applicable
        if matches!(entity, Entity::VisualStack { .. }) {
            self.render_visual_stack(f, render_area, view, entity);
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
        // Use render_area (MIN bounds) for text content calculation
        let content_width = render_area.width.saturating_sub(2) as usize;
        let content_height = render_area.height.saturating_sub(2);

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
                title_spans.push(Span::styled(card_name_part, title_style));
            } else {
                title_spans.push(Span::styled(name, title_style));
            }
            title_spans.push(Span::raw(" ".repeat(padding)));
            title_spans.push(Span::raw(cost_str));
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
                name
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
                    card_name_part
                };
                lines.push(Line::from(vec![
                    Span::styled(mult.clone(), Style::default().fg(Color::Cyan)),
                    Span::styled(card_name_truncated, title_style),
                ]));
            } else {
                lines.push(Line::from(Span::styled(display_name, title_style)));
            }
            lines.push(Line::from(cost_str));
        } else if !cost_str.is_empty() && !name_and_cost_fit && name_fits_alone && have_vertical_space {
            // Name fits, cost doesn't fit on same line, use two lines
            if let Some(mult) = multiplier_prefix.as_ref() {
                lines.push(Line::from(vec![
                    Span::styled(mult.clone(), Style::default().fg(Color::Cyan)),
                    Span::styled(card_name_part, title_style),
                ]));
            } else {
                lines.push(Line::from(Span::styled(name, title_style)));
            }
            lines.push(Line::from(cost_str));
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
                name
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
            if let Some(c) = card.as_ref() {
                // Get effective P/T (includes all continuous effects like anthems, equipment, auras)
                let current_power = view
                    .get_effective_power(card_id)
                    .unwrap_or_else(|| i32::from(c.current_power()));
                let current_toughness = view
                    .get_effective_toughness(card_id)
                    .unwrap_or_else(|| i32::from(c.current_toughness()));

                // Get base/printed P/T
                let base_power = i32::from(c.base_power().unwrap_or(0));
                let base_toughness = i32::from(c.base_toughness().unwrap_or(0));

                // Show both current and base if they differ
                if current_power != base_power || current_toughness != base_toughness {
                    format!(
                        "{}/{} ({}/{})",
                        current_power, current_toughness, base_power, base_toughness
                    )
                } else {
                    format!("{}/{}", current_power, current_toughness)
                }
            } else {
                String::new()
            }
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
        f.render_widget(paragraph, render_area);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{Card, CardType};
    use crate::game::state::GameState;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use smallvec::smallvec;

    /// Helper to create a land card and add it to the game
    fn create_land(game: &mut GameState, name: &str, owner: PlayerId) -> CardId {
        let card_id = game.next_entity_id();
        let mut card = Card::new(card_id, name.to_string(), owner);
        card.add_type(CardType::Land);
        game.cards.insert(card_id, card);
        game.battlefield.add(card_id);
        card_id
    }

    /// Test for visual stack clipping bug
    ///
    /// When using --visual-stacks mode, a 3X stack of lands grows taller than a 2X stack
    /// due to diagonal offsets. If the 3X stack is on the bottom row and the height
    /// exceeds the available area, the card gets silently dropped while the "3X" label
    /// still renders, making it appear the stack disappeared.
    ///
    /// This test reproduces the bug by:
    /// 1. Creating a battlefield with lands that form visual stacks
    /// 2. Using a constrained area where the 3X stack clips
    /// 3. Verifying that both stack count labels AND actual cards render
    #[test]
    fn test_visual_stack_does_not_clip_on_bottom_row() {
        // Create a game state with lands on battlefield
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players.first().unwrap().id;

        // Create lands that will form visual stacks:
        // - 2 Forests -> 2X stack (height = base + 1 offset = 7 + 1 = 8)
        // - 3 Plains -> 3X stack (height = base + 2 offsets = 7 + 2 = 9)
        let forest1 = create_land(&mut game, "Forest", p1_id);
        let forest2 = create_land(&mut game, "Forest", p1_id);
        let plains1 = create_land(&mut game, "Plains", p1_id);
        let plains2 = create_land(&mut game, "Plains", p1_id);
        let plains3 = create_land(&mut game, "Plains", p1_id);

        // Create the view
        let view = GameStateView::new(&game, p1_id);

        // Create renderer with visual_stacks = true
        let mut renderer = FancyTuiRenderer::new(p1_id, true);

        // Set up a constrained area that will trigger the bug:
        // - Width is sufficient for both stacks side by side
        // - Height is just barely enough for a 2X stack but not a 3X stack
        //
        // With DEFAULT_CARD_HEIGHT = 7:
        // - Header row: 1 line
        // - 2X stack: 7 + 1 = 8 lines
        // - 3X stack: 7 + 2 = 9 lines
        //
        // y_offset starts at 0
        // After label, y_offset still 0 (label stored, not advanced)
        // card_y = y_offset + 1 = 1
        // For 2X: card_h = 8, check: card_y + card_h = 1 + 8 = 9 <= area.height
        // For 3X: card_h = 9, check: card_y + card_h = 1 + 9 = 10 <= area.height
        //
        // So if area.height = 9:
        // - 2X: 9 <= 9 -> renders
        // - 3X: 10 <= 9 -> false, DOES NOT RENDER (old bug!)
        //
        // With the fix, the card sizing algorithm now properly accounts for
        // entity heights, so it would reduce card size if needed. For this test,
        // we use height=10 which fits the 3X stack exactly (1 header + 9 card).
        //
        // Width: 2X stack = 11 (10+1 offset), 3X stack = 12 (10+2 offsets)
        // With spacing of 1 between them and label "Lands:" (6 chars)
        // Total min width = 12 + 1 + 12 + 1 = 26 (plus some margin)
        let area = Rect {
            x: 0,
            y: 0,
            width: 50,  // Wide enough for both stacks + centering
            height: 10, // Just enough for 3X stack (1 header + 9 entity height)
        };

        // Create battlefield items with visual stacks
        let items = vec![
            BattlefieldItem::Label {
                text: "Lands".to_string(),
                color: Color::Green,
                force_newline_before: false,
                entity_count: 2,
            },
            BattlefieldItem::Card {
                entity: Entity::VisualStack {
                    card_ids: smallvec![forest1, forest2],
                    card_name: "Forest".to_string(),
                    tapped_count: 0,
                },
            },
            BattlefieldItem::Card {
                entity: Entity::VisualStack {
                    card_ids: smallvec![plains1, plains2, plains3],
                    card_name: "Plains".to_string(),
                    tapped_count: 0,
                },
            },
        ];

        // Create test terminal with enough space
        let backend = TestBackend::new(50, 10);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|f| {
                renderer.render_wordwrap_battlefield(f, area, &view, &items, 10, 7, &[], None);
            })
            .unwrap();

        // Get the buffer content for inspection
        let buffer = terminal.backend().buffer();

        // Convert buffer to string for inspection
        let mut output = String::new();
        for y in 0..area.height {
            for x in 0..area.width {
                let cell = buffer[(x, y)].symbol();
                output.push_str(cell);
            }
            output.push('\n');
        }

        println!("Rendered output:\n{}", output);

        // Check that "2X" label is present
        assert!(output.contains("2X"), "2X stack label should be visible in output");

        // Check that "3X" label is present
        assert!(output.contains("3X"), "3X stack label should be visible in output");

        // THE BUG: The 3X label renders but the card does NOT render!
        // We need to verify the card box is present, not just the label.

        // Look for "Plains" text in the output - this would be inside the card
        // If the card is clipped, "Plains" won't appear, only "3X" will.
        let has_plains = output.contains("Plains");

        // This assertion will FAIL with the bug, demonstrating the issue
        assert!(
            has_plains,
            "Plains card should render (not just the 3X label). Bug: visual stack clipped on bottom row!\n\
             Output:\n{}",
            output
        );
    }

    /// Test that the card size calculation accounts for visual stack height
    #[test]
    fn test_entity_dimensions_for_visual_stacks() {
        // Create a game state with lands on battlefield
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players.first().unwrap().id;

        // Create 3 plains (will form 3X visual stack)
        let plains1 = create_land(&mut game, "Plains", p1_id);
        let plains2 = create_land(&mut game, "Plains", p1_id);
        let plains3 = create_land(&mut game, "Plains", p1_id);

        let view = GameStateView::new(&game, p1_id);

        // Test entity dimension calculation
        let entity = Entity::VisualStack {
            card_ids: smallvec![plains1, plains2, plains3],
            card_name: "Plains".to_string(),
            tapped_count: 0,
        };

        // With base height 7, a 3-card stack should have height 7 + 2 = 9
        // (stack_depth - 1) * DIAGONAL_OFFSET = (3 - 1) * 1 = 2
        let (width, height) = FancyTuiRenderer::get_entity_dimensions(&entity, &view, 10, 7);

        assert_eq!(
            height, 9,
            "3X visual stack should have height = base + (stack_depth - 1) * DIAGONAL_OFFSET"
        );
        assert_eq!(
            width, 12,
            "3X visual stack should have width = base + (stack_depth - 1) * DIAGONAL_OFFSET"
        );
    }

    /// Test section-aware layout keeps sections together when they fit
    #[test]
    fn test_section_aware_layout_keeps_sections_together() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players.first().unwrap().id;

        // Create two sections: 2 Forests and 2 Plains
        let forest1 = create_land(&mut game, "Forest", p1_id);
        let forest2 = create_land(&mut game, "Forest", p1_id);
        let plains1 = create_land(&mut game, "Plains", p1_id);
        let plains2 = create_land(&mut game, "Plains", p1_id);

        let view = GameStateView::new(&game, p1_id);
        let renderer = FancyTuiRenderer::new(p1_id, true);

        // Create battlefield items with two sections
        let items = vec![
            BattlefieldItem::Label {
                text: "Forest".to_string(),
                color: Color::Green,
                force_newline_before: false,
                entity_count: 1,
            },
            BattlefieldItem::Card {
                entity: Entity::VisualStack {
                    card_ids: smallvec![forest1, forest2],
                    card_name: "Forest".to_string(),
                    tapped_count: 0,
                },
            },
            BattlefieldItem::Label {
                text: "Plains".to_string(),
                color: Color::White,
                force_newline_before: false,
                entity_count: 1,
            },
            BattlefieldItem::Card {
                entity: Entity::VisualStack {
                    card_ids: smallvec![plains1, plains2],
                    card_name: "Plains".to_string(),
                    tapped_count: 0,
                },
            },
        ];

        // Area that's wide enough for both sections on one row
        let wide_area = Rect {
            x: 0,
            y: 0,
            width: 50,
            height: 12,
        };

        // Calculate card size and section breaks
        let (card_width, _card_height, section_breaks) =
            renderer.calculate_wordwrap_card_size(wide_area, &view, &items);

        // With enough width, no section breaks needed
        assert!(
            section_breaks.is_empty(),
            "With wide area, sections should fit on one row: got breaks {:?}",
            section_breaks
        );
        assert!(
            card_width >= FancyTuiRenderer::MIN_CARD_WIDTH,
            "Card width should be at least minimum: {} >= {}",
            card_width,
            FancyTuiRenderer::MIN_CARD_WIDTH
        );

        // Now test with narrow area that requires section breaks
        let narrow_area = Rect {
            x: 0,
            y: 0,
            width: 20, // Too narrow for both sections
            height: 20,
        };

        let (_, _, section_breaks_narrow) = renderer.calculate_wordwrap_card_size(narrow_area, &view, &items);

        // With narrow width, we should get section breaks
        // (or the sections might wrap individually if too wide)
        println!(
            "Narrow area section breaks: {:?} (card_width: {})",
            section_breaks_narrow, card_width
        );
    }

    /// Test that entity clipping prevents overflow beyond area bounds
    #[test]
    fn test_entity_clipping_prevents_overflow() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players.first().unwrap().id;

        // Create a 3-card stack (height = base + 2)
        let plains1 = create_land(&mut game, "Plains", p1_id);
        let plains2 = create_land(&mut game, "Plains", p1_id);
        let plains3 = create_land(&mut game, "Plains", p1_id);

        let view = GameStateView::new(&game, p1_id);
        let mut renderer = FancyTuiRenderer::new(p1_id, true);

        let items = vec![
            BattlefieldItem::Label {
                text: "Lands".to_string(),
                color: Color::Green,
                force_newline_before: false,
                entity_count: 1,
            },
            BattlefieldItem::Card {
                entity: Entity::VisualStack {
                    card_ids: smallvec![plains1, plains2, plains3],
                    card_name: "Plains".to_string(),
                    tapped_count: 0,
                },
            },
        ];

        // Very small area that can't fit the 3X stack (needs height = 1 header + 9 entity = 10)
        // Area height 6 should cause clipping
        let small_area = Rect {
            x: 0,
            y: 0,
            width: 30,
            height: 6,
        };

        let backend = TestBackend::new(30, 6);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|f| {
                renderer.render_wordwrap_battlefield(f, small_area, &view, &items, 10, 7, &[], None);
            })
            .unwrap();

        // Get buffer content
        let buffer = terminal.backend().buffer();

        // Verify nothing rendered outside the area bounds
        // The buffer should only have content in the 30x6 area
        let mut output = String::new();
        for y in 0..small_area.height {
            for x in 0..small_area.width {
                let cell = buffer[(x, y)].symbol();
                output.push_str(cell);
            }
            output.push('\n');
        }

        println!("Clipped output:\n{}", output);

        // Verify the area was rendered (should have at least the header)
        assert!(
            output.contains("Lands") || output.contains("3X"),
            "Header or stack count should be visible even in small area"
        );
    }

    /// Test extract_sections correctly identifies section boundaries
    #[test]
    fn test_extract_sections() {
        let items = vec![
            BattlefieldItem::Label {
                text: "Section1".to_string(),
                color: Color::Red,
                force_newline_before: false,
                entity_count: 2,
            },
            BattlefieldItem::Card {
                entity: Entity::SingleCard {
                    card_id: CardId::new(1),
                },
            },
            BattlefieldItem::Card {
                entity: Entity::SingleCard {
                    card_id: CardId::new(2),
                },
            },
            BattlefieldItem::Label {
                text: "Section2".to_string(),
                color: Color::Blue,
                force_newline_before: false,
                entity_count: 1,
            },
            BattlefieldItem::Card {
                entity: Entity::SingleCard {
                    card_id: CardId::new(3),
                },
            },
        ];

        let sections = FancyTuiRenderer::extract_sections(&items);

        assert_eq!(sections.len(), 2, "Should have 2 sections");
        assert_eq!(sections[0], (0, 3), "Section 1: items 0-2 (label + 2 cards)");
        assert_eq!(sections[1], (3, 5), "Section 2: items 3-4 (label + 1 card)");
    }

    /// Test that simulate_layout_rows correctly tracks entity heights
    #[test]
    fn test_simulate_layout_rows_tracks_entity_heights() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players.first().unwrap().id;

        // Create stacks of different sizes
        let f1 = create_land(&mut game, "Forest", p1_id);
        let f2 = create_land(&mut game, "Forest", p1_id);
        let f3 = create_land(&mut game, "Forest", p1_id);
        let p1 = create_land(&mut game, "Plains", p1_id);

        let view = GameStateView::new(&game, p1_id);
        let renderer = FancyTuiRenderer::new(p1_id, true);

        // 3X Forest (height = 7 + 2 = 9) and 1X Plains (height = 7)
        let items = vec![
            BattlefieldItem::Label {
                text: "Lands".to_string(),
                color: Color::Green,
                force_newline_before: false,
                entity_count: 2,
            },
            BattlefieldItem::Card {
                entity: Entity::VisualStack {
                    card_ids: smallvec![f1, f2, f3],
                    card_name: "Forest".to_string(),
                    tapped_count: 0,
                },
            },
            BattlefieldItem::Card {
                entity: Entity::SingleCard { card_id: p1 },
            },
        ];

        // Area that's wide enough for one row but we need to verify height tracking
        // With base card_height=7, 3X stack needs 9 total
        // One row needs: 1 (header) + 9 (max entity height) = 10
        let area = Rect {
            x: 0,
            y: 0,
            width: 50,
            height: 10,
        };

        let rows = renderer.simulate_layout_rows(area, &view, &items, 10, 7, false);
        assert_eq!(rows, 1, "Should fit in 1 row with height=10");

        // Now with height=9, it should NOT fit (need 10)
        let small_area = Rect {
            x: 0,
            y: 0,
            width: 50,
            height: 9,
        };

        let rows_small = renderer.simulate_layout_rows(small_area, &view, &items, 10, 7, false);
        assert_eq!(rows_small, usize::MAX, "Should not fit in height=9");
    }
}
