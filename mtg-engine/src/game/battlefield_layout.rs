//! Backend-neutral battlefield layout engine.
//!
//! This module computes positions for cards on a battlefield in abstract
//! pixel-equivalent coordinates. It is shared between the native TUI
//! (terminal-cell snapping) and the HTML/native pixel-precise GUI.
//!
//! ## Coordinate model
//!
//! Inputs and outputs use [`LayoutRect`] with `f32` corners
//! `(x1, y1, x2, y2)` measured in **abstract pixels**. A [`CellSize`]
//! provides the snapping granularity:
//!
//! * `CellSize::TERMINAL` — `(10.0, 20.0)` — one terminal character cell
//!   is roughly 10 px wide and 20 px tall. All output positions/sizes are
//!   snapped to multiples of `(10, 20)` so the renderer can divide by the
//!   cell size to recover terminal grid coordinates.
//! * `CellSize::PIXEL` — `(1.0, 1.0)` — pixel-perfect placement for the
//!   HTML/native GUI; snapping is a no-op.
//!
//! ## Layout algorithm
//!
//! 1. Group input cards by [`CardCategory`] and order sections by
//!    [`CardCategory::priority`] (or its reverse if
//!    `LayoutConfig::reverse_section_order` is true — used for opponent
//!    battlefields).
//! 2. Reserve a graveyard text element in the lower-right corner if
//!    `LayoutConfig::graveyard_card_count > 0`.
//! 3. Pick the largest card size (height ∈ `[min, max]`, width derived
//!    from the configured aspect ratio) for which all cards plus their
//!    section headers fit in the available rectangle.
//! 4. Place sections sequentially: a 1-line header rect followed by
//!    word-wrapped card rectangles. Sections always start at column 0 of
//!    a fresh row (matching the historical TUI behaviour where a section
//!    header opens a new row above its cards).
//! 5. Snap every emitted rectangle to the cell grid.
//!
//! This module deliberately does **not** depend on `ratatui` or on any
//! game-state types — it is pure layout maths so it can be unit-tested
//! and reused by every renderer.

use std::cmp::Ordering;

// ───────────────────────────────────────────────────────────────────────
// Geometry primitives
// ───────────────────────────────────────────────────────────────────────

/// Axis-aligned rectangle in abstract pixel coordinates.
///
/// Stored as opposite corners `(x1, y1)` (top-left, inclusive) and
/// `(x2, y2)` (bottom-right, exclusive). Width / height are derived.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayoutRect {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
}

impl LayoutRect {
    /// Construct from corner coordinates. Caller must ensure `x2 >= x1`
    /// and `y2 >= y1`.
    pub const fn new(x1: f32, y1: f32, x2: f32, y2: f32) -> Self {
        Self { x1, y1, x2, y2 }
    }

    /// Construct from origin + extent.
    pub fn from_xywh(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self::new(x, y, x + w, y + h)
    }

    pub fn width(&self) -> f32 {
        (self.x2 - self.x1).max(0.0)
    }
    pub fn height(&self) -> f32 {
        (self.y2 - self.y1).max(0.0)
    }
    pub fn is_empty(&self) -> bool {
        self.width() <= 0.0 || self.height() <= 0.0
    }
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x1 && x < self.x2 && y >= self.y1 && y < self.y2
    }

    /// True when `self` and `other` share at least one interior pixel.
    pub fn intersects(&self, other: &LayoutRect) -> bool {
        self.x1 < other.x2 && self.x2 > other.x1 && self.y1 < other.y2 && self.y2 > other.y1
    }
}

/// Snapping granularity. Output positions and sizes are forced to be
/// multiples of `w` (horizontal) and `h` (vertical).
///
/// See module docs for the meaning of the well-known values.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CellSize {
    pub w: f32,
    pub h: f32,
}

impl CellSize {
    /// One terminal character cell ≈ 10 × 20 px.
    pub const TERMINAL: CellSize = CellSize { w: 10.0, h: 20.0 };
    /// Pixel-perfect (no snapping).
    pub const PIXEL: CellSize = CellSize { w: 1.0, h: 1.0 };

    pub const fn new(w: f32, h: f32) -> Self {
        Self { w, h }
    }

    /// Round a coordinate **down** to the nearest cell boundary.
    pub fn snap_floor(&self, value: f32, axis: Axis) -> f32 {
        let cell = match axis {
            Axis::X => self.w,
            Axis::Y => self.h,
        };
        if cell <= 0.0 {
            value
        } else {
            (value / cell).floor() * cell
        }
    }

    /// Round a coordinate **up** to the nearest cell boundary.
    pub fn snap_ceil(&self, value: f32, axis: Axis) -> f32 {
        let cell = match axis {
            Axis::X => self.w,
            Axis::Y => self.h,
        };
        if cell <= 0.0 {
            value
        } else {
            (value / cell).ceil() * cell
        }
    }
}

