//! Shared layout engine for MTG Forge UI
//!
//! This module provides backend-neutral layout computation used by both
//! the native TUI (crossterm/ratatui) and web (RatZilla/HTML) backends.
//!
//! ## Architecture
//!
//! Layout is computed in two phases:
//! 1. **Pane layout**: Subdivides the viewport into pane rects (3-column grid)
//! 2. **Card layout**: Places cards within pane inner areas (word-wrap flow)
//!
//! The [`BackendMetrics`] trait provides backend-specific measurements
//! (cell size, inner area after decorators) so the same layout logic works
//! across backends with different cell sizes and border styles.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use std::collections::HashMap;

/// Identifies each pane in the game UI.
///
/// Used as keys into the layout result to look up pane positions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PaneId {
    /// Game log / info pane (left column, top)
    Log,
    /// Actions / prompt pane (left column, bottom)
    Actions,
    /// Opponent info bar (middle column, top)
    OpponentInfo,
    /// Opponent battlefield (middle column, upper)
    OpponentBattlefield,
    /// Your battlefield (middle column, lower)
    YourBattlefield,
    /// Your info bar (middle column, bottom)
    YourInfo,
    /// Card details pane (right column, top)
    CardDetails,
    /// Hand pane (right column, bottom)
    Hand,
}

/// Backend-specific metrics for layout computation.
///
/// Backends with different cell sizes and border styles implement this
/// so the shared layout engine can compute positions correctly.
pub trait BackendMetrics {
    /// Cell dimensions in pixels `(width_px, height_px)`.
    ///
    /// Used for aspect ratio calculations and pixel-space positioning.
    /// Terminal cells are typically ~10x20 px. HTML backends may differ.
    fn cell_size_px(&self) -> (f32, f32);

    /// Compute the usable inner area after borders/decorators.
    ///
    /// Shared layout provides the outer rect; the backend reports how
    /// much space is left after borders, padding, and decorators.
    fn inner_area(&self, outer: Rect, has_border: bool) -> Rect;
}

/// Default backend metrics for ratatui (1-char borders).
pub struct RatatuiMetrics {
    pub cell_width_px: f32,
    pub cell_height_px: f32,
}

impl RatatuiMetrics {
    pub fn new(cell_width_px: f32, cell_height_px: f32) -> Self {
        Self {
            cell_width_px,
            cell_height_px,
        }
    }

    /// CLI default: 10x20 pixel cells
    pub fn cli() -> Self {
        Self::new(10.0, 20.0)
    }
}

impl BackendMetrics for RatatuiMetrics {
    fn cell_size_px(&self) -> (f32, f32) {
        (self.cell_width_px, self.cell_height_px)
    }

    fn inner_area(&self, outer: Rect, has_border: bool) -> Rect {
        if has_border {
            // ratatui Block with Borders::ALL removes 1 cell on each side
            Rect {
                x: outer.x + 1,
                y: outer.y + 1,
                width: outer.width.saturating_sub(2),
                height: outer.height.saturating_sub(2),
            }
        } else {
            outer
        }
    }
}

/// Configuration for pane layout computation.
pub struct PaneLayoutConfig {
    // Column width percentages
    pub left_column_pct: u16,
    pub middle_column_pct: u16,
    pub right_column_pct: u16,

    // Boosted left column (used when all panes meet minimums)
    pub boosted_left_column_pct: u16,

    // Minimum pane widths (in columns/cells)
    pub min_width_log: u16,
    pub min_width_actions: u16,
    pub min_width_battlefield: u16,
    pub min_width_card_details: u16,
    pub min_width_hand: u16,
}

impl Default for PaneLayoutConfig {
    fn default() -> Self {
        Self {
            left_column_pct: 25,
            middle_column_pct: 50,
            right_column_pct: 25,
            boosted_left_column_pct: 30,
            min_width_log: 40,
            min_width_actions: 40,
            min_width_battlefield: 60,
            min_width_card_details: 30,
            min_width_hand: 30,
        }
    }
}

