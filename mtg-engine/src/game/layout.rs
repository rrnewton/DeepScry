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
}
