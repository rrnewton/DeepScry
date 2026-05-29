//! Efficient bitfield representation of mana color sets
//!
//! This module provides a compact bitfield representation for sets of mana colors,
//! replacing Vec<ManaColor> with a single u8 value. This eliminates heap allocations
//! and provides O(1) operations for all set operations.
//!
//! # Performance Benefits
//!
//! - **Zero allocations**: Stored inline, no heap allocation needed
//! - **Compact**: 1 byte vs 24+ bytes for Vec<ManaColor>
//! - **Fast operations**: All operations are simple bit manipulations
//! - **Copy semantics**: Trivially copyable, no need for cloning
//!
//! # Example
//!
//! ```
//! use mtg_engine::core::ManaColor;
//! use mtg_engine::game::mana_colors::ManaColors;
//!
//! // Create a set with Red and Green (dual land like Taiga)
//! let taiga = ManaColors::new()
//!     .with(ManaColor::Red)
//!     .with(ManaColor::Green);
//!
//! assert_eq!(taiga.len(), 2);
//! assert!(taiga.contains(ManaColor::Red));
//! assert!(taiga.contains(ManaColor::Green));
//! assert!(!taiga.contains(ManaColor::Blue));
//!
//! // Iterate over colors
//! for color in taiga.iter() {
//!     println!("{:?}", color);
//! }
//! ```

use crate::core::ManaColor;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Efficient bitfield representation of a set of mana colors
///
/// Uses a single u8 with 5 bits (one per color: W, U, B, R, G).
/// This is Copy, requires no allocation, and supports all common set operations.
///
/// # Bit Layout
///
/// ```text
/// Bit 0: White  (W)
/// Bit 1: Blue   (U)
/// Bit 2: Black  (B)
/// Bit 3: Red    (R)
/// Bit 4: Green  (G)
/// Bits 5-7: Unused (reserved for future colors or flags)
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct ManaColors {
    bits: u8,
}

impl ManaColors {
    // Bit positions for each color
    const WHITE_BIT: u8 = 1 << 0;
    const BLUE_BIT: u8 = 1 << 1;
    const BLACK_BIT: u8 = 1 << 2;
    const RED_BIT: u8 = 1 << 3;
    const GREEN_BIT: u8 = 1 << 4;

    /// Create an empty set of colors
    #[inline]
    pub const fn new() -> Self {
        Self { bits: 0 }
    }

    /// Create a set containing a single color
    #[inline]
    pub const fn single(color: ManaColor) -> Self {
        Self {
            bits: Self::color_to_bit(color),
        }
    }

    /// Create a set from a slice of colors
    ///
    /// This is useful for converting from Vec<ManaColor> during migration.
    pub fn from_slice(colors: &[ManaColor]) -> Self {
        let mut result = Self::new();
        for &color in colors {
            result = result.with(color);
        }
        result
    }

    /// Convert a color to its bit representation
    #[inline]
    const fn color_to_bit(color: ManaColor) -> u8 {
        match color {
            ManaColor::White => Self::WHITE_BIT,
            ManaColor::Blue => Self::BLUE_BIT,
            ManaColor::Black => Self::BLACK_BIT,
            ManaColor::Red => Self::RED_BIT,
            ManaColor::Green => Self::GREEN_BIT,
        }
    }

    /// Add a color to this set (builder pattern)
    #[inline]
    pub const fn with(self, color: ManaColor) -> Self {
        Self {
            bits: self.bits | Self::color_to_bit(color),
        }
    }

    /// Add a color to this set (mutating)
    #[inline]
    pub fn insert(&mut self, color: ManaColor) {
        self.bits |= Self::color_to_bit(color);
    }

    /// Remove a color from this set
    #[inline]
    pub fn remove(&mut self, color: ManaColor) {
        self.bits &= !Self::color_to_bit(color);
    }

    /// Check if this set contains a color
    #[inline]
    pub const fn contains(&self, color: ManaColor) -> bool {
        self.bits & Self::color_to_bit(color) != 0
    }