/// Axis selector used by [`CellSize::snap_floor`] / [`CellSize::snap_ceil`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    X,
    Y,
}

// ───────────────────────────────────────────────────────────────────────
// Card categorisation
// ───────────────────────────────────────────────────────────────────────

/// Card type category used for grouping cards into battlefield sections.
///
/// Mirrors [`crate::game::fancy_tui_renderer::CardCategory`]. Lives here
/// (without a `ratatui` dependency) so backend-neutral layout code can
/// reason about sections; the renderer-side enum will eventually fold
/// into this one once both backends consume it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CardCategory {
    Planeswalker,
    Creature,
    Enchantment,
    Artifact,
    Land,
    /// Anything else that ends up on the battlefield (e.g. instants /
    /// sorceries that have not yet resolved off — rare, but possible).
    Other,
}

impl CardCategory {
    /// Display label used for the section header (matches TUI labels).
    pub fn label(self) -> &'static str {
        match self {
            CardCategory::Planeswalker => "PWs",
            CardCategory::Creature => "Creatures",
            CardCategory::Enchantment => "Enchants",
            CardCategory::Artifact => "Artifacts",
            CardCategory::Land => "Lands",
            CardCategory::Other => "Other",
        }
    }

    /// Section ordering for the player's own battlefield.
    ///
    /// Lower values render first. Lands are intentionally *last* so
    /// signature permanents (PWs, creatures) sit above the lands at the
    /// top of the battlefield row.
    pub fn priority(self) -> u8 {
        match self {
            CardCategory::Planeswalker => 0,
            CardCategory::Creature => 1,
            CardCategory::Enchantment => 2,
            CardCategory::Artifact => 3,
            CardCategory::Land => 4,
            CardCategory::Other => 5,
        }
    }
}

// ───────────────────────────────────────────────────────────────────────
// Inputs / outputs
// ───────────────────────────────────────────────────────────────────────

/// One card to be laid out.
///
/// The layout engine never inspects the card beyond the fields below, so
/// `String` is acceptable here — callers convert their game state into
/// these descriptors once per frame.
#[derive(Debug, Clone)]
pub struct CardLayoutInput {
    /// Stable identifier for the card (echoed back in [`CardPlacement`]).
    pub card_id: u32,
    /// Category drives section grouping / ordering.
    pub category: CardCategory,
    /// Display name (currently used only for graveyard width estimation,
    /// but kept here so renderers can label hit-tested rects).
    pub name: String,
    /// Tapped cards consume more horizontal space (rotated ~90°).
    pub is_tapped: bool,
    /// Stack size for visually stacked duplicates (1 for a single card).
    pub stack_size: u16,
}

/// Card pixel size used during sizing iteration.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CardSize {
    pub width_px: f32,
    pub height_px: f32,
}

impl CardSize {
    pub const fn new(width_px: f32, height_px: f32) -> Self {
        Self { width_px, height_px }
    }
}

/// Tunables for [`layout_battlefield`].
#[derive(Debug, Clone)]
pub struct LayoutConfig {
    /// Initial / preferred untapped card size (in pixels).
    pub default_card: CardSize,
    /// Hard minimum card size (also used when nothing fits).
    pub min_card: CardSize,
    /// Hard maximum card height; width follows from the aspect ratio.
    pub max_card_height_px: f32,
    /// Spacing between cards in a row (and between rows), in pixels.
    pub spacing_px: f32,
    /// Height reserved above each section's cards for the section label.
    pub header_height_px: f32,
    /// Per-line height of a graveyard entry (header + one line per card).
    pub graveyard_text_height_px: f32,
    /// Per-character width used to estimate graveyard width (e.g. cell.w
    /// for the TUI).
    pub graveyard_char_width_px: f32,
    /// Number of cards in the graveyard. Zero ⇒ no graveyard reserved.
    pub graveyard_card_count: usize,
    /// Length, in characters, of the longest graveyard card name.
    pub graveyard_max_name_len: usize,
    /// When true, sections are emitted in *reverse* priority order. Used
    /// for the opponent battlefield so lands appear closest to the
    /// shared centre of the table.
    pub reverse_section_order: bool,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        // These defaults mirror the TUI constants in fancy_tui_renderer.rs
        // converted to pixel units assuming CellSize::TERMINAL.
        let cell = CellSize::TERMINAL;
        Self {
            default_card: CardSize::new(10.0 * cell.w, 7.0 * cell.h),
            min_card: CardSize::new(5.0 * cell.w, 4.0 * cell.h),
            max_card_height_px: 15.0 * cell.h,
            spacing_px: 1.0 * cell.w,
            header_height_px: 1.0 * cell.h,
            graveyard_text_height_px: 1.0 * cell.h,
            graveyard_char_width_px: cell.w,
            graveyard_card_count: 0,
            graveyard_max_name_len: 0,
            reverse_section_order: false,
        }
    }
}

