//! Mana system for casting spells

use serde::{Deserialize, Serialize};
use std::fmt;

/// Mana colors in MTG
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Color {
    White,
    Blue,
    Black,
    Red,
    Green,
    Colorless,
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Color::White => write!(f, "W"),
            Color::Blue => write!(f, "U"),
            Color::Black => write!(f, "B"),
            Color::Red => write!(f, "R"),
            Color::Green => write!(f, "G"),
            Color::Colorless => write!(f, "C"),
        }
    }
}

/// The five colors in WUBRG order (does not include colorless)
pub const ALL_COLORS: [Color; 5] = [Color::White, Color::Blue, Color::Black, Color::Red, Color::Green];

impl Color {
    /// Returns iterator over the five colors in WUBRG order.
    /// Does not include colorless - use `all_colors_and_colorless()` for that.
    ///
    /// This is a zero-cost abstraction: the array is const and
    /// `into_iter()` compiles to the same code as an inline array.
    #[inline]
    pub fn all_colors() -> impl Iterator<Item = Color> {
        ALL_COLORS.into_iter()
    }

    /// Returns iterator over all six mana types including colorless.
    /// Order is WUBRGC (White, Blue, Black, Red, Green, Colorless).
    #[inline]
    pub fn all_colors_and_colorless() -> impl Iterator<Item = Color> {
        [
            Color::White,
            Color::Blue,
            Color::Black,
            Color::Red,
            Color::Green,
            Color::Colorless,
        ]
        .into_iter()
    }
}

/// Represents a mana cost (e.g., "2RR" = 2 generic + 2 red, "X R" = X + 1 red)
/// Copy-eligible since it's just 8 u8 fields (8 bytes)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManaCost {
    pub generic: u8,
    pub white: u8,
    pub blue: u8,
    pub black: u8,
    pub red: u8,
    pub green: u8,
    pub colorless: u8,
    /// Number of X symbols in the cost (e.g., "X R" has x_count=1, "X X R R" has x_count=2)
    /// The actual value of X is determined when the spell is cast
    pub x_count: u8,
}

impl ManaCost {
    pub fn new() -> Self {
        ManaCost {
            generic: 0,
            white: 0,
            blue: 0,
            black: 0,
            red: 0,
            green: 0,
            colorless: 0,
            x_count: 0,
        }
    }

    /// Parse a mana cost string like "2RR", "1UB", or "X R"
    pub fn from_string(s: &str) -> Self {
        let mut cost = ManaCost::new();
        let mut generic_str = String::new();

        for c in s.chars() {
            match c {
                'W' => cost.white += 1,
                'U' => cost.blue += 1,
                'B' => cost.black += 1,
                'R' => cost.red += 1,
                'G' => cost.green += 1,
                'C' => cost.colorless += 1,
                'X' => cost.x_count += 1,
                '0'..='9' => generic_str.push(c),
                _ => {} // Ignore other characters (spaces, etc.)
            }
        }

        if !generic_str.is_empty() {
            cost.generic = generic_str.parse().unwrap_or(0);
        }

        cost
    }

    /// Total converted mana cost (not including X)
    pub fn cmc(&self) -> u8 {
        self.generic + self.white + self.blue + self.black + self.red + self.green + self.colorless
    }

    /// Returns true if this cost contains X (one or more X symbols)
    #[inline]
    pub fn has_x(&self) -> bool {
        self.x_count > 0
    }

    /// Create a new ManaCost with X resolved to a specific value.
    /// Each X symbol adds `x_value` to the generic cost.
    /// The returned cost has x_count = 0.
    pub fn with_x_value(&self, x_value: u8) -> Self {
        ManaCost {
            generic: self.generic.saturating_add(self.x_count.saturating_mul(x_value)),
            white: self.white,
            blue: self.blue,
            black: self.black,
            red: self.red,
            green: self.green,
            colorless: self.colorless,
            x_count: 0,
        }
    }