/// Result of pane layout computation.
///
/// Contains the outer rect for each pane. Backends use [`BackendMetrics::inner_area`]
/// to get the usable inner space for content.
pub struct PaneLayout {
    panes: HashMap<PaneId, Rect>,
}

impl PaneLayout {
    /// Get the outer rect for a pane.
    pub fn get(&self, pane: PaneId) -> Option<Rect> {
        self.panes.get(&pane).copied()
    }

    /// Iterate over all pane rects.
    pub fn iter(&self) -> impl Iterator<Item = (&PaneId, &Rect)> {
        self.panes.iter()
    }
}

/// Compute pane layout for the game UI.
///
/// Subdivides the viewport into the standard 3-column pane structure.
/// This is pure computation — no rendering, no `Frame` dependency.
///
/// The returned [`PaneLayout`] contains outer rects. Use
/// [`BackendMetrics::inner_area`] to get usable inner space.
pub fn compute_pane_layout(viewport: Rect, config: &PaneLayoutConfig) -> PaneLayout {
    let total_width = viewport.width;

    // Try boosted left column (wider log/actions)
    let boosted_left = (total_width * config.boosted_left_column_pct) / 100;
    let boosted_middle = (total_width * (config.middle_column_pct - 5)) / 100;
    let boosted_right = total_width.saturating_sub(boosted_left + boosted_middle);

    let can_boost = boosted_left >= config.min_width_log
        && boosted_left >= config.min_width_actions
        && boosted_middle >= config.min_width_battlefield
        && boosted_right >= config.min_width_card_details
        && boosted_right >= config.min_width_hand;

    let (left_pct, middle_pct, right_pct) = if can_boost {
        (
            config.boosted_left_column_pct,
            config.middle_column_pct - 5,
            config.right_column_pct,
        )
    } else {
        (
            config.left_column_pct,
            config.middle_column_pct,
            config.right_column_pct,
        )
    };

    // 3-column split
    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(left_pct),
            Constraint::Percentage(middle_pct),
            Constraint::Percentage(right_pct),
        ])
        .split(viewport);

    // Left column: 50/50 vertical split
    let left_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(main_chunks[0]);

    // Middle column: 50/50 vertical split, then info bar + battlefield in each half
    let middle_halves = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(main_chunks[1]);

    let info_bar_height = calculate_info_bar_height(main_chunks[1].width);

    // Top half: opponent info bar at top, battlefield fills rest
    let top_half = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(info_bar_height),
            Constraint::Min(0),
        ])
        .split(middle_halves[0]);

    // Bottom half: battlefield fills most, your info bar at bottom
    let bottom_half = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(info_bar_height),
        ])
        .split(middle_halves[1]);

    // Right column: 50/50 vertical split
    let right_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(main_chunks[2]);

    let mut panes = HashMap::new();
    panes.insert(PaneId::Log, left_chunks[0]);
    panes.insert(PaneId::Actions, left_chunks[1]);
    panes.insert(PaneId::OpponentInfo, top_half[0]);
    panes.insert(PaneId::OpponentBattlefield, top_half[1]);
    panes.insert(PaneId::YourBattlefield, bottom_half[0]);
    panes.insert(PaneId::YourInfo, bottom_half[1]);
    panes.insert(PaneId::CardDetails, right_chunks[0]);
    panes.insert(PaneId::Hand, right_chunks[1]);

    PaneLayout { panes }
}