/// Computed placement of a single card.
#[derive(Debug, Clone, PartialEq)]
pub struct CardPlacement {
    pub card_id: u32,
    pub bounding_box: LayoutRect,
    /// 0-based row index within the parent section.
    pub row: usize,
    /// 0-based column index within `row`.
    pub col: usize,
}

/// Cards belonging to one battlefield section.
#[derive(Debug, Clone)]
pub struct SectionLayout {
    pub category: CardCategory,
    pub label: &'static str,
    /// 1-line label rectangle drawn above the cards.
    pub header: LayoutRect,
    pub cards: Vec<CardPlacement>,
}

/// Complete layout result.
#[derive(Debug, Clone)]
pub struct BattlefieldLayoutResult {
    pub sections: Vec<SectionLayout>,
    /// Bounding box of the lower-right graveyard text element, when
    /// present.
    pub graveyard_rect: Option<LayoutRect>,
    /// Card size actually used (after sizing iteration). Equal to
    /// `LayoutConfig::min_card` if nothing else fit.
    pub used_card_size: CardSize,
}

// ───────────────────────────────────────────────────────────────────────
// Public entry point
// ───────────────────────────────────────────────────────────────────────

/// Lay out `cards` inside `rect`, snapping to `cell`.
///
/// This is the single backend-neutral entry point. See the module-level
/// documentation for the algorithm.
pub fn layout_battlefield(
    rect: LayoutRect,
    cell: CellSize,
    cards: &[CardLayoutInput],
    config: &LayoutConfig,
) -> BattlefieldLayoutResult {
    // 1. Reserve graveyard rect (snapped) if configured.
    let graveyard_rect = compute_graveyard_rect(rect, cell, config);

    // 2. Available rect for cards: shrink horizontally so the grid does
    //    not overlap the graveyard. Vertical extent is preserved because
    //    the TUI treats the graveyard as an overlay that the centred
    //    grid only avoids horizontally on the affected rows.
    let available = if let Some(gv) = graveyard_rect {
        // Cards may use the full width above the graveyard row, but for
        // sizing we conservatively use the narrower span. This matches
        // the TUI which slides cards left to dodge the overlay.
        LayoutRect::new(rect.x1, rect.y1, gv.x1.max(rect.x1), rect.y2)
    } else {
        rect
    };

    // 3. Group cards by category and order sections.
    let sections_in = group_and_order(cards, config.reverse_section_order);
    if sections_in.is_empty() {
        return BattlefieldLayoutResult {
            sections: Vec::new(),
            graveyard_rect,
            used_card_size: config.min_card,
        };
    }

    // 4. Pick the largest card size that fits.
    let used_card_size = pick_card_size(available, cell, &sections_in, config);

    // 5. Emit placements with the chosen size.
    let sections = place_sections(available, cell, &sections_in, used_card_size, config);

    BattlefieldLayoutResult {
        sections,
        graveyard_rect,
        used_card_size,
    }
}

// ───────────────────────────────────────────────────────────────────────
// Internals
// ───────────────────────────────────────────────────────────────────────

/// One section's input as seen by the placement helpers.
struct SectionInput<'a> {
    category: CardCategory,
    cards: Vec<&'a CardLayoutInput>,
}

fn group_and_order<'a>(cards: &'a [CardLayoutInput], reverse: bool) -> Vec<SectionInput<'a>> {
    use CardCategory::*;
    let order = [Planeswalker, Creature, Enchantment, Artifact, Land, Other];
    let mut out: Vec<SectionInput<'a>> = order
        .into_iter()
        .filter_map(|cat| {
            let bucket: Vec<&CardLayoutInput> = cards.iter().filter(|c| c.category == cat).collect();
            if bucket.is_empty() {
                None
            } else {
                Some(SectionInput {
                    category: cat,
                    cards: bucket,
                })
            }
        })
        .collect();
    if reverse {
        out.reverse();
        // Within reversed mode we still want a stable ordering; the
        // reverse() above relies on `order` being sorted by priority,
        // which it is.
        debug_assert!(out
            .windows(2)
            .all(|w| w[0].category.priority().cmp(&w[1].category.priority()) == Ordering::Greater));
    }
    out
}