    /// Check if this cost can be paid with the given mana amounts.
    ///
    /// This is the canonical affordability check used by both ManaPool::can_pay()
    /// and ManaCapacity::can_pay_simple(). It checks:
    /// 1. Each color requirement can be met
    /// 2. Total available mana is enough for generic + colored requirements
    ///
    /// # Arguments
    /// * `white`, `blue`, `black`, `red`, `green`, `colorless` - available mana amounts
    #[inline]
    pub fn is_affordable(&self, white: u8, blue: u8, black: u8, red: u8, green: u8, colorless: u8) -> bool {
        // Check each specific color requirement
        if white < self.white
            || blue < self.blue
            || black < self.black
            || red < self.red
            || green < self.green
            || colorless < self.colorless
        {
            return false;
        }

        // Check total mana for generic requirement
        let total_available = white + blue + black + red + green + colorless;
        total_available >= self.cmc()
    }

    /// Multiply all mana amounts by a factor
    /// Useful for abilities like Sol Ring that produce multiple mana (e.g., {C}{C})
    /// Note: x_count is NOT multiplied since X is a placeholder
    pub fn multiply(&self, factor: u8) -> Self {
        ManaCost {
            generic: self.generic.saturating_mul(factor),
            white: self.white.saturating_mul(factor),
            blue: self.blue.saturating_mul(factor),
            black: self.black.saturating_mul(factor),
            red: self.red.saturating_mul(factor),
            green: self.green.saturating_mul(factor),
            colorless: self.colorless.saturating_mul(factor),
            x_count: self.x_count, // X is not multiplied
        }
    }
}

impl Default for ManaCost {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for ManaCost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // X comes first in mana cost notation
        for _ in 0..self.x_count {
            write!(f, "X")?;
        }
        if self.generic > 0 {
            write!(f, "{}", self.generic)?;
        }
        for _ in 0..self.white {
            write!(f, "W")?;
        }
        for _ in 0..self.blue {
            write!(f, "U")?;
        }
        for _ in 0..self.black {
            write!(f, "B")?;
        }
        for _ in 0..self.red {
            write!(f, "R")?;
        }
        for _ in 0..self.green {
            write!(f, "G")?;
        }
        for _ in 0..self.colorless {
            write!(f, "C")?;
        }
        Ok(())
    }
}

/// Mana pool for a player
/// Copy-eligible since it's just 6 u8 fields (6 bytes)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManaPool {
    pub white: u8,
    pub blue: u8,
    pub black: u8,
    pub red: u8,
    pub green: u8,
    pub colorless: u8,
}

impl ManaPool {
    pub fn new() -> Self {
        ManaPool {
            white: 0,
            blue: 0,
            black: 0,
            red: 0,
            green: 0,
            colorless: 0,
        }
    }

