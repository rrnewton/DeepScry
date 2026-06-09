//! Public data model for the battlefield layout engine.
//!
//! These are the backend-neutral input and output types: card
//! categorisation ([`CardCategory`]), the per-card layout descriptor
//! ([`CardLayoutInput`]), the tunables ([`LayoutConfig`]), and the
//! computed placement result ([`BattlefieldLayoutResult`] and its
//! constituents). The layout algorithm itself lives in [`super::engine`].

use super::geometry::{CellSize, LayoutRect};

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