    /// Check if this set is empty
    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.bits == 0
    }

    /// Get the number of colors in this set
    #[inline]
    pub const fn len(&self) -> usize {
        self.bits.count_ones() as usize
    }

    /// Iterate over the colors in this set
    pub fn iter(&self) -> ManaColorsIter {
        ManaColorsIter {
            bits: self.bits,
            pos: 0,
        }
    }

    /// Check if this set is a subset of another
    #[inline]
    pub const fn is_subset_of(&self, other: &Self) -> bool {
        (self.bits & other.bits) == self.bits
    }

    /// Union of two color sets
    #[inline]
    pub const fn union(&self, other: &Self) -> Self {
        Self {
            bits: self.bits | other.bits,
        }
    }

    /// Intersection of two color sets
    #[inline]
    pub const fn intersection(&self, other: &Self) -> Self {
        Self {
            bits: self.bits & other.bits,
        }
    }

    /// Get the raw bitfield value (for debugging/serialization)
    #[inline]
    pub const fn as_bits(&self) -> u8 {
        self.bits
    }

    /// Create from raw bitfield value (for deserialization)
    #[inline]
    pub const fn from_bits(bits: u8) -> Self {
        Self { bits: bits & 0x1F } // Mask to only valid bits
    }
}

/// Iterator over colors in a ManaColors set
pub struct ManaColorsIter {
    bits: u8,
    pos: u8,
}

impl Iterator for ManaColorsIter {
    type Item = ManaColor;

    fn next(&mut self) -> Option<Self::Item> {
        // Find the next set bit
        while self.pos < 5 {
            let bit = 1 << self.pos;
            self.pos += 1;
            if self.bits & bit != 0 {
                // Convert bit position to color
                return Some(match bit {
                    ManaColors::WHITE_BIT => ManaColor::White,
                    ManaColors::BLUE_BIT => ManaColor::Blue,
                    ManaColors::BLACK_BIT => ManaColor::Black,
                    ManaColors::RED_BIT => ManaColor::Red,
                    ManaColors::GREEN_BIT => ManaColor::Green,
                    _ => unreachable!(),
                });
            }
        }
        None
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = (self.bits >> self.pos).count_ones() as usize;
        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for ManaColorsIter {
    fn len(&self) -> usize {
        (self.bits >> self.pos).count_ones() as usize
    }
}

impl fmt::Display for ManaColors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_empty() {
            return write!(f, "{{}}");
        }

        write!(f, "{{")?;
        let mut first = true;
        for color in self.iter() {
            if !first {
                write!(f, ", ")?;
            }
            write!(f, "{}", color.to_char())?;
            first = false;
        }
        write!(f, "}}")
    }
}

impl std::iter::FromIterator<ManaColor> for ManaColors {
    fn from_iter<I: IntoIterator<Item = ManaColor>>(iter: I) -> Self {
        let mut result = Self::new();
        for color in iter {
            result = result.with(color);
        }
        result
    }
}

// Convenience constructors for common combinations
impl ManaColors {
    /// All five colors (WUBRG)
    pub const fn all() -> Self {
        Self {
            bits: Self::WHITE_BIT | Self::BLUE_BIT | Self::BLACK_BIT | Self::RED_BIT | Self::GREEN_BIT,
        }
    }

    /// Allied colors (WU, UB, BR, RG, GW)
    pub const fn allied_pair(color1: ManaColor, color2: ManaColor) -> Self {
        Self {
            bits: Self::color_to_bit(color1) | Self::color_to_bit(color2),
        }
    }

    /// Common dual land combinations
    pub const fn taiga() -> Self {
        Self {
            bits: Self::RED_BIT | Self::GREEN_BIT,
        }
    }

    pub const fn savannah() -> Self {
        Self {
            bits: Self::GREEN_BIT | Self::WHITE_BIT,
        }
    }

    pub const fn underground_sea() -> Self {
        Self {
            bits: Self::BLUE_BIT | Self::BLACK_BIT,
        }
    }

    pub const fn badlands() -> Self {
        Self {
            bits: Self::BLACK_BIT | Self::RED_BIT,
        }
    }

    pub const fn tundra() -> Self {
        Self {
            bits: Self::WHITE_BIT | Self::BLUE_BIT,
        }
    }

    pub const fn tropical_island() -> Self {
        Self {
            bits: Self::GREEN_BIT | Self::BLUE_BIT,
        }
    }

    pub const fn volcanic_island() -> Self {
        Self {
            bits: Self::BLUE_BIT | Self::RED_BIT,
        }
    }

    pub const fn bayou() -> Self {
        Self {
            bits: Self::BLACK_BIT | Self::GREEN_BIT,
        }
    }

    pub const fn scrubland() -> Self {
        Self {
            bits: Self::WHITE_BIT | Self::BLACK_BIT,
        }
    }