/// Calculate info bar height based on available width.
///
/// Returns 3 (1-line content + 2 borders) or 4 (2-line content + 2 borders)
/// depending on whether the status line fits on one line.
fn calculate_info_bar_height(available_width: u16) -> u16 {
    let inner_width = available_width.saturating_sub(4);

    // Left: "Name: 20 life | Hand: 7 | GY: 99 | Lib: 99" (~42 chars)
    // Right: "Turn: 99 (99) | UP UK DR M1 BC DA DB CD EC M2 ET" (~48 chars)
    const STATS_MAX_LEN: u16 = 42;
    const PHASE_MAX_LEN: u16 = 48;
    const MIN_SPACING: u16 = 3;

    if STATS_MAX_LEN + PHASE_MAX_LEN + MIN_SPACING > inner_width {
        4 // 2 borders + 2 content lines
    } else {
        3 // 2 borders + 1 content line
    }
}

// ═══════════════════════════════════════════════════════════════
// Card sizing and placement
// ═══════════════════════════════════════════════════════════════

/// Configuration for card sizing calculations.
pub struct CardSizeConfig {
    /// Default card width in cells (untapped)
    pub default_width: u16,
    /// Default card height in cells (untapped)
    pub default_height: u16,
    /// Minimum card width in cells
    pub min_width: u16,
    /// Minimum card height in cells
    pub min_height: u16,
    /// Maximum card height in cells
    pub max_height: u16,
    /// Spacing between cards in cells
    pub spacing: u16,
}

impl Default for CardSizeConfig {
    fn default() -> Self {
        Self {
            default_width: 10,
            default_height: 7,
            min_width: 5,
            min_height: 4,
            max_height: 15,
            spacing: 1,
        }
    }
}

/// Compute card width from height maintaining the default aspect ratio.
///
/// Uses the ratio `default_width / default_height` (typically 10/7 ≈ 1.43)
/// which accounts for terminal character aspect (~2:1 height:width).
pub fn compute_card_width(height: u16, config: &CardSizeConfig) -> u16 {
    ((f32::from(height) * f32::from(config.default_width)) / f32::from(config.default_height)).round() as u16
}

/// Compute dimensions for a tapped card (rotated ~90 degrees).
///
/// Tapped cards become wider and shorter to simulate horizontal rotation.
pub fn tapped_dimensions(base_width: u16, base_height: u16) -> (u16, u16) {
    let tapped_width = (base_width * 3 / 2).max(base_width);
    let tapped_height = (base_height * 3 / 5).max(4);
    (tapped_width, tapped_height)
}

/// A card item for layout computation.
///
/// Backend-neutral description of a card to be positioned.
/// The layout engine doesn't need to know game-specific types.
#[derive(Debug, Clone)]
pub struct CardItem {
    /// Unique identifier for this card/entity
    pub id: u32,
    /// Display name
    pub name: String,
    /// Whether the card is tapped (affects dimensions)
    pub is_tapped: bool,
    /// Card category for section grouping
    pub category: CardCategory,
    /// Number of cards in a stack (for visual stacks)
    pub stack_size: u16,
}

/// Card category for battlefield section grouping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CardCategory {
    Planeswalker,
    Creature,
    Enchantment,
    Artifact,
    Land,
}