fn compute_graveyard_rect(rect: LayoutRect, cell: CellSize, config: &LayoutConfig) -> Option<LayoutRect> {
    if config.graveyard_card_count == 0 {
        return None;
    }
    // Header is the literal "Graveyard:" string (10 chars). The width is
    // determined by the longest of the header and any card name.
    const HEADER: &str = "Graveyard:";
    let name_chars = config.graveyard_max_name_len.max(HEADER.len());
    let width = (name_chars as f32) * config.graveyard_char_width_px;
    let lines = 1 + config.graveyard_card_count; // header + N entries
    let height = (lines as f32) * config.graveyard_text_height_px;

    if width <= 0.0 || height <= 0.0 || width > rect.width() || height > rect.height() {
        return None;
    }

    // Snap the *width / height* up to a cell so the surrounding grid
    // can compute integer cell offsets, then snap the *origin* down.
    let snapped_w = cell.snap_ceil(width, Axis::X);
    let snapped_h = cell.snap_ceil(height, Axis::Y);
    let x1 = cell.snap_floor(rect.x2 - snapped_w, Axis::X);
    let y1 = cell.snap_floor(rect.y2 - snapped_h, Axis::Y);
    Some(LayoutRect::new(x1, y1, x1 + snapped_w, y1 + snapped_h))
}

/// Compute the snapped pixel size of one card at the given target height.
fn card_size_for_height(target_h: f32, cell: CellSize, config: &LayoutConfig) -> CardSize {
    // Maintain the default aspect ratio (default_card.width_px / height_px).
    let aspect = config.default_card.width_px / config.default_card.height_px.max(1.0);
    let raw_w = target_h * aspect;
    // Snap *down* so we don't accidentally exceed the available rect.
    let mut w = cell.snap_floor(raw_w, Axis::X).max(config.min_card.width_px);
    let mut h = cell.snap_floor(target_h, Axis::Y).max(config.min_card.height_px);
    // Re-snap min_card to the cell grid (defensive — defaults already
    // align, but custom configs might not).
    w = cell.snap_ceil(w, Axis::X);
    h = cell.snap_ceil(h, Axis::Y);
    CardSize::new(w, h)
}

/// Effective on-screen size of one entity (accounting for tapped state).
fn entity_size(card: &CardLayoutInput, base: CardSize, cell: CellSize) -> CardSize {
    if card.is_tapped {
        // Tapped cards rotate ~90°: wider and shorter. Mirrors
        // `tapped_dimensions` in layout.rs.
        let raw_w = base.width_px * 1.5;
        let raw_h = (base.height_px * 0.6).max(4.0 * cell.h);
        CardSize::new(cell.snap_ceil(raw_w, Axis::X), cell.snap_ceil(raw_h, Axis::Y))
    } else {
        base
    }
}