    pub const fn plateau() -> Self {
        Self {
            bits: Self::RED_BIT | Self::WHITE_BIT,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty() {
        let colors = ManaColors::new();
        assert!(colors.is_empty());
        assert_eq!(colors.len(), 0);
    }

    #[test]
    fn test_single_color() {
        let red = ManaColors::single(ManaColor::Red);
        assert!(!red.is_empty());
        assert_eq!(red.len(), 1);
        assert!(red.contains(ManaColor::Red));
        assert!(!red.contains(ManaColor::Blue));
    }

    #[test]
    fn test_builder_pattern() {
        let taiga = ManaColors::new().with(ManaColor::Red).with(ManaColor::Green);

        assert_eq!(taiga.len(), 2);
        assert!(taiga.contains(ManaColor::Red));
        assert!(taiga.contains(ManaColor::Green));
        assert!(!taiga.contains(ManaColor::Blue));
    }

    #[test]
    fn test_insert_and_remove() {
        let mut colors = ManaColors::new();
        colors.insert(ManaColor::White);
        colors.insert(ManaColor::Blue);
        assert_eq!(colors.len(), 2);

        colors.remove(ManaColor::White);
        assert_eq!(colors.len(), 1);
        assert!(colors.contains(ManaColor::Blue));
        assert!(!colors.contains(ManaColor::White));
    }

    #[test]
    fn test_iteration() {
        let colors = ManaColors::new().with(ManaColor::Red).with(ManaColor::Green);

        let collected: Vec<_> = colors.iter().collect();
        assert_eq!(collected.len(), 2);
        assert!(collected.contains(&ManaColor::Red));
        assert!(collected.contains(&ManaColor::Green));
    }

    #[test]
    fn test_from_slice() {
        let vec = vec![ManaColor::Red, ManaColor::Green];
        let colors = ManaColors::from_slice(&vec);

        assert_eq!(colors.len(), 2);
        assert!(colors.contains(ManaColor::Red));
        assert!(colors.contains(ManaColor::Green));
    }

    #[test]
    fn test_set_operations() {
        let rg = ManaColors::new().with(ManaColor::Red).with(ManaColor::Green);
        let ub = ManaColors::new().with(ManaColor::Blue).with(ManaColor::Black);

        let union = rg.union(&ub);
        assert_eq!(union.len(), 4);
        assert!(union.contains(ManaColor::Red));
        assert!(union.contains(ManaColor::Blue));

        let intersection = rg.intersection(&ub);
        assert!(intersection.is_empty());

        let rg2 = ManaColors::new().with(ManaColor::Red);
        assert!(rg2.is_subset_of(&rg));
        assert!(!rg.is_subset_of(&rg2));
    }

    #[test]
    fn test_all_colors() {
        let all = ManaColors::all();
        assert_eq!(all.len(), 5);
        assert!(all.contains(ManaColor::White));
        assert!(all.contains(ManaColor::Blue));
        assert!(all.contains(ManaColor::Black));
        assert!(all.contains(ManaColor::Red));
        assert!(all.contains(ManaColor::Green));
    }

    #[test]
    fn test_dual_land_constructors() {
        let taiga = ManaColors::taiga();
        assert_eq!(taiga.len(), 2);
        assert!(taiga.contains(ManaColor::Red));
        assert!(taiga.contains(ManaColor::Green));

        let tundra = ManaColors::tundra();
        assert_eq!(tundra.len(), 2);
        assert!(tundra.contains(ManaColor::White));
        assert!(tundra.contains(ManaColor::Blue));
    }

    #[test]
    fn test_display() {
        let taiga = ManaColors::taiga();
        let display = format!("{}", taiga);
        assert!(display.contains('R'));
        assert!(display.contains('G'));

        let empty = ManaColors::new();
        assert_eq!(format!("{}", empty), "{}");
    }

    #[test]
    fn test_size() {
        // Verify that ManaColors is indeed just 1 byte
        assert_eq!(std::mem::size_of::<ManaColors>(), 1);

        // Compare to Vec<ManaColor> which would be at least 24 bytes (3 * 8 bytes on 64-bit)
        assert!(std::mem::size_of::<Vec<ManaColor>>() >= 24);
    }

    #[test]
    fn test_copy_semantics() {
        let colors1 = ManaColors::taiga();
        let colors2 = colors1; // Should be a copy, not a move

        // Both should be usable
        assert_eq!(colors1.len(), 2);
        assert_eq!(colors2.len(), 2);
    }
}