/// Result of laying out a card — its computed position.
#[derive(Debug, Clone)]
pub struct CardPlacement {
    /// The card item that was placed
    pub id: u32,
    /// Position and size in cell coordinates
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

/// A section of cards on the battlefield (e.g., "Creatures", "Lands").
#[derive(Debug)]
pub struct CardSection {
    pub label: String,
    pub category: CardCategory,
    pub cards: Vec<CardItem>,
}

/// Compute optimal card size for a battlefield area.
///
/// Tries different row counts (1-4) and finds the maximum card height
/// that fits all cards. Returns (width, height).
pub fn compute_battlefield_card_size(
    area: Rect,
    total_cards: usize,
    config: &CardSizeConfig,
) -> (u16, u16) {
    if total_cards == 0 || area.width == 0 || area.height == 0 {
        return (config.min_width, config.min_height);
    }

    let mut best_height = config.min_height;

    for target_rows in 1..=4u16 {
        // Try increasing heights for this row count
        for h in config.min_height..=config.max_height {
            let w = compute_card_width(h, config).max(config.min_width);
            let cards_per_row = area
                .width
                .checked_div(w + config.spacing)
                .unwrap_or(1)
                .max(1);
            let rows_needed = (total_cards as u16 + cards_per_row - 1) / cards_per_row;
            let height_needed = rows_needed * (h + config.spacing);

            if rows_needed <= target_rows && height_needed <= area.height {
                best_height = h;
            } else if rows_needed > target_rows {
                break;
            }
        }
    }

    let best_width = compute_card_width(best_height, config).max(config.min_width);
    (best_width, best_height)
}

/// Lay out cards in a word-wrap flow within the given area.
///
/// Places cards left-to-right, wrapping to the next row when the
/// current row is full. Returns positioned `CardPlacement` values.
///
/// This is the core shared algorithm used by both TUI and web backends.
pub fn layout_cards_wordwrap(
    area: Rect,
    cards: &[CardItem],
    card_width: u16,
    card_height: u16,
    config: &CardSizeConfig,
) -> Vec<CardPlacement> {
    if cards.is_empty() || area.width == 0 || area.height == 0 {
        return Vec::new();
    }

    let mut placements = Vec::with_capacity(cards.len());
    let mut x = area.x;
    let mut y = area.y;
    let mut row_max_height = 0u16;

    for card in cards {
        let (w, h) = if card.is_tapped {
            tapped_dimensions(card_width, card_height)
        } else {
            (card_width, card_height)
        };

        // Wrap to next row if this card doesn't fit
        if x > area.x && x + w > area.x + area.width {
            y += row_max_height + config.spacing;
            x = area.x;
            row_max_height = 0;
        }

        // Stop if we've exceeded the vertical space
        if y + h > area.y + area.height {
            break;
        }

        placements.push(CardPlacement {
            id: card.id,
            x,
            y,
            width: w,
            height: h,
        });

        x += w + config.spacing;
        row_max_height = row_max_height.max(h);
    }

    placements
}

/// Lay out hand cards as a vertical list (one per row).
///
/// Each card occupies one row at full pane width.
pub fn layout_hand_cards(area: Rect, card_count: usize) -> Vec<CardPlacement> {
    let mut placements = Vec::with_capacity(card_count);

    for i in 0..card_count {
        let y = area.y + i as u16;
        if y >= area.y + area.height {
            break;
        }

        placements.push(CardPlacement {
            id: i as u32,
            x: area.x,
            y,
            width: area.width,
            height: 1,
        });
    }

    placements
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pane_layout_basic() {
        let viewport = Rect::new(0, 0, 120, 40);
        let config = PaneLayoutConfig::default();
        let layout = compute_pane_layout(viewport, &config);

        // All 8 panes should be present
        assert!(layout.get(PaneId::Log).is_some());
        assert!(layout.get(PaneId::Actions).is_some());
        assert!(layout.get(PaneId::OpponentInfo).is_some());
        assert!(layout.get(PaneId::OpponentBattlefield).is_some());
        assert!(layout.get(PaneId::YourBattlefield).is_some());
        assert!(layout.get(PaneId::YourInfo).is_some());
        assert!(layout.get(PaneId::CardDetails).is_some());
        assert!(layout.get(PaneId::Hand).is_some());
    }

    #[test]
    fn test_pane_layout_covers_viewport() {
        let viewport = Rect::new(0, 0, 160, 50);
        let config = PaneLayoutConfig::default();
        let layout = compute_pane_layout(viewport, &config);

        // Left column panes should be at x=0
        let log = layout.get(PaneId::Log).unwrap();
        assert_eq!(log.x, 0);

        // Hand pane should extend to the right edge
        let hand = layout.get(PaneId::Hand).unwrap();
        assert_eq!(hand.x + hand.width, viewport.width);
    }

    #[test]
    fn test_pane_layout_narrow_viewport() {
        // Very narrow — should NOT boost left column
        let viewport = Rect::new(0, 0, 80, 30);
        let config = PaneLayoutConfig::default();
        let layout = compute_pane_layout(viewport, &config);

        let log = layout.get(PaneId::Log).unwrap();
        // At 80 cols, 25% = 20, which is < min_width_log (40)
        // So no boost, default percentages apply
        assert!(log.width > 0);
    }

    #[test]
    fn test_ratatui_metrics_inner_area() {
        let metrics = RatatuiMetrics::cli();
        let outer = Rect::new(5, 5, 40, 20);

        let inner = metrics.inner_area(outer, true);
        assert_eq!(inner.x, 6);
        assert_eq!(inner.y, 6);
        assert_eq!(inner.width, 38);
        assert_eq!(inner.height, 18);

        let no_border = metrics.inner_area(outer, false);
        assert_eq!(no_border, outer);
    }

    #[test]
    fn test_info_bar_height() {
        // Wide enough for single line
        assert_eq!(calculate_info_bar_height(200), 3);
        // Too narrow, needs two lines
        assert_eq!(calculate_info_bar_height(80), 4);
    }

    #[test]
    fn test_compute_card_width() {
        let config = CardSizeConfig::default();
        // Default: height 7 → width 10 (10/7 ratio)
        assert_eq!(compute_card_width(7, &config), 10);
        // Height 4 → width 6 (rounded)
        assert_eq!(compute_card_width(4, &config), 6);
    }

    #[test]
    fn test_tapped_dimensions() {
        let (w, h) = tapped_dimensions(10, 7);
        assert!(w > 10, "Tapped width should be larger");
        assert!(h < 7, "Tapped height should be smaller");
        assert!(h >= 4, "Tapped height should be at least 4");
    }

    #[test]
    fn test_battlefield_card_size() {
        let area = Rect::new(0, 0, 60, 20);
        let config = CardSizeConfig::default();

        // Few cards should get large sizes
        let (w, h) = compute_battlefield_card_size(area, 3, &config);
        assert!(w >= config.min_width);
        assert!(h >= config.min_height);

        // Many cards should get smaller sizes
        let (w2, h2) = compute_battlefield_card_size(area, 20, &config);
        assert!(h2 <= h, "More cards should produce smaller or equal height");
        assert!(w2 >= config.min_width);
    }

    #[test]
    fn test_layout_cards_wordwrap() {
        let area = Rect::new(5, 5, 30, 20);
        let config = CardSizeConfig::default();
        let cards: Vec<CardItem> = (0..6)
            .map(|i| CardItem {
                id: i,
                name: format!("Card {}", i),
                is_tapped: false,
                category: CardCategory::Creature,
                stack_size: 1,
            })
            .collect();

        let placements = layout_cards_wordwrap(area, &cards, 8, 5, &config);
        assert_eq!(placements.len(), 6);

        // First card starts at area origin
        assert_eq!(placements[0].x, 5);
        assert_eq!(placements[0].y, 5);

        // Cards should wrap to next row (30 width, 8+1 per card = 3 per row)
        // Row 1: cards 0,1,2 at y=5
        // Row 2: cards 3,4,5 at y=5+5+1=11
        assert_eq!(placements[3].y, 11);
    }

    #[test]
    fn test_layout_hand_cards() {
        let area = Rect::new(0, 0, 30, 10);
        let placements = layout_hand_cards(area, 7);
        assert_eq!(placements.len(), 7);

        // Each card is 1 row tall at full width
        for (i, p) in placements.iter().enumerate() {
            assert_eq!(p.y, i as u16);
            assert_eq!(p.width, 30);
            assert_eq!(p.height, 1);
        }
    }

    #[test]
    fn test_layout_hand_truncates_at_area_height() {
        let area = Rect::new(0, 0, 30, 5);
        let placements = layout_hand_cards(area, 10);
        // Only 5 fit in 5 rows
        assert_eq!(placements.len(), 5);
    }
}
