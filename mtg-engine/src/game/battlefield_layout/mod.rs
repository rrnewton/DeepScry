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
//!
//! ## Module structure
//!
//! The engine is split by concept:
//! * [`geometry`] — value-type primitives (`LayoutRect`, `CellSize`, `Axis`).
//! * [`types`] — the public input / output data model (`CardCategory`,
//!   `CardLayoutInput`, `LayoutConfig`, `BattlefieldLayoutResult`, …).
//! * [`engine`] — the layout algorithm and its placement helpers, plus
//!   the public entry points.
//!
//! Every public item is re-exported here, so external callers continue to
//! reference `crate::game::battlefield_layout::{...}` unchanged.

mod engine;
mod geometry;
mod types;

pub use engine::{
    compute_centering_and_collision_offset, compute_graveyard_layout_rect, layout_battlefield,
    pick_card_size_for_battlefield, CardBoundsInput,
};
pub use geometry::{Axis, CellSize, LayoutRect};
pub use types::{
    BattlefieldLayoutResult, CardCategory, CardLayoutInput, CardPlacement, CardSize, LayoutConfig, SectionLayout,
};