    /// Check if the mana pool is empty (no floating mana)
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.white == 0 && self.blue == 0 && self.black == 0 && self.red == 0 && self.green == 0 && self.colorless == 0
    }

    pub fn add_color(&mut self, color: Color) {
        match color {
            Color::White => self.white += 1,
            Color::Blue => self.blue += 1,
            Color::Black => self.black += 1,
            Color::Red => self.red += 1,
            Color::Green => self.green += 1,
            Color::Colorless => self.colorless += 1,
        }
    }

    pub fn clear(&mut self) {
        self.white = 0;
        self.blue = 0;
        self.black = 0;
        self.red = 0;
        self.green = 0;
        self.colorless = 0;
    }

    /// Check if we can pay the given mana cost with mana currently in the pool.
    ///
    /// This delegates to `ManaCost::is_affordable()` which is the canonical
    /// affordability check shared with `ManaCapacity::can_pay_simple()`.
    #[inline]
    pub fn can_pay(&self, cost: &ManaCost) -> bool {
        cost.is_affordable(self.white, self.blue, self.black, self.red, self.green, self.colorless)
    }

    /// Pay a mana cost from this pool
    ///
    /// This method deducts the mana from the pool. It first pays colored requirements,
    /// then pays generic cost using any remaining mana in WUBRG order.
    ///
    /// # Errors
    ///
    /// Returns an error message if insufficient mana to pay the cost.
    pub fn pay_cost(&mut self, cost: &ManaCost) -> Result<(), String> {
        // First check if we can pay
        if !self.can_pay(cost) {
            return Err(format!(
                "Insufficient mana to pay cost {}. Pool has: {}W {}U {}B {}R {}G {}C",
                cost, self.white, self.blue, self.black, self.red, self.green, self.colorless
            ));
        }

        // Pay colored requirements first
        self.white -= cost.white;
        self.blue -= cost.blue;
        self.black -= cost.black;
        self.red -= cost.red;
        self.green -= cost.green;
        self.colorless -= cost.colorless;

        // Pay generic cost using any remaining mana (WUBRG order)
        let mut generic_remaining = cost.generic;

        // Use white mana for generic
        let white_used = generic_remaining.min(self.white);
        self.white -= white_used;
        generic_remaining -= white_used;

        // Use blue mana for generic
        let blue_used = generic_remaining.min(self.blue);
        self.blue -= blue_used;
        generic_remaining -= blue_used;

        // Use black mana for generic
        let black_used = generic_remaining.min(self.black);
        self.black -= black_used;
        generic_remaining -= black_used;

        // Use red mana for generic
        let red_used = generic_remaining.min(self.red);
        self.red -= red_used;
        generic_remaining -= red_used;

        // Use green mana for generic
        let green_used = generic_remaining.min(self.green);
        self.green -= green_used;
        generic_remaining -= green_used;

        // Use colorless mana for generic
        let colorless_used = generic_remaining.min(self.colorless);
        self.colorless -= colorless_used;
        generic_remaining -= colorless_used;

        debug_assert_eq!(generic_remaining, 0, "Failed to pay generic cost");

        Ok(())
    }

    /// Total mana in pool
    pub fn total(&self) -> u8 {
        self.white + self.blue + self.black + self.red + self.green + self.colorless
    }
}

impl Default for ManaPool {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mana_cost_parsing() {
        let cost = ManaCost::from_string("2RR");
        assert_eq!(cost.generic, 2);
        assert_eq!(cost.red, 2);
        assert_eq!(cost.cmc(), 4);

        let cost2 = ManaCost::from_string("1UB");
        assert_eq!(cost2.generic, 1);
        assert_eq!(cost2.blue, 1);
        assert_eq!(cost2.black, 1);
        assert_eq!(cost2.cmc(), 3);
    }

    #[test]
    fn test_mana_pool() {
        let mut pool = ManaPool::new();
        pool.add_color(Color::Red);
        pool.add_color(Color::Red);
        pool.add_color(Color::Blue);

        assert_eq!(pool.red, 2);
        assert_eq!(pool.blue, 1);

        // Can pay 1R (CMC 2) with our 3 mana
        let cost = ManaCost::from_string("1R");
        assert!(pool.can_pay(&cost));

        // Can pay 2R (CMC 3) with our 3 mana
        let cost2 = ManaCost::from_string("2R");
        assert!(pool.can_pay(&cost2));

        // Cannot pay 3R (CMC 4) with only 3 mana
        let cost3 = ManaCost::from_string("3R");
        assert!(!pool.can_pay(&cost3));

        // Cannot pay RRR (need 3 red, only have 2)
        let cost4 = ManaCost::from_string("RRR");
        assert!(!pool.can_pay(&cost4));
    }

    #[test]
    fn test_pay_cost_simple() {
        let mut pool = ManaPool::new();
        pool.add_color(Color::Red);

        let cost = ManaCost::from_string("R");
        assert!(pool.pay_cost(&cost).is_ok());
        assert_eq!(pool.red, 0);
        assert_eq!(pool.total(), 0);
    }

    #[test]
    fn test_pay_cost_with_generic() {
        let mut pool = ManaPool::new();
        pool.add_color(Color::Red);
        pool.add_color(Color::Red);
        pool.add_color(Color::Blue);

        // Pay 1R: should use 1 red for R, and 1 blue for generic 1
        let cost = ManaCost::from_string("1R");
        assert!(pool.pay_cost(&cost).is_ok());
        assert_eq!(pool.red, 1);
        assert_eq!(pool.blue, 0);
        assert_eq!(pool.total(), 1);
    }

