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
    /// When true, consecutive sections may share the same row provided
    /// the next section's first card still fits horizontally; otherwise
    /// every section starts on a fresh row. The TUI sets this to `true`
    /// to match its historical "flow with break only when needed"
    /// wrapping behaviour.
    pub flow_sections_on_same_row: bool,
    /// When true, every wrapped continuation row inside a section also
    /// reserves `header_height_px` of vertical space above its cards
    /// (matching the TUI's `1 + max_h + spacing` per-row vertical
    /// stride). When false, only the section's *first* row carries a
    /// header reservation — appropriate for tighter pixel-mode flows.
    pub reserve_header_per_row: bool,
    /// Weight multiplier applied to gaps between cards in *different*
    /// sections during per-row horizontal redistribution (the TUI's
    /// `SECTION_GAP_MULTIPLIER`). 1.0 disables the multiplier so all
    /// gaps share evenly. Has no effect when
    /// `redistribute_extra_horizontal` is false.
    pub section_gap_multiplier: f32,
    /// Minimum horizontal padding reserved on the left and right edges
    /// of a row during redistribution.
    pub min_edge_padding_px: f32,
    /// When true, after placement the engine redistributes any extra
    /// horizontal space across each row's edges and inter-card gaps.
    /// Off by default — opt-in for backends that want filled rows
    /// (e.g. the TUI's flow mode).
    pub redistribute_extra_horizontal: bool,
    /// When true, after placement the engine centres the resulting
    /// grid horizontally inside `rect`.
    pub center_horizontal: bool,
    /// When `Some(rect)`, after centring, any cards overlapping `rect`
    /// trigger a shared leftward slide of the entire grid until no
    /// card collides. Use this to keep the grid out from under the
    /// graveyard overlay (which the TUI draws on top of the
    /// battlefield in the lower right corner).
    ///
    /// The collision rectangle should be expressed in the same pixel
    /// coordinate space as `rect` (i.e. it is **not** automatically
    /// derived from `graveyard_card_count` — pass it explicitly so
    /// callers can opt out of collision handling per-frame).
    pub graveyard_collision_rect: Option<LayoutRect>,
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
            flow_sections_on_same_row: false,
            reserve_header_per_row: true,
            section_gap_multiplier: 1.5,
            min_edge_padding_px: cell.w,
            redistribute_extra_horizontal: false,
            center_horizontal: false,
            graveyard_collision_rect: None,
        }
    }
}