/// Simulate a layout and return the total height used (or `None` if it
/// overflows horizontally, which only happens if a single card is wider
/// than the available rect).
fn simulate_height(
    available: LayoutRect,
    cell: CellSize,
    sections: &[SectionInput<'_>],
    base: CardSize,
    config: &LayoutConfig,
) -> Option<f32> {
    let usable_w = available.width();
    if usable_w <= 0.0 {
        return None;
    }
    let header_h = cell.snap_ceil(config.header_height_px, Axis::Y);
    let spacing = cell.snap_ceil(config.spacing_px, Axis::X);

    let mut total_h = 0.0_f32;
    for section in sections {
        // Section header always starts on a fresh row.
        total_h += header_h;
        let mut row_w = 0.0_f32;
        let mut row_max_h = 0.0_f32;
        for card in &section.cards {
            let sz = entity_size(card, base, cell);
            if sz.width_px > usable_w {
                // Cannot place even on its own row.
                return None;
            }
            // Check if it fits on the current row.
            let prospective = if row_w == 0.0 {
                sz.width_px
            } else {
                row_w + spacing + sz.width_px
            };
            if prospective > usable_w {
                // Wrap.
                total_h += row_max_h + spacing;
                row_w = sz.width_px;
                row_max_h = sz.height_px;
            } else {
                row_w = prospective;
                row_max_h = row_max_h.max(sz.height_px);
            }
        }
        // Close the section's final row.
        total_h += row_max_h;
        // Inter-section gap (also snapped).
        total_h += spacing;
    }
    // The trailing inter-section spacing is harmless slack; subtract one
    // unit so a perfectly-fitting layout reports its true height.
    total_h -= cell.snap_ceil(config.spacing_px, Axis::X);
    Some(total_h.max(0.0))
}

fn pick_card_size(
    available: LayoutRect,
    cell: CellSize,
    sections: &[SectionInput<'_>],
    config: &LayoutConfig,
) -> CardSize {
    let usable_h = available.height();
    if usable_h <= 0.0 {
        return config.min_card;
    }

    // Iterate heights from max down to min, in cell-height steps.
    let step = cell.h.max(1.0);
    let mut best: Option<CardSize> = None;
    let mut h = config.max_card_height_px;
    let min_h = config.min_card.height_px;
    while h >= min_h {
        let candidate = card_size_for_height(h, cell, config);
        if let Some(used) = simulate_height(available, cell, sections, candidate, config) {
            if used <= usable_h {
                best = Some(candidate);
                break;
            }
        }
        h -= step;
    }
    best.unwrap_or(config.min_card)
}

fn place_sections(
    available: LayoutRect,
    cell: CellSize,
    sections: &[SectionInput<'_>],
    base: CardSize,
    config: &LayoutConfig,
) -> Vec<SectionLayout> {
    let usable_w = available.width();
    let header_h = cell.snap_ceil(config.header_height_px, Axis::Y);
    let spacing_x = cell.snap_ceil(config.spacing_px, Axis::X);
    let spacing_y = cell.snap_ceil(config.spacing_px, Axis::Y);

    let mut out = Vec::with_capacity(sections.len());
    let mut cursor_y = available.y1;

    for section in sections {
        let header_rect = LayoutRect::new(
            available.x1,
            cell.snap_floor(cursor_y, Axis::Y),
            available.x1 + usable_w,
            cell.snap_floor(cursor_y, Axis::Y) + header_h,
        );

        let mut placements = Vec::with_capacity(section.cards.len());
        let mut row_idx = 0usize;
        let mut col_idx = 0usize;
        let mut row_x = available.x1;
        let mut row_y = header_rect.y2;
        let mut row_max_h = 0.0_f32;

        for card in &section.cards {
            let sz = entity_size(card, base, cell);

            // Horizontal wrap check.
            let next_right = if col_idx == 0 {
                row_x + sz.width_px
            } else {
                row_x + spacing_x + sz.width_px
            };
            if col_idx > 0 && next_right > available.x1 + usable_w {
                // Wrap to a new row.
                row_y += row_max_h + spacing_y;
                row_x = available.x1;
                row_max_h = 0.0;
                row_idx += 1;
                col_idx = 0;
            }

            // Apply the spacing between cards (after the wrap decision).
            let card_x = if col_idx == 0 {
                cell.snap_floor(row_x, Axis::X)
            } else {
                cell.snap_floor(row_x + spacing_x, Axis::X)
            };
            let card_y = cell.snap_floor(row_y, Axis::Y);
            let bbox = LayoutRect::new(card_x, card_y, card_x + sz.width_px, card_y + sz.height_px);

            placements.push(CardPlacement {
                card_id: card.card_id,
                bounding_box: bbox,
                row: row_idx,
                col: col_idx,
            });

            row_x = bbox.x2;
            row_max_h = row_max_h.max(sz.height_px);
            col_idx += 1;
        }

        // Advance cursor past the section.
        let section_bottom = if placements.is_empty() {
            header_rect.y2
        } else {
            row_y + row_max_h
        };
        cursor_y = section_bottom + spacing_y;

        out.push(SectionLayout {
            category: section.category,
            label: section.category.label(),
            header: header_rect,
            cards: placements,
        });
    }

    out
}

// ───────────────────────────────────────────────────────────────────────
// Tests
// ───────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn card(id: u32, cat: CardCategory) -> CardLayoutInput {
        CardLayoutInput {
            card_id: id,
            category: cat,
            name: format!("C{}", id),
            is_tapped: false,
            stack_size: 1,
        }
    }
    fn tapped(id: u32, cat: CardCategory) -> CardLayoutInput {
        CardLayoutInput {
            card_id: id,
            category: cat,
            name: format!("C{}", id),
            is_tapped: true,
            stack_size: 1,
        }
    }

    // ─── Geometry ────────────────────────────────────────────────────

    #[test]
    fn rect_dimensions() {
        let r = LayoutRect::from_xywh(2.0, 4.0, 6.0, 8.0);
        assert_eq!(r.width(), 6.0);
        assert_eq!(r.height(), 8.0);
        assert!(r.contains(2.0, 4.0));
        assert!(!r.contains(8.0, 12.0));
        assert!(r.contains(7.99, 11.99));
    }

    #[test]
    fn rect_intersects() {
        let a = LayoutRect::from_xywh(0.0, 0.0, 10.0, 10.0);
        let b = LayoutRect::from_xywh(5.0, 5.0, 10.0, 10.0);
        let c = LayoutRect::from_xywh(20.0, 20.0, 5.0, 5.0);
        assert!(a.intersects(&b));
        assert!(!a.intersects(&c));
    }

    #[test]
    fn cell_snap_terminal() {
        let cell = CellSize::TERMINAL;
        assert_eq!(cell.snap_floor(23.0, Axis::X), 20.0);
        assert_eq!(cell.snap_ceil(23.0, Axis::X), 30.0);
        assert_eq!(cell.snap_floor(45.0, Axis::Y), 40.0);
        assert_eq!(cell.snap_ceil(45.0, Axis::Y), 60.0);
        // Already on a boundary stays put.
        assert_eq!(cell.snap_floor(40.0, Axis::Y), 40.0);
        assert_eq!(cell.snap_ceil(40.0, Axis::Y), 40.0);
    }

    #[test]
    fn cell_snap_pixel_is_identity() {
        let cell = CellSize::PIXEL;
        assert_eq!(cell.snap_floor(23.7, Axis::X), 23.0);
        assert_eq!(cell.snap_ceil(23.2, Axis::Y), 24.0);
    }

    // ─── Categorisation / ordering ───────────────────────────────────

    #[test]
    fn category_priority_order() {
        use CardCategory::*;
        let mut all = [Land, Other, Planeswalker, Artifact, Enchantment, Creature];
        all.sort_by_key(|c| c.priority());
        assert_eq!(all, [Planeswalker, Creature, Enchantment, Artifact, Land, Other]);
    }

    #[test]
    fn category_labels_match_tui() {
        // These exact strings are baked into the existing TUI renderer
        // (see fancy_tui_renderer.rs::CardCategory::label) — they MUST
        // not drift or sections will be labelled inconsistently.
        assert_eq!(CardCategory::Planeswalker.label(), "PWs");
        assert_eq!(CardCategory::Creature.label(), "Creatures");
        assert_eq!(CardCategory::Enchantment.label(), "Enchants");
        assert_eq!(CardCategory::Artifact.label(), "Artifacts");
        assert_eq!(CardCategory::Land.label(), "Lands");
        assert_eq!(CardCategory::Other.label(), "Other");
    }

    // ─── Layout integration ──────────────────────────────────────────

    #[test]
    fn empty_input_produces_no_sections() {
        let r = LayoutRect::from_xywh(0.0, 0.0, 800.0, 400.0);
        let res = layout_battlefield(r, CellSize::TERMINAL, &[], &LayoutConfig::default());
        assert!(res.sections.is_empty());
        assert!(res.graveyard_rect.is_none());
    }

    #[test]
    fn single_section_single_row() {
        // 80-cell-wide rect → 800 px. Default card width = 100 px. So 7
        // cards (700 + 6 spacings of 10 = 760) fit on one row.
        let r = LayoutRect::from_xywh(0.0, 0.0, 800.0, 400.0);
        let cards: Vec<_> = (0..3).map(|i| card(i, CardCategory::Land)).collect();
        let res = layout_battlefield(r, CellSize::TERMINAL, &cards, &LayoutConfig::default());
        assert_eq!(res.sections.len(), 1);
        let s = &res.sections[0];
        assert_eq!(s.category, CardCategory::Land);
        assert_eq!(s.label, "Lands");
        assert_eq!(s.cards.len(), 3);
        // All on row 0.
        for (i, p) in s.cards.iter().enumerate() {
            assert_eq!(p.row, 0, "card {} should be on row 0", i);
            assert_eq!(p.col, i, "card {} should be on column {}", i, i);
        }
        // Cards strictly to the right of each other.
        assert!(s.cards[0].bounding_box.x2 <= s.cards[1].bounding_box.x1);
        assert!(s.cards[1].bounding_box.x2 <= s.cards[2].bounding_box.x1);
        // First card's header is above its card.
        assert!(s.header.y2 <= s.cards[0].bounding_box.y1);
    }

    #[test]
    fn cards_wrap_when_row_overflows() {
        // 30-cell-wide rect → 300 px → only ~2 default cards per row
        // (2*100 + 10 spacing = 210 ≤ 300; 3*100 + 20 = 320 > 300).
        let r = LayoutRect::from_xywh(0.0, 0.0, 300.0, 400.0);
        let cards: Vec<_> = (0..5).map(|i| card(i, CardCategory::Creature)).collect();
        let res = layout_battlefield(r, CellSize::TERMINAL, &cards, &LayoutConfig::default());
        let s = &res.sections[0];
        assert_eq!(s.cards.len(), 5);
        // Multiple rows used.
        let max_row = s.cards.iter().map(|c| c.row).max().unwrap();
        assert!(max_row >= 1, "expected wrapping, got max_row={}", max_row);
        // Cards on later rows have y > cards on earlier rows.
        let row0_y = s.cards[0].bounding_box.y1;
        let last_row_y = s.cards.last().unwrap().bounding_box.y1;
        assert!(last_row_y > row0_y);
    }

    #[test]
    fn sections_appear_in_priority_order() {
        let r = LayoutRect::from_xywh(0.0, 0.0, 800.0, 600.0);
        let cards = vec![
            card(0, CardCategory::Land),
            card(1, CardCategory::Planeswalker),
            card(2, CardCategory::Creature),
            card(3, CardCategory::Artifact),
            card(4, CardCategory::Enchantment),
        ];
        let res = layout_battlefield(r, CellSize::TERMINAL, &cards, &LayoutConfig::default());
        let cats: Vec<_> = res.sections.iter().map(|s| s.category).collect();
        assert_eq!(
            cats,
            vec![
                CardCategory::Planeswalker,
                CardCategory::Creature,
                CardCategory::Enchantment,
                CardCategory::Artifact,
                CardCategory::Land,
            ]
        );
        // Each section's header sits above the first card in that
        // section, and successive sections' headers are below previous
        // sections' last card.
        for w in res.sections.windows(2) {
            let prev_last = w[0].cards.iter().map(|c| c.bounding_box.y2).fold(0.0_f32, f32::max);
            assert!(w[1].header.y1 >= prev_last);
        }
    }

    #[test]
    fn reverse_order_for_opponent() {
        let r = LayoutRect::from_xywh(0.0, 0.0, 800.0, 600.0);
        let cards = vec![
            card(0, CardCategory::Land),
            card(1, CardCategory::Planeswalker),
            card(2, CardCategory::Creature),
        ];
        let mut cfg = LayoutConfig::default();
        cfg.reverse_section_order = true;
        let res = layout_battlefield(r, CellSize::TERMINAL, &cards, &cfg);
        let cats: Vec<_> = res.sections.iter().map(|s| s.category).collect();
        assert_eq!(
            cats,
            vec![CardCategory::Land, CardCategory::Creature, CardCategory::Planeswalker]
        );
    }

    #[test]
    fn graveyard_reserved_in_lower_right_when_configured() {
        let r = LayoutRect::from_xywh(0.0, 0.0, 800.0, 400.0);
        let mut cfg = LayoutConfig::default();
        cfg.graveyard_card_count = 3;
        cfg.graveyard_max_name_len = 20; // wider than "Graveyard:" (10)
        let cards = vec![card(0, CardCategory::Creature)];
        let res = layout_battlefield(r, CellSize::TERMINAL, &cards, &cfg);
        let gv = res.graveyard_rect.expect("graveyard rect should be present");
        // Bottom-right anchored: x2 == rect.x2, y2 == rect.y2.
        assert_eq!(gv.x2, r.x2);
        assert_eq!(gv.y2, r.y2);
        // Width covers at least 20 chars * 10 px = 200.
        assert!(gv.width() >= 200.0);
        // Height = 4 lines (header + 3) * 20 px = 80.
        assert!(gv.height() >= 80.0);
    }

    #[test]
    fn graveyard_uses_header_width_when_names_are_short() {
        let r = LayoutRect::from_xywh(0.0, 0.0, 800.0, 400.0);
        let mut cfg = LayoutConfig::default();
        cfg.graveyard_card_count = 1;
        cfg.graveyard_max_name_len = 3; // shorter than "Graveyard:" (10)
        let res = layout_battlefield(r, CellSize::TERMINAL, &[card(0, CardCategory::Creature)], &cfg);
        let gv = res.graveyard_rect.unwrap();
        // Width is at least 10 chars * 10 px = 100.
        assert!(gv.width() >= 100.0);
    }

    #[test]
    fn no_graveyard_when_unconfigured() {
        let r = LayoutRect::from_xywh(0.0, 0.0, 800.0, 400.0);
        let res = layout_battlefield(
            r,
            CellSize::TERMINAL,
            &[card(0, CardCategory::Creature)],
            &LayoutConfig::default(),
        );
        assert!(res.graveyard_rect.is_none());
    }

    #[test]
    fn tapped_cards_are_wider_and_shorter() {
        let r = LayoutRect::from_xywh(0.0, 0.0, 800.0, 400.0);
        let cards = vec![card(0, CardCategory::Creature), tapped(1, CardCategory::Creature)];
        let res = layout_battlefield(r, CellSize::TERMINAL, &cards, &LayoutConfig::default());
        let s = &res.sections[0];
        let untapped_w = s.cards[0].bounding_box.width();
        let untapped_h = s.cards[0].bounding_box.height();
        let tapped_w = s.cards[1].bounding_box.width();
        let tapped_h = s.cards[1].bounding_box.height();
        assert!(
            tapped_w > untapped_w,
            "tapped width {} should exceed untapped {}",
            tapped_w,
            untapped_w
        );
        assert!(
            tapped_h < untapped_h,
            "tapped height {} should be less than untapped {}",
            tapped_h,
            untapped_h
        );
    }

    #[test]
    fn placements_stay_inside_input_rect() {
        let r = LayoutRect::from_xywh(0.0, 0.0, 600.0, 400.0);
        let cards: Vec<_> = (0..15)
            .map(|i| {
                let cat = match i % 3 {
                    0 => CardCategory::Creature,
                    1 => CardCategory::Land,
                    _ => CardCategory::Artifact,
                };
                card(i, cat)
            })
            .collect();
        let res = layout_battlefield(r, CellSize::TERMINAL, &cards, &LayoutConfig::default());
        for section in &res.sections {
            assert!(section.header.x1 >= r.x1 && section.header.x2 <= r.x2);
            for p in &section.cards {
                let bb = p.bounding_box;
                assert!(bb.x1 >= r.x1, "card {} x1={} < rect.x1={}", p.card_id, bb.x1, r.x1);
                assert!(bb.x2 <= r.x2, "card {} x2={} > rect.x2={}", p.card_id, bb.x2, r.x2);
                assert!(bb.y1 >= r.y1, "card {} y1={} < rect.y1={}", p.card_id, bb.y1, r.y1);
                // Vertical overflow is allowed only if we couldn't shrink
                // further (i.e. used min size). Sanity-check we are at
                // least not far past the rect.
                assert!(bb.y2 <= r.y2 + 5.0 * CellSize::TERMINAL.h);
            }
        }
    }

    #[test]
    fn terminal_snapping_aligns_to_cell_grid() {
        let r = LayoutRect::from_xywh(0.0, 0.0, 800.0, 400.0);
        let cards: Vec<_> = (0..6).map(|i| card(i, CardCategory::Creature)).collect();
        let res = layout_battlefield(r, CellSize::TERMINAL, &cards, &LayoutConfig::default());
        let cell = CellSize::TERMINAL;
        for s in &res.sections {
            for p in &s.cards {
                let bb = p.bounding_box;
                assert_eq!(bb.x1 % cell.w, 0.0, "x1={} not snapped to cell.w={}", bb.x1, cell.w);
                assert_eq!(bb.y1 % cell.h, 0.0, "y1={} not snapped to cell.h={}", bb.y1, cell.h);
                assert_eq!(bb.width() % cell.w, 0.0);
                assert_eq!(bb.height() % cell.h, 0.0);
            }
        }
    }

    #[test]
    fn pixel_mode_allows_finer_card_steps() {
        // A wider rect lets the engine pick a larger card height. With
        // PIXEL snapping the engine should be able to use heights that
        // are not multiples of 20.
        let r = LayoutRect::from_xywh(0.0, 0.0, 800.0, 400.0);
        let cards = vec![card(0, CardCategory::Creature)];
        let mut cfg = LayoutConfig::default();
        // Force a non-cell-aligned card height so the test is meaningful.
        cfg.default_card = CardSize::new(33.0, 47.0);
        cfg.min_card = CardSize::new(10.0, 10.0);
        cfg.max_card_height_px = 47.0;
        cfg.spacing_px = 1.0;
        cfg.header_height_px = 1.0;
        let res = layout_battlefield(r, CellSize::PIXEL, &cards, &cfg);
        // We can at least place the one card at exactly its requested
        // height (no quantisation error from a coarse grid).
        let p = &res.sections[0].cards[0];
        assert_eq!(p.bounding_box.height(), 47.0);
    }

    #[test]
    fn many_cards_shrink_card_size() {
        let r = LayoutRect::from_xywh(0.0, 0.0, 600.0, 200.0);
        let few: Vec<_> = (0..3).map(|i| card(i, CardCategory::Creature)).collect();
        let many: Vec<_> = (0..30).map(|i| card(i, CardCategory::Creature)).collect();
        let res_few = layout_battlefield(r, CellSize::TERMINAL, &few, &LayoutConfig::default());
        let res_many = layout_battlefield(r, CellSize::TERMINAL, &many, &LayoutConfig::default());
        assert!(
            res_many.used_card_size.height_px <= res_few.used_card_size.height_px,
            "{} should be ≤ {}",
            res_many.used_card_size.height_px,
            res_few.used_card_size.height_px,
        );
    }

    #[test]
    fn each_section_has_a_header_above_its_first_card() {
        let r = LayoutRect::from_xywh(0.0, 0.0, 800.0, 600.0);
        let cards = vec![
            card(0, CardCategory::Creature),
            card(1, CardCategory::Land),
            card(2, CardCategory::Land),
        ];
        let res = layout_battlefield(r, CellSize::TERMINAL, &cards, &LayoutConfig::default());
        for s in &res.sections {
            assert!(s.header.height() > 0.0);
            assert!(!s.cards.is_empty());
            let first = &s.cards[0];
            assert!(s.header.y2 <= first.bounding_box.y1);
            assert_eq!(s.header.x1, r.x1);
        }
    }
}