    #[test]
    fn test_pay_cost_multicolor() {
        let mut pool = ManaPool::new();
        pool.add_color(Color::Red);
        pool.add_color(Color::Blue);
        pool.add_color(Color::White);

        // Pay RU: should use 1 red and 1 blue
        let cost = ManaCost::from_string("RU");
        assert!(pool.pay_cost(&cost).is_ok());
        assert_eq!(pool.red, 0);
        assert_eq!(pool.blue, 0);
        assert_eq!(pool.white, 1);
        assert_eq!(pool.total(), 1);
    }

    #[test]
    fn test_pay_cost_insufficient_mana() {
        let mut pool = ManaPool::new();
        pool.add_color(Color::Red);

        // Try to pay 2R with only 1R
        let cost = ManaCost::from_string("2R");
        assert!(pool.pay_cost(&cost).is_err());
        // Pool should be unchanged
        assert_eq!(pool.red, 1);
    }

    #[test]
    fn test_pay_cost_wrong_color() {
        let mut pool = ManaPool::new();
        pool.add_color(Color::Blue);
        pool.add_color(Color::Blue);

        // Try to pay RR with only UU
        let cost = ManaCost::from_string("RR");
        assert!(pool.pay_cost(&cost).is_err());
        // Pool should be unchanged
        assert_eq!(pool.blue, 2);
        assert_eq!(pool.red, 0);
    }

    #[test]
    fn test_pay_cost_complex() {
        let mut pool = ManaPool::new();
        pool.add_color(Color::Red);
        pool.add_color(Color::Red);
        pool.add_color(Color::Red);
        pool.add_color(Color::Blue);

        // Pay 2R: uses 1 red for R requirement, then WUBRG order for generic 2
        // Generic pays: 1 blue, 1 red (WUBRG order)
        let cost = ManaCost::from_string("2R");
        assert!(pool.pay_cost(&cost).is_ok());
        assert_eq!(pool.red, 1); // Started with 3, used 1 for R, 1 for generic
        assert_eq!(pool.blue, 0); // Started with 1, used 1 for generic
        assert_eq!(pool.total(), 1);
    }

    #[test]
    fn test_mana_pool_total() {
        let mut pool = ManaPool::new();
        assert_eq!(pool.total(), 0);

        pool.add_color(Color::Red);
        pool.add_color(Color::Blue);
        pool.add_color(Color::White);
        assert_eq!(pool.total(), 3);

        pool.clear();
        assert_eq!(pool.total(), 0);
    }

    #[test]
    fn test_color_all_colors() {
        // Verify all_colors returns exactly 5 colors in WUBRG order
        let colors: Vec<_> = Color::all_colors().collect();
        assert_eq!(colors.len(), 5);
        assert_eq!(colors[0], Color::White);
        assert_eq!(colors[1], Color::Blue);
        assert_eq!(colors[2], Color::Black);
        assert_eq!(colors[3], Color::Red);
        assert_eq!(colors[4], Color::Green);
    }

    #[test]
    fn test_color_all_colors_and_colorless() {
        // Verify all_colors_and_colorless returns all 6 mana types
        let colors: Vec<_> = Color::all_colors_and_colorless().collect();
        assert_eq!(colors.len(), 6);
        assert_eq!(colors[5], Color::Colorless);
    }

    #[test]
    fn test_mana_cost_is_affordable() {
        // Test basic affordability
        let cost = ManaCost::from_string("2RR"); // 2 generic + 2 red
                                                 // Exact match
        assert!(cost.is_affordable(0, 0, 0, 2, 2, 0)); // 2R + 2G for generic
                                                       // More than enough
        assert!(cost.is_affordable(1, 1, 1, 3, 1, 0)); // extra mana is fine
                                                       // Not enough red
        assert!(!cost.is_affordable(5, 0, 0, 1, 0, 0)); // only 1R
                                                        // Not enough total
        assert!(!cost.is_affordable(0, 0, 0, 2, 0, 0)); // only 2R, need 4 total

        // Test colorless requirement
        let cost2 = ManaCost::from_string("2C"); // 2 generic + 1 colorless
        assert!(cost2.is_affordable(0, 0, 0, 0, 0, 3)); // 3 colorless
        assert!(!cost2.is_affordable(2, 0, 0, 0, 0, 0)); // 2 white, no colorless
    }
}
