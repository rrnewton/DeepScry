//! Geometry primitives for the battlefield layout engine.
//!
//! Backend-neutral value types — an axis-aligned rectangle
//! ([`LayoutRect`]) in abstract pixel coordinates, the cell-snapping
//! granularity ([`CellSize`]) shared between the terminal and pixel
//! renderers, and the [`Axis`] selector used by the snap helpers. These
//! carry no game-state or rendering-backend dependencies.

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