impl LayoutConfig {
    /// Preset matching the historical TUI behaviour exactly: sections
    /// flow on shared rows, every wrapped row reserves a 1-line header
    /// above its cards, rows redistribute extra horizontal space using
    /// the section-gap multiplier, and the resulting grid is centred
    /// horizontally inside the available rect.
    ///
    /// Callers should additionally set `graveyard_collision_rect` if a
    /// graveyard overlay needs to be dodged for the current frame.
    pub fn tui_compat() -> Self {
        Self {
            flow_sections_on_same_row: true,
            reserve_header_per_row: true,
            redistribute_extra_horizontal: true,
            center_horizontal: true,
            ..Self::default()
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
    let mut sections = place_sections(available, cell, &sections_in, used_card_size, config);

    // 6. Per-row horizontal redistribution (widens inter-card gaps,
    //    weighting inter-section gaps by section_gap_multiplier).
    if config.redistribute_extra_horizontal {
        redistribute_rows_horizontal(&mut sections, available, cell, config);
    }

    // 7. Single x-offset to centre the (possibly redistributed) grid
    //    inside `available`.
    if config.center_horizontal {
        let centre_offset = compute_centring_offset(&sections, available, cell);
        if centre_offset != 0.0 {
            shift_sections_x(&mut sections, centre_offset);
        }
    }

    // 8. If a collision rect is configured, slide the grid further left
    //    so no card overlaps it. This runs *after* centring because the
    //    TUI has historically only collided after applying the
    //    centre offset.
    if let Some(collision) = config.graveyard_collision_rect {
        let slide = compute_collision_slide(&sections, &collision);
        if slide > 0.0 {
            let snapped = cell.snap_ceil(slide, Axis::X);
            shift_sections_x(&mut sections, -snapped);
        }
    }

    BattlefieldLayoutResult {
        sections,
        graveyard_rect,
        used_card_size,
    }
}

/// Pick the largest card size that fits all `cards` inside `rect`,
/// without producing the full per-card placement.
///
/// This is the size-only fast-path used by callers that want to drive
/// their own placement (e.g. the TUI renderer, which then applies its
/// own per-row centring and graveyard-collision sliding) but still
/// want the layout engine to be the single source of truth for card
/// sizing decisions.
pub fn pick_card_size_for_battlefield(
    rect: LayoutRect,
    cell: CellSize,
    cards: &[CardLayoutInput],
    config: &LayoutConfig,
) -> CardSize {
    if cards.is_empty() || rect.is_empty() {
        return config.min_card;
    }
    // Mirror the same available-area shrink that `layout_battlefield`
    // does so the chosen size respects the graveyard reservation.
    let graveyard_rect = compute_graveyard_rect(rect, cell, config);
    let available = if let Some(gv) = graveyard_rect {
        LayoutRect::new(rect.x1, rect.y1, gv.x1.max(rect.x1), rect.y2)
    } else {
        rect
    };
    let sections_in = group_and_order(cards, config.reverse_section_order);
    if sections_in.is_empty() {
        return config.min_card;
    }
    pick_card_size(available, cell, &sections_in, config)
}

/// Public wrapper around the internal graveyard rect computation —
/// returns the bounding box reserved in the lower-right corner for
/// the graveyard text element, snapped to the cell grid.
///
/// Returns `None` when `LayoutConfig::graveyard_card_count == 0` or
/// the requested rect won't fit inside `rect`.
pub fn compute_graveyard_layout_rect(rect: LayoutRect, cell: CellSize, config: &LayoutConfig) -> Option<LayoutRect> {
    compute_graveyard_rect(rect, cell, config)
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

// ───────────────────────────────────────────────────────────────────────
// Trace-based row computation
//
// The placement algorithm is split into two passes:
//   1. `trace_layout` walks `sections` left-to-right, allocating row
//      slots and computing per-card x positions. It honours the two
//      modal flags (`flow_sections_on_same_row`, `reserve_header_per_row`).
//   2. `row_y_positions` then sums the row stride to assign every row
//      its absolute y coordinate.
//
// `simulate_height` calls both to compute total height for the size
// picker, and `place_sections` calls both to build the public
// `SectionLayout` output. This avoids the historical bug where the two
// codepaths drifted apart and produced different wrap decisions.
// ───────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
struct LayoutRow {
    /// Tallest card height on this row.
    max_h: f32,
    /// Whether this row reserves `header_height_px` above its cards.
    has_header: bool,
}

#[derive(Debug, Clone, Copy)]
struct CardTrace {
    row_idx: usize,
    /// Column within the row (resets at each row wrap, including
    /// section-induced new rows).
    col_idx: usize,
    /// Absolute x position (already snapped to the cell grid).
    x: f32,
    width: f32,
    height: f32,
    /// Index into the *parent section's* `cards` Vec — needed so the
    /// caller can recover `card_id` after row reordering.
    card_input_idx: usize,
}

#[derive(Debug, Clone, Copy)]
struct SectionTrace {
    header_row: usize,
    /// Absolute x of the section label rect (matches the x of the
    /// section's first card).
    header_x: f32,
}

struct LayoutTrace {
    rows: Vec<LayoutRow>,
    sections: Vec<SectionTrace>,
    /// One inner Vec per section, in input order.
    cards: Vec<Vec<CardTrace>>,
    /// True if any card was wider than the available rect.
    overflowed: bool,
}

fn trace_layout(
    available: LayoutRect,
    cell: CellSize,
    sections: &[SectionInput<'_>],
    base: CardSize,
    config: &LayoutConfig,
) -> LayoutTrace {
    let usable_w = available.width();
    let spacing_x = cell.snap_ceil(config.spacing_px, Axis::X);

    let mut rows: Vec<LayoutRow> = Vec::new();
    let mut section_traces: Vec<SectionTrace> = Vec::with_capacity(sections.len());
    let mut card_traces: Vec<Vec<CardTrace>> = Vec::with_capacity(sections.len());
    let mut overflowed = false;

    // Cursor state — `cur_row_idx == None` means "we have not opened a
    // row yet". `cur_x` is the x position of the *next* card on the
    // current row (i.e., past the last card already placed).
    let mut cur_row_idx: Option<usize> = None;
    let mut cur_x = available.x1;
    let mut col_idx = 0usize;

    for section in sections {
        // Width of the first card decides whether this section can flow
        // onto the current row.
        let first_card_w = section
            .cards
            .first()
            .map(|c| entity_size(c, base, cell).width_px)
            .unwrap_or(0.0);

        let needs_new_row = match cur_row_idx {
            None => true,
            Some(_) if !config.flow_sections_on_same_row => true,
            Some(_) => {
                // Try to share with the current row. The next card needs
                // either `first_card_w` (if cur_x == available.x1) or
                // `spacing + first_card_w` more horizontal room.
                let needed = if col_idx == 0 {
                    first_card_w
                } else {
                    spacing_x + first_card_w
                };
                cur_x + needed > available.x1 + usable_w
            }
        };
        if needs_new_row {
            rows.push(LayoutRow {
                max_h: 0.0,
                has_header: true, // section-start rows always reserve a header line
            });
            cur_row_idx = Some(rows.len() - 1);
            cur_x = available.x1;
            col_idx = 0;
        }

        let header_row = cur_row_idx.unwrap();
        // Header is anchored to the same x as the section's first card.
        // When flowing onto the current row, that x lies past the
        // inter-card spacing, not at the bare cursor position.
        let header_anchor_x = if col_idx == 0 { cur_x } else { cur_x + spacing_x };
        let header_x = cell.snap_floor(header_anchor_x, Axis::X);
        section_traces.push(SectionTrace { header_row, header_x });

        let mut this_section: Vec<CardTrace> = Vec::with_capacity(section.cards.len());
        for (input_idx, card) in section.cards.iter().enumerate() {
            let sz = entity_size(card, base, cell);
            if sz.width_px > usable_w {
                overflowed = true;
                continue;
            }
            // Horizontal fit check for *this* card.
            let needed = if col_idx == 0 {
                sz.width_px
            } else {
                spacing_x + sz.width_px
            };
            if col_idx > 0 && cur_x + needed > available.x1 + usable_w {
                // Wrap to a new row inside the section.
                rows.push(LayoutRow {
                    max_h: 0.0,
                    has_header: config.reserve_header_per_row,
                });
                cur_row_idx = Some(rows.len() - 1);
                cur_x = available.x1;
                col_idx = 0;
            }
            let card_x = if col_idx == 0 {
                cell.snap_floor(cur_x, Axis::X)
            } else {
                cell.snap_floor(cur_x + spacing_x, Axis::X)
            };
            let row_idx = cur_row_idx.unwrap();
            this_section.push(CardTrace {
                row_idx,
                col_idx,
                x: card_x,
                width: sz.width_px,
                height: sz.height_px,
                card_input_idx: input_idx,
            });
            cur_x = card_x + sz.width_px;
            rows[row_idx].max_h = rows[row_idx].max_h.max(sz.height_px);
            col_idx += 1;
        }
        card_traces.push(this_section);
    }

    LayoutTrace {
        rows,
        sections: section_traces,
        cards: card_traces,
        overflowed,
    }
}

/// Compute the absolute y position of every row in the trace.
/// Row stride is `(maybe header_h) + max_h + spacing_y` between rows.
fn row_y_positions(trace: &LayoutTrace, available: LayoutRect, cell: CellSize, config: &LayoutConfig) -> Vec<f32> {
    let header_h = cell.snap_ceil(config.header_height_px, Axis::Y);
    let spacing_y = cell.snap_ceil(config.spacing_px, Axis::Y);
    let mut out = Vec::with_capacity(trace.rows.len());
    let mut y = available.y1;
    for (i, row) in trace.rows.iter().enumerate() {
        if i > 0 {
            y += spacing_y;
        }
        if row.has_header {
            y += header_h;
        }
        out.push(cell.snap_floor(y, Axis::Y));
        y += row.max_h;
    }
    out
}

/// Total vertical extent of the laid-out content, or `None` if any card
/// failed to fit horizontally (caller should reject the candidate size).
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
    let trace = trace_layout(available, cell, sections, base, config);
    if trace.overflowed {
        return None;
    }
    if trace.rows.is_empty() {
        return Some(0.0);
    }
    let ys = row_y_positions(&trace, available, cell, config);
    let last = trace.rows.len() - 1;
    let bottom = ys[last] + trace.rows[last].max_h;
    Some((bottom - available.y1).max(0.0))
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
    let header_h = cell.snap_ceil(config.header_height_px, Axis::Y);
    let trace = trace_layout(available, cell, sections, base, config);
    let row_ys = row_y_positions(&trace, available, cell, config);

    let mut out = Vec::with_capacity(sections.len());
    for (sec_idx, section) in sections.iter().enumerate() {
        let st = trace.sections[sec_idx];
        // The header sits in the row's reserved 1-line space, directly
        // above the row's cards.
        let header_y = cell.snap_floor(row_ys[st.header_row] - header_h, Axis::Y);
        let label_w = section_label_width(section, cell);
        let header_rect = LayoutRect::new(st.header_x, header_y, st.header_x + label_w, header_y + header_h);

        // Section-local row index: each section's first card is row 0.
        let card_traces = &trace.cards[sec_idx];
        let base_row = card_traces.first().map(|c| c.row_idx).unwrap_or(0);

        let placements: Vec<CardPlacement> = card_traces
            .iter()
            .map(|c| {
                let y = row_ys[c.row_idx];
                CardPlacement {
                    card_id: section.cards[c.card_input_idx].card_id,
                    bounding_box: LayoutRect::new(c.x, y, c.x + c.width, y + c.height),
                    row: c.row_idx - base_row,
                    col: c.col_idx,
                }
            })
            .collect();

        out.push(SectionLayout {
            category: section.category,
            label: section.category.label(),
            header: header_rect,
            cards: placements,
        });
    }

    out
}

/// Width of a section's label rect in pixels: enough for `"{label}:"`
/// snapped up to the cell grid. Renderers that paint the label text
/// will use this so the rect approximately matches the painted text.
fn section_label_width(section: &SectionInput<'_>, cell: CellSize) -> f32 {
    let chars = section.category.label().len() + 1; // trailing ':'
    cell.snap_ceil((chars as f32) * cell.w, Axis::X)
}

// ───────────────────────────────────────────────────────────────────────
// Post-processing: redistribution, centring, collision avoidance
//
// These helpers run *after* `place_sections` produces the raw grid.
// They move card rectangles (and their headers) horizontally only —
// vertical positions are fixed by the row stride computed during
// placement.
//
// The TUI used to perform these passes inside its renderer; the
// migration into the layout engine ensures both backends (TUI and
// HTML/native GUI) get bit-identical positions when they opt in via
// `LayoutConfig::tui_compat()`.
// ───────────────────────────────────────────────────────────────────────

/// Lightweight reference to one card placement, plus its owning
/// section index (so we can detect inter-section gaps in row layout).
#[derive(Clone, Copy)]
struct CardRef {
    sec_idx: usize,
    /// Card index within its owning section.
    card_idx_within_section: usize,
    x: f32,
    width: f32,
}

/// Group every card placement by its row's `y1` coordinate (which is
/// shared by all cards on the same physical row). Sections may
/// contribute cards to *several* rows; we walk in section/card order to
/// preserve left-to-right ordering within a row.
fn rows_from_sections(sections: &[SectionLayout]) -> Vec<Vec<CardRef>> {
    use std::collections::BTreeMap;
    // BTreeMap<i64, Vec<CardRef>> keyed by quantised y so equal rows
    // always merge regardless of f32 representation noise.
    let mut by_y: BTreeMap<i64, Vec<CardRef>> = BTreeMap::new();
    for (sec_idx, section) in sections.iter().enumerate() {
        for (card_idx, p) in section.cards.iter().enumerate() {
            let key = p.bounding_box.y1.round() as i64;
            by_y.entry(key).or_default().push(CardRef {
                sec_idx,
                card_idx_within_section: card_idx,
                x: p.bounding_box.x1,
                width: p.bounding_box.width(),
            });
        }
    }
    // Sort each row by x so per-row distribution sees cards in
    // visual order, not insertion order.
    let mut rows: Vec<Vec<CardRef>> = by_y.into_values().collect();
    for row in &mut rows {
        row.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap_or(Ordering::Equal));
    }
    rows
}

/// Apply a horizontal redistribution pass. For each row, compute the
/// extra room past `min_edge_padding_px * 2 + row_width` and spread it
/// across (left edge, inter-card gaps, right edge), weighting
/// inter-section gaps by `section_gap_multiplier`.
fn redistribute_rows_horizontal(
    sections: &mut [SectionLayout],
    available: LayoutRect,
    cell: CellSize,
    config: &LayoutConfig,
) {
    if sections.is_empty() || available.width() <= 0.0 {
        return;
    }
    let rows = rows_from_sections(sections);
    let edge_pad = config.min_edge_padding_px.max(0.0);
    for row in &rows {
        if row.is_empty() {
            continue;
        }
        let first_x = row.first().unwrap().x;
        let last = row.last().unwrap();
        let row_width = (last.x + last.width) - first_x;
        let row_extra = (available.width() - row_width - edge_pad * 2.0).max(0.0);
        if row_extra <= 0.0 {
            continue;
        }
        // Total weight: 1.0 (left edge) + 1.0 (right edge) + per-gap weight.
        let mut total_weight: f32 = 2.0;
        for w in row.windows(2) {
            total_weight += if w[0].sec_idx == w[1].sec_idx {
                1.0
            } else {
                config.section_gap_multiplier.max(1.0)
            };
        }
        if total_weight <= 0.0 {
            continue;
        }
        let extra_per_unit = row_extra / total_weight;
        if extra_per_unit < 0.5 {
            continue;
        }
        // Snap each per-unit advance independently so we never advance
        // by a sub-cell amount on the TUI grid (matches the TUI's
        // `.round() as u16` behaviour).
        let snap_x = |v: f32| cell.snap_floor(v.max(0.0), Axis::X);
        let left_edge_extra = snap_x(extra_per_unit);
        let mut cumulative = left_edge_extra;
        let first = row[0];
        shift_card_x_in_section(sections, first.sec_idx, first.card_idx_within_section, cumulative);
        retarget_header_to_first_row_card(sections, first.sec_idx, first.card_idx_within_section);
        for w in row.windows(2) {
            let gap_weight = if w[0].sec_idx == w[1].sec_idx {
                1.0
            } else {
                config.section_gap_multiplier.max(1.0)
            };
            cumulative += snap_x(extra_per_unit * gap_weight);
            shift_card_x_in_section(sections, w[1].sec_idx, w[1].card_idx_within_section, cumulative);
            // If we just crossed into a new section AND this is that
            // section's first card, retarget the section's header so
            // it stays anchored above the redistributed first card.
            if w[0].sec_idx != w[1].sec_idx {
                retarget_header_to_first_row_card(sections, w[1].sec_idx, w[1].card_idx_within_section);
            }
        }
    }
}

/// If `(sec_idx, card_idx)` is the first card in its section, move
/// that section's header rect so its `x1` matches the card's `x1`.
/// (When sections flow on the same row, the header rides above the
/// section's first card — keeping them aligned during redistribution
/// preserves the TUI's "label sits above its cards" invariant.)
fn retarget_header_to_first_row_card(sections: &mut [SectionLayout], sec_idx: usize, card_idx: usize) {
    if card_idx != 0 {
        return;
    }
    let new_x = sections[sec_idx].cards[card_idx].bounding_box.x1;
    let header = &mut sections[sec_idx].header;
    let w = header.width();
    header.x1 = new_x;
    header.x2 = new_x + w;
}

/// Shift one card's bounding box by `dx` (positive moves right).
fn shift_card_x_in_section(sections: &mut [SectionLayout], sec_idx: usize, card_idx: usize, dx: f32) {
    if dx == 0.0 {
        return;
    }
    let p = &mut sections[sec_idx].cards[card_idx];
    p.bounding_box.x1 += dx;
    p.bounding_box.x2 += dx;
}

/// Compute how much to shift the *whole* grid horizontally so it sits
/// centred inside `available`. Headers are included in the bounds
/// calculation so an extra-wide label doesn't get clipped off-screen
/// after centring.
fn compute_centring_offset(sections: &[SectionLayout], available: LayoutRect, cell: CellSize) -> f32 {
    let mut min_x = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    for s in sections {
        if s.header.width() > 0.0 {
            min_x = min_x.min(s.header.x1);
            max_x = max_x.max(s.header.x2);
        }
        for p in &s.cards {
            min_x = min_x.min(p.bounding_box.x1);
            max_x = max_x.max(p.bounding_box.x2);
        }
    }
    if !min_x.is_finite() || !max_x.is_finite() {
        return 0.0;
    }
    let grid_width = max_x - min_x;
    if grid_width >= available.width() {
        return 0.0;
    }
    let ideal_x1 = available.x1 + (available.width() - grid_width) / 2.0;
    let dx = ideal_x1 - min_x;
    cell.snap_floor(dx.max(0.0), Axis::X)
}

/// Translate every section header + card by `dx` (positive ⇒ right).
fn shift_sections_x(sections: &mut [SectionLayout], dx: f32) {
    if dx == 0.0 {
        return;
    }
    for s in sections {
        s.header.x1 += dx;
        s.header.x2 += dx;
        for p in &mut s.cards {
            p.bounding_box.x1 += dx;
            p.bounding_box.x2 += dx;
        }
    }
}

/// Largest leftward slide (positive number) needed to keep every card
/// out of the supplied collision rectangle. Headers are intentionally
/// *not* checked — they sit above the cards so they don't conflict
/// with the bottom-right graveyard overlay in practice.
///
/// Returns `0.0` when no collision exists.
fn compute_collision_slide(sections: &[SectionLayout], collision: &LayoutRect) -> f32 {
    let mut max_slide = 0.0_f32;
    for s in sections {
        for p in &s.cards {
            if p.bounding_box.intersects(collision) {
                let slide = p.bounding_box.x2 - collision.x1;
                if slide > max_slide {
                    max_slide = slide;
                }
            }
        }
    }
    max_slide
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
        let cfg = LayoutConfig {
            reverse_section_order: true,
            ..LayoutConfig::default()
        };
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
        let cfg = LayoutConfig {
            graveyard_card_count: 3,
            graveyard_max_name_len: 20, // wider than "Graveyard:" (10)
            ..LayoutConfig::default()
        };
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
        let cfg = LayoutConfig {
            graveyard_card_count: 1,
            graveyard_max_name_len: 3, // shorter than "Graveyard:" (10)
            ..LayoutConfig::default()
        };
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
        // Force a non-cell-aligned card height so the test is meaningful.
        let cfg = LayoutConfig {
            default_card: CardSize::new(33.0, 47.0),
            min_card: CardSize::new(10.0, 10.0),
            max_card_height_px: 47.0,
            spacing_px: 1.0,
            header_height_px: 1.0,
            ..LayoutConfig::default()
        };
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

    // ─── Flow mode + per-row header behaviour (TUI compat) ───────────

    #[test]
    fn tui_compat_default_enables_flow_and_per_row_headers() {
        let cfg = LayoutConfig::tui_compat();
        assert!(cfg.flow_sections_on_same_row);
        assert!(cfg.reserve_header_per_row);
    }

    #[test]
    fn flow_mode_lets_two_small_sections_share_a_row() {
        // Wide rect: 800 px; default cards 100 px wide.
        // Section A: 1 creature; Section B: 1 land. Combined: 100 + 10
        // (spacing) + 100 = 210, fits easily on a 800-px row.
        let r = LayoutRect::from_xywh(0.0, 0.0, 800.0, 400.0);
        let cards = vec![card(0, CardCategory::Creature), card(1, CardCategory::Land)];
        let cfg = LayoutConfig::tui_compat();
        let res = layout_battlefield(r, CellSize::TERMINAL, &cards, &cfg);
        assert_eq!(res.sections.len(), 2);
        let row_a = res.sections[0].cards[0].bounding_box.y1;
        let row_b = res.sections[1].cards[0].bounding_box.y1;
        assert_eq!(
            row_a, row_b,
            "in flow mode the two single-card sections should share a row (y={} vs {})",
            row_a, row_b
        );
        // The second section's header sits above its card, not at x=0.
        assert!(res.sections[1].header.x1 > res.sections[0].header.x1);
        assert_eq!(res.sections[1].header.y1, res.sections[0].header.y1);
    }

    #[test]
    fn flow_mode_breaks_when_section_does_not_fit() {
        // Narrow rect: 250 px. Default card = 100 px wide.
        // Section A (1 creature) at x=0..100. Section B (1 land) needs
        // 100 + 10 = 110 more → would land at x=210, fits.
        // Force a break by adding a third section that pushes us over.
        let r = LayoutRect::from_xywh(0.0, 0.0, 250.0, 400.0);
        let cards = vec![
            card(0, CardCategory::Creature), // x = 0
            card(1, CardCategory::Land),     // x = 110 (flows)
            card(2, CardCategory::Artifact), // would be x = 220+ → wraps
        ];
        let cfg = LayoutConfig::tui_compat();
        let res = layout_battlefield(r, CellSize::TERMINAL, &cards, &cfg);
        // Order is PWs/Creatures/Enchants/Artifacts/Lands → so:
        //   section[0] = Creature, section[1] = Artifact, section[2] = Land
        let creature_y = res.sections[0].cards[0].bounding_box.y1;
        let artifact_y = res.sections[1].cards[0].bounding_box.y1;
        let land_y = res.sections[2].cards[0].bounding_box.y1;
        assert_eq!(creature_y, res.sections[0].header.y2.max(creature_y));
        // Creature (1 card, 100 px) + Artifact (1 card, +110 = 210 px) fits.
        // Land (+110 = 320 px) does NOT fit on row 0 → wraps.
        assert_eq!(creature_y, artifact_y, "Creature and Artifact should share row 0");
        assert!(
            land_y > creature_y,
            "Land should be on a later row (y={} > {})",
            land_y,
            creature_y
        );
    }

    #[test]
    fn per_row_header_reservation_changes_chosen_card_size() {
        // Set up a height-constrained rect where the per-row header
        // reservation tips the size picker into a smaller card.
        // 30 cells wide × 14 cells tall. 5 default cards (10×7) wrap to
        // 2 rows.
        //   reserve_header_per_row=false → row stride = 7 + 1 = 8 cells;
        //                                  total = 1 (hdr) + 7 + 1 + 7 = 16 > 14 → shrink.
        //   reserve_header_per_row=true  → row stride = 1 + 7 + 1 = 9 cells;
        //                                  total = 1 + 7 + 1 + 1 + 7 = 17 > 14 → shrink more.
        let r = LayoutRect::from_xywh(0.0, 0.0, 300.0, 280.0); // 30 × 14 cells
        let cards: Vec<_> = (0..5).map(|i| card(i, CardCategory::Creature)).collect();
        let mut cfg_no = LayoutConfig::tui_compat();
        cfg_no.reserve_header_per_row = false;
        let cfg_yes = LayoutConfig::tui_compat();
        let res_no = layout_battlefield(r, CellSize::TERMINAL, &cards, &cfg_no);
        let res_yes = layout_battlefield(r, CellSize::TERMINAL, &cards, &cfg_yes);
        assert!(
            res_yes.used_card_size.height_px <= res_no.used_card_size.height_px,
            "per-row header reservation should produce ≤ card height ({} vs {})",
            res_yes.used_card_size.height_px,
            res_no.used_card_size.height_px,
        );
    }

    #[test]
    fn flow_mode_is_independent_of_priority_order() {
        // In flow mode with reverse_section_order, sections still flow
        // — but in their reversed order.
        let r = LayoutRect::from_xywh(0.0, 0.0, 800.0, 400.0);
        let cards = vec![card(0, CardCategory::Land), card(1, CardCategory::Creature)];
        let mut cfg = LayoutConfig::tui_compat();
        cfg.reverse_section_order = true;
        let res = layout_battlefield(r, CellSize::TERMINAL, &cards, &cfg);
        assert_eq!(res.sections[0].category, CardCategory::Land);
        assert_eq!(res.sections[1].category, CardCategory::Creature);
        // Both share row 0.
        assert_eq!(
            res.sections[0].cards[0].bounding_box.y1,
            res.sections[1].cards[0].bounding_box.y1
        );
    }

    #[test]
    fn header_x_matches_first_cards_x_in_flow_mode() {
        let r = LayoutRect::from_xywh(0.0, 0.0, 800.0, 400.0);
        let cards = vec![card(0, CardCategory::Creature), card(1, CardCategory::Land)];
        let cfg = LayoutConfig::tui_compat();
        let res = layout_battlefield(r, CellSize::TERMINAL, &cards, &cfg);
        for s in &res.sections {
            let first_x = s.cards[0].bounding_box.x1;
            assert_eq!(
                s.header.x1, first_x,
                "section {} header.x={} should match first card x={}",
                s.label, s.header.x1, first_x
            );
        }
    }

    #[test]
    fn section_local_row_indices_are_zero_based_per_section() {
        // Section A wraps internally → its rows should be 0, 1, ...
        // Section B that follows on a later row → its rows should *also*
        // start at 0 (locally), not continue counting.
        let r = LayoutRect::from_xywh(0.0, 0.0, 250.0, 800.0); // narrow, tall
        let cards: Vec<_> = (0..6)
            .map(|i| {
                let cat = if i < 3 {
                    CardCategory::Creature
                } else {
                    CardCategory::Land
                };
                card(i, cat)
            })
            .collect();
        let cfg = LayoutConfig::tui_compat();
        let res = layout_battlefield(r, CellSize::TERMINAL, &cards, &cfg);
        for s in &res.sections {
            let first_row = s.cards[0].row;
            assert_eq!(first_row, 0, "section {} first card should be on local row 0", s.label);
        }
    }

    // ─── Post-processing: redistribution / centring / collision ─────

    #[test]
    fn centring_offset_is_zero_when_disabled() {
        // Default config has center_horizontal = false. With one card
        // way left of the rect's centre, no shift should occur.
        let r = LayoutRect::from_xywh(0.0, 0.0, 800.0, 400.0);
        let cards = vec![card(0, CardCategory::Creature)];
        let mut cfg = LayoutConfig::default();
        cfg.center_horizontal = false;
        cfg.redistribute_extra_horizontal = false;
        let res = layout_battlefield(r, CellSize::TERMINAL, &cards, &cfg);
        // With centring disabled the single card sits flush against x = 0.
        assert_eq!(res.sections[0].cards[0].bounding_box.x1, 0.0);
    }

    #[test]
    fn centring_shifts_grid_to_middle_of_rect() {
        // One small card in a wide rect: with centring on, it should
        // sit roughly in the middle of the rect.
        let r = LayoutRect::from_xywh(0.0, 0.0, 800.0, 400.0);
        let cards = vec![card(0, CardCategory::Creature)];
        let mut cfg = LayoutConfig::tui_compat();
        // Disable redistribution so we measure pure centring.
        cfg.redistribute_extra_horizontal = false;
        let res = layout_battlefield(r, CellSize::TERMINAL, &cards, &cfg);
        let bb = res.sections[0].cards[0].bounding_box;
        let card_centre = (bb.x1 + bb.x2) / 2.0;
        // Allow ±1 cell tolerance (snap_floor in the offset).
        assert!(
            (card_centre - 400.0).abs() <= CellSize::TERMINAL.w,
            "card centre {} should be ≈ 400 (rect centre)",
            card_centre
        );
    }

    #[test]
    fn redistribution_widens_gaps_to_fill_row() {
        // 3 cards on a 800-px row. Default sizing → 100 px each →
        // 300 px of cards, leaving ~480 px of slack distributed
        // across edges + gaps.
        let r = LayoutRect::from_xywh(0.0, 0.0, 800.0, 400.0);
        let cards: Vec<_> = (0..3).map(|i| card(i, CardCategory::Creature)).collect();

        let mut cfg_no = LayoutConfig::tui_compat();
        cfg_no.redistribute_extra_horizontal = false;
        cfg_no.center_horizontal = false;
        let mut cfg_yes = cfg_no.clone();
        cfg_yes.redistribute_extra_horizontal = true;

        let res_no = layout_battlefield(r, CellSize::TERMINAL, &cards, &cfg_no);
        let res_yes = layout_battlefield(r, CellSize::TERMINAL, &cards, &cfg_yes);

        let gap_no = res_no.sections[0].cards[1].bounding_box.x1 - res_no.sections[0].cards[0].bounding_box.x2;
        let gap_yes = res_yes.sections[0].cards[1].bounding_box.x1 - res_yes.sections[0].cards[0].bounding_box.x2;

        assert!(
            gap_yes > gap_no,
            "redistribution should widen inter-card gap ({} → {})",
            gap_no,
            gap_yes,
        );
    }

    #[test]
    fn redistribution_weights_inter_section_gap_more() {
        // Two sections sharing one row. Inter-section gap should be
        // wider than the intra-section gap by the multiplier.
        let r = LayoutRect::from_xywh(0.0, 0.0, 800.0, 400.0);
        let cards = vec![
            card(0, CardCategory::Creature),
            card(1, CardCategory::Creature),
            // New section starts here:
            card(2, CardCategory::Land),
            card(3, CardCategory::Land),
        ];
        let mut cfg = LayoutConfig::tui_compat();
        cfg.section_gap_multiplier = 3.0; // Exaggerate so the assert is robust.
        cfg.center_horizontal = false; // Isolate from centring.
        let res = layout_battlefield(r, CellSize::TERMINAL, &cards, &cfg);
        // section 0 (Creature): 2 cards, section 1 (Land): 2 cards.
        let intra = res.sections[0].cards[1].bounding_box.x1 - res.sections[0].cards[0].bounding_box.x2;
        let inter = res.sections[1].cards[0].bounding_box.x1 - res.sections[0].cards[1].bounding_box.x2;
        assert!(
            inter > intra,
            "inter-section gap {} should exceed intra-section gap {}",
            inter,
            intra,
        );
    }

    #[test]
    fn collision_rect_slides_grid_left() {
        // Wide rect, one card, with centring enabled. Then add a
        // collision rect that overlaps where the centred card would
        // sit. The result must shift further left so no card touches
        // the collision rect.
        let r = LayoutRect::from_xywh(0.0, 0.0, 800.0, 400.0);
        let cards = vec![card(0, CardCategory::Creature)];

        let mut cfg = LayoutConfig::tui_compat();
        cfg.redistribute_extra_horizontal = false; // simpler reasoning
                                                   // Without collision, the card centres around x ≈ 350..450.
        let res_no_collision = layout_battlefield(r, CellSize::TERMINAL, &cards, &cfg);
        let centred_x2 = res_no_collision.sections[0].cards[0].bounding_box.x2;

        // Now add a collision rect that begins *before* the centred
        // card's right edge.
        cfg.graveyard_collision_rect = Some(LayoutRect::from_xywh(centred_x2 - 50.0, 0.0, 200.0, 400.0));
        let res = layout_battlefield(r, CellSize::TERMINAL, &cards, &cfg);
        let collided_x2 = res.sections[0].cards[0].bounding_box.x2;

        assert!(
            collided_x2 <= centred_x2 - 50.0,
            "collision should slide grid left so x2 ({}) ≤ collision.x1 ({})",
            collided_x2,
            centred_x2 - 50.0,
        );
        // And nothing should overlap the collision rect.
        let collision = cfg.graveyard_collision_rect.unwrap();
        for s in &res.sections {
            for p in &s.cards {
                assert!(
                    !p.bounding_box.intersects(&collision),
                    "card {} bb {:?} still intersects collision {:?}",
                    p.card_id,
                    p.bounding_box,
                    collision,
                );
            }
        }
    }

    #[test]
    fn collision_rect_no_op_when_no_overlap() {
        // Collision rect placed far below the cards — no slide should
        // happen.
        let r = LayoutRect::from_xywh(0.0, 0.0, 800.0, 400.0);
        let cards = vec![card(0, CardCategory::Creature)];
        let mut cfg = LayoutConfig::tui_compat();
        cfg.graveyard_collision_rect = Some(LayoutRect::from_xywh(700.0, 380.0, 100.0, 20.0));
        let res = layout_battlefield(r, CellSize::TERMINAL, &cards, &cfg);
        // Should still be roughly centred (within ±1 cell).
        let bb = res.sections[0].cards[0].bounding_box;
        let centre = (bb.x1 + bb.x2) / 2.0;
        assert!(
            (centre - 400.0).abs() <= CellSize::TERMINAL.w,
            "no collision → still centred (centre={})",
            centre
        );
    }

    #[test]
    fn redistribution_keeps_header_above_first_card() {
        // After redistribution the first card of each section moves
        // right; the section's header must move with it so the label
        // continues to sit directly above its cards.
        let r = LayoutRect::from_xywh(0.0, 0.0, 800.0, 400.0);
        let cards = vec![card(0, CardCategory::Creature), card(1, CardCategory::Land)];
        let cfg = LayoutConfig::tui_compat(); // redistribute + centre
        let res = layout_battlefield(r, CellSize::TERMINAL, &cards, &cfg);
        for s in &res.sections {
            assert_eq!(
                s.header.x1, s.cards[0].bounding_box.x1,
                "section {} header.x={} should track first card x={}",
                s.label, s.header.x1, s.cards[0].bounding_box.x1
            );
        }
    }

    #[test]
    fn placements_remain_inside_rect_after_post_processing() {
        // With centring + redistribution + a collision rect, no card
        // should escape the original input rectangle.
        let r = LayoutRect::from_xywh(0.0, 0.0, 800.0, 400.0);
        let cards: Vec<_> = (0..6)
            .map(|i| {
                card(
                    i,
                    if i % 2 == 0 {
                        CardCategory::Creature
                    } else {
                        CardCategory::Land
                    },
                )
            })
            .collect();
        let mut cfg = LayoutConfig::tui_compat();
        cfg.graveyard_collision_rect = Some(LayoutRect::from_xywh(700.0, 200.0, 100.0, 200.0));
        let res = layout_battlefield(r, CellSize::TERMINAL, &cards, &cfg);
        for s in &res.sections {
            for p in &s.cards {
                let bb = p.bounding_box;
                assert!(
                    bb.x1 >= r.x1 - 0.001 && bb.x2 <= r.x2 + 0.001,
                    "card {} bb {:?} escaped rect {:?}",
                    p.card_id,
                    bb,
                    r,
                );
            }
        }
    }
}
