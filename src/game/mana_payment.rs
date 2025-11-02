//! Mana payment resolution system
//!
//! This module provides the interface and implementations for determining
//! how to pay mana costs using available mana sources.
//!
//! # Architecture
//!
//! The system is designed with a clean interface that allows multiple
//! implementation strategies:
//!
//! - **SimpleManaResolver**: Handles basic lands (Mountains, Islands, etc.)
//! - **GreedyManaResolver**: Java Forge-style greedy algorithm for complex sources
//! - **BacktrackingResolver**: Complete search for optimal solutions (future)
//! - **OptimalResolver**: Graph-based optimal solver (future)
//!
//! # Example
//!
//! ```ignore
//! use mtg_forge_rs::game::mana_payment::{ManaSource, ManaPaymentResolver, SimpleManaResolver};
//! use mtg_forge_rs::core::ManaCost;
//!
//! let resolver = SimpleManaResolver::new();
//! let sources = vec![/* ... */];
//! let cost = ManaCost::from_string("2R");
//!
//! if resolver.can_pay(&cost, &sources) {
//!     let tap_order = resolver.compute_tap_order(&cost, &sources).unwrap();
//!     // Use tap_order to actually tap the lands
//! }
//! ```

use crate::core::{CardId, ManaCost};

/// Result of checking whether a mana cost can be paid
///
/// This three-valued logic allows us to distinguish between:
/// - Definite success (with solution)
/// - Definite failure (provably impossible)
/// - Uncertain (greedy failed but backtracking might succeed)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaymentResult {
    /// We can definitely pay this cost, here's the tap order
    Yes(Vec<CardId>),

    /// We can prove that this cost cannot be paid with available sources
    No,

    /// Our greedy algorithm couldn't find a solution, but one might exist
    /// via backtracking. This means the problem is complex enough that
    /// we'd need a full search to be certain.
    Maybe,
}

/// Represents a single mana-producing source (land or creature)
///
/// This struct captures all the information needed to determine what mana
/// a permanent can produce and under what conditions.
#[derive(Debug, Clone)]
pub struct ManaSource {
    /// The card producing the mana
    pub card_id: CardId,

    /// The type of mana this source produces
    pub production: ManaProduction,

    /// Whether this source is currently tapped
    pub is_tapped: bool,

    /// Whether this source has summoning sickness (for creatures)
    pub has_summoning_sickness: bool,
}

/// What mana a source can produce and at what cost
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManaProduction {
    /// The type of mana this source produces
    pub kind: ManaProductionKind,

    /// Optional activation cost (e.g., pay {2} to produce mana)
    /// None means no mana cost (tap-only or free ability)
    pub activation_cost: Option<ManaCost>,
}

impl ManaProduction {
    /// Create a new mana production with no activation cost
    pub fn free(kind: ManaProductionKind) -> Self {
        Self {
            kind,
            activation_cost: None,
        }
    }

    /// Create a new mana production with an activation cost
    pub fn with_cost(kind: ManaProductionKind, cost: ManaCost) -> Self {
        Self {
            kind,
            activation_cost: Some(cost),
        }
    }

    /// Get the net mana delta (production - cost) for total mana bounds checking
    /// This is an i8 because you can have negative delta (pay more than you produce)
    pub fn net_delta(&self) -> i8 {
        let production = 1; // Each source produces 1 mana (we'll handle Amount$ later)
        let cost = self.activation_cost.as_ref().map(|c| c.cmc() as i8).unwrap_or(0);
        production - cost
    }
}

/// The kind of mana a source can produce
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManaProductionKind {
    /// Produces exactly one specific color (e.g., Mountain → {R})
    Fixed(ManaColor),

    /// Can produce one of several colors (e.g., Taiga → {R} or {G})
    Choice(Vec<ManaColor>),

    /// Can produce any color (e.g., City of Brass)
    AnyColor,

    /// Produces colorless mana
    Colorless,
}

/// Represents a color of mana
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ManaColor {
    White,
    Blue,
    Black,
    Red,
    Green,
}

impl ManaColor {
    /// Convert to single-character representation (W, U, B, R, G)
    pub fn to_char(self) -> char {
        match self {
            ManaColor::White => 'W',
            ManaColor::Blue => 'U',
            ManaColor::Black => 'B',
            ManaColor::Red => 'R',
            ManaColor::Green => 'G',
        }
    }

    /// Parse from single-character representation
    pub fn from_char(c: char) -> Option<Self> {
        match c {
            'W' | 'w' => Some(ManaColor::White),
            'U' | 'u' => Some(ManaColor::Blue),
            'B' | 'b' => Some(ManaColor::Black),
            'R' | 'r' => Some(ManaColor::Red),
            'G' | 'g' => Some(ManaColor::Green),
            _ => None,
        }
    }
}

/// Trait for mana payment resolution strategies
///
/// Different implementations can provide different algorithms for determining
/// how to pay mana costs. The interface is kept minimal to allow flexibility.
pub trait ManaPaymentResolver {
    /// Check if a cost can be paid and return the payment result
    ///
    /// This is the primary method that should be implemented. It returns:
    /// - `PaymentResult::Yes(tap_order)` if we found a solution
    /// - `PaymentResult::No` if we can prove it's impossible
    /// - `PaymentResult::Maybe` if our algorithm couldn't find a solution but one might exist
    fn check_payment(&self, cost: &ManaCost, sources: &[ManaSource]) -> PaymentResult;

    /// Quick bounds check without attempting to construct a solution
    ///
    /// This is a fast pessimistic check that returns:
    /// - `PaymentResult::No` if we can prove it's impossible (insufficient mana, wrong colors)
    /// - `PaymentResult::Maybe` otherwise (might be possible, need full check)
    ///
    /// This never returns `Yes` - use `check_payment()` for that.
    fn quick_check(&self, cost: &ManaCost, sources: &[ManaSource]) -> PaymentResult {
        // Default implementation: just do full check
        match self.check_payment(cost, sources) {
            PaymentResult::Yes(_) => PaymentResult::Maybe,
            other => other,
        }
    }

    /// Check if a mana cost can be paid with the given sources
    ///
    /// This is pessimistic: `Maybe` is treated as `No`.
    /// Returns `true` only if we have a definite solution.
    fn can_pay(&self, cost: &ManaCost, sources: &[ManaSource]) -> bool {
        matches!(self.check_payment(cost, sources), PaymentResult::Yes(_))
    }

    /// Compute the actual tap order for paying a cost
    ///
    /// Returns `Some(vec)` with the CardIds to tap in order if payment is
    /// possible, or `None` if the cost cannot be paid or is uncertain.
    ///
    /// The returned vector should contain exactly the cards needed to pay
    /// the cost, in the order they should be tapped.
    fn compute_tap_order(&self, cost: &ManaCost, sources: &[ManaSource]) -> Option<Vec<CardId>> {
        match self.check_payment(cost, sources) {
            PaymentResult::Yes(tap_order) => Some(tap_order),
            _ => None,
        }
    }
}

/// Simple resolver for basic lands only
///
/// This is the initial implementation that only handles lands that produce
/// a single fixed color (Plains, Island, Swamp, Mountain, Forest, Wastes).
///
/// This resolver uses a straightforward algorithm:
/// 1. Count available mana of each color
/// 2. Match specific color requirements first
/// 3. Use remaining sources for generic costs
pub struct SimpleManaResolver;

impl SimpleManaResolver {
    pub fn new() -> Self {
        Self
    }
}

impl Default for SimpleManaResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl ManaPaymentResolver for SimpleManaResolver {
    fn check_payment(&self, cost: &ManaCost, sources: &[ManaSource]) -> PaymentResult {
        // Count available mana by color (only untapped, fixed-color sources)
        let mut white = 0u8;
        let mut blue = 0u8;
        let mut black = 0u8;
        let mut red = 0u8;
        let mut green = 0u8;
        let mut colorless = 0u8;
        let mut has_complex = false;

        for source in sources {
            if source.is_tapped || source.has_summoning_sickness {
                continue;
            }

            match &source.production.kind {
                ManaProductionKind::Fixed(color) => match color {
                    ManaColor::White => white += 1,
                    ManaColor::Blue => blue += 1,
                    ManaColor::Black => black += 1,
                    ManaColor::Red => red += 1,
                    ManaColor::Green => green += 1,
                },
                ManaProductionKind::Colorless => colorless += 1,
                _ => {
                    // SimpleManaResolver doesn't handle complex sources
                    // If we encounter any, we return Maybe (backtracking might help)
                    has_complex = true;
                }
            }
        }

        // If we have complex sources, we can't be certain
        if has_complex {
            return PaymentResult::Maybe;
        }

        // Check specific color requirements - these are definite "No" proofs
        if cost.white > white {
            return PaymentResult::No;
        }
        if cost.blue > blue {
            return PaymentResult::No;
        }
        if cost.black > black {
            return PaymentResult::No;
        }
        if cost.red > red {
            return PaymentResult::No;
        }
        if cost.green > green {
            return PaymentResult::No;
        }
        if cost.colorless > colorless {
            return PaymentResult::No;
        }

        // Check if we have enough total mana for generic requirement
        let total = white
            .saturating_add(blue)
            .saturating_add(black)
            .saturating_add(red)
            .saturating_add(green)
            .saturating_add(colorless);

        let used = cost
            .white
            .saturating_add(cost.blue)
            .saturating_add(cost.black)
            .saturating_add(cost.red)
            .saturating_add(cost.green)
            .saturating_add(cost.colorless);

        let remaining = total.saturating_sub(used);

        if remaining < cost.generic {
            return PaymentResult::No;
        }

        // We can pay! Now compute the tap order
        let mut tap_order = Vec::new();
        let mut remaining_cost = *cost;

        // Helper to tap sources of a specific color
        let mut tap_color = |color: ManaColor, amount: u8, sources: &[ManaSource]| {
            let mut tapped = 0;
            for source in sources {
                if tapped >= amount {
                    break;
                }
                if source.is_tapped || source.has_summoning_sickness || tap_order.contains(&source.card_id) {
                    continue;
                }
                if let ManaProductionKind::Fixed(c) = source.production.kind {
                    if c == color {
                        tap_order.push(source.card_id);
                        tapped += 1;
                    }
                }
            }
        };

        // Tap sources for specific color requirements first
        tap_color(ManaColor::White, remaining_cost.white, sources);
        remaining_cost.white = 0;

        tap_color(ManaColor::Blue, remaining_cost.blue, sources);
        remaining_cost.blue = 0;

        tap_color(ManaColor::Black, remaining_cost.black, sources);
        remaining_cost.black = 0;

        tap_color(ManaColor::Red, remaining_cost.red, sources);
        remaining_cost.red = 0;

        tap_color(ManaColor::Green, remaining_cost.green, sources);
        remaining_cost.green = 0;

        // Tap colorless sources for colorless requirement
        let mut tapped_colorless = 0;
        for source in sources {
            if tapped_colorless >= remaining_cost.colorless {
                break;
            }
            if source.is_tapped || source.has_summoning_sickness || tap_order.contains(&source.card_id) {
                continue;
            }
            if source.production.kind == ManaProductionKind::Colorless {
                tap_order.push(source.card_id);
                tapped_colorless += 1;
            }
        }
        remaining_cost.colorless = 0;

        // Tap any remaining sources for generic cost
        let mut tapped_generic = 0;
        for source in sources {
            if tapped_generic >= remaining_cost.generic {
                break;
            }
            if source.is_tapped || source.has_summoning_sickness || tap_order.contains(&source.card_id) {
                continue;
            }
            // Can use any untapped source for generic
            match source.production.kind {
                ManaProductionKind::Fixed(_) | ManaProductionKind::Colorless => {
                    tap_order.push(source.card_id);
                    tapped_generic += 1;
                }
                _ => {} // Skip complex sources (shouldn't be any at this point)
            }
        }

        PaymentResult::Yes(tap_order)
    }
}

/// Greedy resolver for complex mana sources
///
/// This resolver handles dual lands (Taiga, Badlands, etc.) and multicolor lands
/// (City of Brass) using a greedy algorithm similar to Java Forge.
///
/// Algorithm:
/// 1. Pay specific color requirements first, preferring:
///    - Fixed sources of that color (e.g., Mountain for R)
///    - Dual lands that produce that color (e.g., Taiga for R)
///    - Any-color sources (e.g., City of Brass)
/// 2. Pay colorless requirements with Wastes
/// 3. Pay generic requirements with any remaining sources
///
/// The greedy approach preserves more flexible sources (any-color lands)
/// for later requirements when possible.
pub struct GreedyManaResolver;

impl GreedyManaResolver {
    pub fn new() -> Self {
        Self
    }

    /// Check if a source can produce a specific color
    fn can_produce_color(production: &ManaProduction, color: ManaColor) -> bool {
        match &production.kind {
            ManaProductionKind::Fixed(c) => *c == color,
            ManaProductionKind::Choice(colors) => colors.contains(&color),
            ManaProductionKind::AnyColor => true,
            ManaProductionKind::Colorless => false,
        }
    }

    /// Score a source for a specific color (lower = better = more specific)
    /// This helps us tap the most specific sources first
    fn score_for_color(production: &ManaProduction, color: ManaColor) -> u8 {
        match &production.kind {
            ManaProductionKind::Fixed(c) if *c == color => 0, // Best: exact match
            ManaProductionKind::Choice(colors) if colors.contains(&color) => {
                colors.len() as u8 // Better: dual land (prefer fewer options)
            }
            ManaProductionKind::AnyColor => 100, // Worst: save for last resort
            _ => 255,                            // Can't produce this color
        }
    }
}

impl Default for GreedyManaResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl ManaPaymentResolver for GreedyManaResolver {
    fn check_payment(&self, cost: &ManaCost, sources: &[ManaSource]) -> PaymentResult {
        // First, do bounds checking to see if we can prove "No"

        // Check total available mana (accounting for activation costs)
        // Use net delta: production - cost for each source
        // For example, Celestial Prism ({2}, {T}: Add one mana of any color) has delta of -1
        let mut available_delta: i16 = 0; // Use i16 to handle negative deltas
        for source in sources {
            if !source.is_tapped && !source.has_summoning_sickness {
                available_delta += source.production.net_delta() as i16;
            }
        }

        let needed = cost
            .white
            .saturating_add(cost.blue)
            .saturating_add(cost.black)
            .saturating_add(cost.red)
            .saturating_add(cost.green)
            .saturating_add(cost.colorless)
            .saturating_add(cost.generic);

        // Can only prove "No" if the total delta is negative and insufficient
        // If available_delta < needed, we definitely can't pay
        if available_delta < needed as i16 {
            return PaymentResult::No; // Provably impossible - not enough total mana (accounting for costs)
        }

        // Check if we can produce enough of each required color
        // NOTE: For upper bound color checking, we IGNORE activation costs
        // (treating all sources as free). This is an optimistic approximation
        // that allows us to prove impossibility when we can't even meet the
        // color requirements with free mana.
        let mut max_white = 0u8;
        let mut max_blue = 0u8;
        let mut max_black = 0u8;
        let mut max_red = 0u8;
        let mut max_green = 0u8;
        let mut max_colorless = 0u8;

        for source in sources {
            if source.is_tapped || source.has_summoning_sickness {
                continue;
            }

            match &source.production.kind {
                ManaProductionKind::Fixed(color) => match color {
                    ManaColor::White => max_white += 1,
                    ManaColor::Blue => max_blue += 1,
                    ManaColor::Black => max_black += 1,
                    ManaColor::Red => max_red += 1,
                    ManaColor::Green => max_green += 1,
                },
                ManaProductionKind::Colorless => max_colorless += 1,
                ManaProductionKind::Choice(colors) => {
                    // Choice lands count toward each color they can produce
                    for color in colors {
                        match color {
                            ManaColor::White => max_white += 1,
                            ManaColor::Blue => max_blue += 1,
                            ManaColor::Black => max_black += 1,
                            ManaColor::Red => max_red += 1,
                            ManaColor::Green => max_green += 1,
                        }
                    }
                }
                ManaProductionKind::AnyColor => {
                    // Any-color lands count toward all colors
                    max_white += 1;
                    max_blue += 1;
                    max_black += 1;
                    max_red += 1;
                    max_green += 1;
                }
            }
        }

        // If we can't produce enough of a specific color, it's provably impossible
        if cost.white > max_white {
            return PaymentResult::No;
        }
        if cost.blue > max_blue {
            return PaymentResult::No;
        }
        if cost.black > max_black {
            return PaymentResult::No;
        }
        if cost.red > max_red {
            return PaymentResult::No;
        }
        if cost.green > max_green {
            return PaymentResult::No;
        }
        if cost.colorless > max_colorless {
            return PaymentResult::No;
        }

        // Bounds check passed, now try greedy algorithm
        let tap_order_result = self.try_greedy_payment(cost, sources);

        match tap_order_result {
            Some(tap_order) => PaymentResult::Yes(tap_order),
            None => {
                // Greedy failed but bounds check says it might be possible
                // A backtracking search might find a solution
                PaymentResult::Maybe
            }
        }
    }

    fn quick_check(&self, cost: &ManaCost, sources: &[ManaSource]) -> PaymentResult {
        // Same bounds checks as check_payment, but don't try greedy algorithm

        // Check total available mana using net delta (accounting for activation costs)
        let mut available_delta: i16 = 0;
        for source in sources {
            if !source.is_tapped && !source.has_summoning_sickness {
                available_delta += source.production.net_delta() as i16;
            }
        }

        let needed = cost
            .white
            .saturating_add(cost.blue)
            .saturating_add(cost.black)
            .saturating_add(cost.red)
            .saturating_add(cost.green)
            .saturating_add(cost.colorless)
            .saturating_add(cost.generic);

        if available_delta < needed as i16 {
            return PaymentResult::No;
        }

        // Quick color bounds check (simplified - just check if impossible)
        let mut max_white = 0u8;
        let mut max_blue = 0u8;
        let mut max_black = 0u8;
        let mut max_red = 0u8;
        let mut max_green = 0u8;
        let mut max_colorless = 0u8;

        for source in sources {
            if source.is_tapped || source.has_summoning_sickness {
                continue;
            }

            match &source.production.kind {
                ManaProductionKind::Fixed(color) => match color {
                    ManaColor::White => max_white += 1,
                    ManaColor::Blue => max_blue += 1,
                    ManaColor::Black => max_black += 1,
                    ManaColor::Red => max_red += 1,
                    ManaColor::Green => max_green += 1,
                },
                ManaProductionKind::Colorless => max_colorless += 1,
                ManaProductionKind::Choice(colors) => {
                    for color in colors {
                        match color {
                            ManaColor::White => max_white += 1,
                            ManaColor::Blue => max_blue += 1,
                            ManaColor::Black => max_black += 1,
                            ManaColor::Red => max_red += 1,
                            ManaColor::Green => max_green += 1,
                        }
                    }
                }
                ManaProductionKind::AnyColor => {
                    max_white += 1;
                    max_blue += 1;
                    max_black += 1;
                    max_red += 1;
                    max_green += 1;
                }
            }
        }

        if cost.white > max_white
            || cost.blue > max_blue
            || cost.black > max_black
            || cost.red > max_red
            || cost.green > max_green
            || cost.colorless > max_colorless
        {
            return PaymentResult::No;
        }

        // Might be possible, need full check
        PaymentResult::Maybe
    }
}

impl GreedyManaResolver {
    /// Try to pay using greedy algorithm, return tap order if successful
    fn try_greedy_payment(&self, cost: &ManaCost, sources: &[ManaSource]) -> Option<Vec<CardId>> {
        let mut tap_order = Vec::new();
        let mut remaining_cost = *cost;

        // Helper to tap sources for a specific color
        let mut tap_for_color = |color: ManaColor, amount: u8| {
            let mut tapped = 0u8;

            // Create list of available sources that can produce this color
            let mut candidates: Vec<(usize, u8)> = sources
                .iter()
                .enumerate()
                .filter(|(_, s)| {
                    !s.is_tapped
                        && !s.has_summoning_sickness
                        && !tap_order.contains(&s.card_id)
                        && Self::can_produce_color(&s.production, color)
                })
                .map(|(idx, s)| (idx, Self::score_for_color(&s.production, color)))
                .collect();

            // Sort by score (lower = more specific = tap first)
            candidates.sort_by_key(|(_, score)| *score);

            // Tap sources in priority order
            for (idx, _score) in candidates {
                if tapped >= amount {
                    break;
                }
                tap_order.push(sources[idx].card_id);
                tapped += 1;
            }

            tapped >= amount
        };

        // Pay specific color requirements first
        if remaining_cost.white > 0 && !tap_for_color(ManaColor::White, remaining_cost.white) {
            return None;
        }
        remaining_cost.white = 0;

        if remaining_cost.blue > 0 && !tap_for_color(ManaColor::Blue, remaining_cost.blue) {
            return None;
        }
        remaining_cost.blue = 0;

        if remaining_cost.black > 0 && !tap_for_color(ManaColor::Black, remaining_cost.black) {
            return None;
        }
        remaining_cost.black = 0;

        if remaining_cost.red > 0 && !tap_for_color(ManaColor::Red, remaining_cost.red) {
            return None;
        }
        remaining_cost.red = 0;

        if remaining_cost.green > 0 && !tap_for_color(ManaColor::Green, remaining_cost.green) {
            return None;
        }
        remaining_cost.green = 0;

        // Pay colorless requirement with colorless sources
        if remaining_cost.colorless > 0 {
            let mut tapped = 0u8;
            for source in sources {
                if tapped >= remaining_cost.colorless {
                    break;
                }
                if source.is_tapped || source.has_summoning_sickness || tap_order.contains(&source.card_id) {
                    continue;
                }
                if source.production.kind == ManaProductionKind::Colorless {
                    tap_order.push(source.card_id);
                    tapped += 1;
                }
            }
            if tapped < remaining_cost.colorless {
                return None;
            }
        }
        remaining_cost.colorless = 0;

        // Pay generic cost with any remaining sources
        if remaining_cost.generic > 0 {
            let mut tapped = 0u8;
            for source in sources {
                if tapped >= remaining_cost.generic {
                    break;
                }
                if source.is_tapped || source.has_summoning_sickness || tap_order.contains(&source.card_id) {
                    continue;
                }
                tap_order.push(source.card_id);
                tapped += 1;
            }
            if tapped < remaining_cost.generic {
                return None;
            }
        }

        Some(tap_order)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mana_color_conversion() {
        assert_eq!(ManaColor::White.to_char(), 'W');
        assert_eq!(ManaColor::Blue.to_char(), 'U');
        assert_eq!(ManaColor::from_char('R'), Some(ManaColor::Red));
        assert_eq!(ManaColor::from_char('g'), Some(ManaColor::Green));
        assert_eq!(ManaColor::from_char('X'), None);
    }

    #[test]
    fn test_simple_resolver_exact_match() {
        let resolver = SimpleManaResolver::new();

        let sources = vec![
            ManaSource {
                card_id: CardId::new(1),
                production: ManaProduction::free(ManaProductionKind::Fixed(ManaColor::Red)),
                is_tapped: false,
                has_summoning_sickness: false,
            },
            ManaSource {
                card_id: CardId::new(2),
                production: ManaProduction::free(ManaProductionKind::Fixed(ManaColor::Red)),
                is_tapped: false,
                has_summoning_sickness: false,
            },
            ManaSource {
                card_id: CardId::new(3),
                production: ManaProduction::free(ManaProductionKind::Fixed(ManaColor::Red)),
                is_tapped: false,
                has_summoning_sickness: false,
            },
        ];

        // Cost: 2R requires 1 red + 2 generic (can pay with 3 red)
        let cost = ManaCost {
            generic: 2,
            white: 0,
            blue: 0,
            black: 0,
            red: 1,
            green: 0,
            colorless: 0,
            x_count: 0,
        };

        assert!(resolver.can_pay(&cost, &sources));

        let tap_order = resolver.compute_tap_order(&cost, &sources).unwrap();
        assert_eq!(tap_order.len(), 3); // Should tap all 3 mountains
    }

    #[test]
    fn test_simple_resolver_insufficient_color() {
        let resolver = SimpleManaResolver::new();

        let sources = vec![ManaSource {
            card_id: CardId::new(1),
            production: ManaProduction::free(ManaProductionKind::Fixed(ManaColor::Red)),
            is_tapped: false,
            has_summoning_sickness: false,
        }];

        // Cost: 1U requires blue mana
        let cost = ManaCost {
            generic: 1,
            white: 0,
            blue: 1,
            black: 0,
            red: 0,
            green: 0,
            colorless: 0,
            x_count: 0,
        };

        assert!(!resolver.can_pay(&cost, &sources));
        assert!(resolver.compute_tap_order(&cost, &sources).is_none());
    }

    #[test]
    fn test_simple_resolver_rejects_complex_sources() {
        let resolver = SimpleManaResolver::new();

        let sources = vec![
            ManaSource {
                card_id: CardId::new(1),
                production: ManaProduction::free(ManaProductionKind::Fixed(ManaColor::Red)),
                is_tapped: false,
                has_summoning_sickness: false,
            },
            ManaSource {
                card_id: CardId::new(2),
                production: ManaProduction::free(ManaProductionKind::AnyColor),
                is_tapped: false,
                has_summoning_sickness: false,
            },
        ];

        let cost = ManaCost {
            generic: 1,
            white: 0,
            blue: 0,
            black: 0,
            red: 0,
            green: 0,
            colorless: 0,
            x_count: 0,
        };

        // SimpleManaResolver conservatively rejects when complex sources present
        assert!(!resolver.can_pay(&cost, &sources));
    }

    #[test]
    fn test_greedy_resolver_dual_land() {
        let resolver = GreedyManaResolver::new();

        // Taiga (dual land: R or G)
        let sources = vec![
            ManaSource {
                card_id: CardId::new(1),
                production: ManaProduction::free(ManaProductionKind::Choice(vec![ManaColor::Red, ManaColor::Green])),
                is_tapped: false,
                has_summoning_sickness: false,
            },
            ManaSource {
                card_id: CardId::new(2),
                production: ManaProduction::free(ManaProductionKind::Fixed(ManaColor::Red)),
                is_tapped: false,
                has_summoning_sickness: false,
            },
        ];

        // Cost: 1R
        let cost = ManaCost {
            generic: 1,
            white: 0,
            blue: 0,
            black: 0,
            red: 1,
            green: 0,
            colorless: 0,
            x_count: 0,
        };

        assert!(resolver.can_pay(&cost, &sources));

        let tap_order = resolver.compute_tap_order(&cost, &sources).unwrap();
        assert_eq!(tap_order.len(), 2);
        // Should prefer Mountain (card 2) for R, then Taiga for generic
        assert_eq!(tap_order[0], CardId::new(2)); // Mountain for R
        assert_eq!(tap_order[1], CardId::new(1)); // Taiga for generic
    }

    #[test]
    fn test_greedy_resolver_city_of_brass() {
        let resolver = GreedyManaResolver::new();

        // City of Brass (any color)
        let sources = vec![ManaSource {
            card_id: CardId::new(1),
            production: ManaProduction::free(ManaProductionKind::AnyColor),
            is_tapped: false,
            has_summoning_sickness: false,
        }];

        // Cost: 1R
        let cost = ManaCost {
            generic: 0,
            white: 0,
            blue: 0,
            black: 0,
            red: 1,
            green: 0,
            colorless: 0,
            x_count: 0,
        };

        assert!(resolver.can_pay(&cost, &sources));
        let tap_order = resolver.compute_tap_order(&cost, &sources).unwrap();
        assert_eq!(tap_order.len(), 1);
    }

    #[test]
    fn test_greedy_resolver_prefers_specific_sources() {
        let resolver = GreedyManaResolver::new();

        let sources = vec![
            ManaSource {
                card_id: CardId::new(1),
                production: ManaProduction::free(ManaProductionKind::AnyColor), // City of Brass
                is_tapped: false,
                has_summoning_sickness: false,
            },
            ManaSource {
                card_id: CardId::new(2),
                production: ManaProduction::free(ManaProductionKind::Choice(vec![ManaColor::Red, ManaColor::Green])), // Taiga
                is_tapped: false,
                has_summoning_sickness: false,
            },
            ManaSource {
                card_id: CardId::new(3),
                production: ManaProduction::free(ManaProductionKind::Fixed(ManaColor::Red)), // Mountain
                is_tapped: false,
                has_summoning_sickness: false,
            },
        ];

        // Cost: R (just one red)
        let cost = ManaCost {
            generic: 0,
            white: 0,
            blue: 0,
            black: 0,
            red: 1,
            green: 0,
            colorless: 0,
            x_count: 0,
        };

        let tap_order = resolver.compute_tap_order(&cost, &sources).unwrap();
        assert_eq!(tap_order.len(), 1);
        // Should prefer Mountain (most specific) over Taiga or City of Brass
        assert_eq!(tap_order[0], CardId::new(3));
    }

    #[test]
    fn test_greedy_resolver_multicolor_cost() {
        let resolver = GreedyManaResolver::new();

        let sources = vec![
            ManaSource {
                card_id: CardId::new(1),
                production: ManaProduction::free(ManaProductionKind::Fixed(ManaColor::Red)),
                is_tapped: false,
                has_summoning_sickness: false,
            },
            ManaSource {
                card_id: CardId::new(2),
                production: ManaProduction::free(ManaProductionKind::Fixed(ManaColor::Green)),
                is_tapped: false,
                has_summoning_sickness: false,
            },
            ManaSource {
                card_id: CardId::new(3),
                production: ManaProduction::free(ManaProductionKind::Choice(vec![ManaColor::Red, ManaColor::Green])), // Taiga
                is_tapped: false,
                has_summoning_sickness: false,
            },
        ];

        // Cost: 1RG
        let cost = ManaCost {
            generic: 1,
            white: 0,
            blue: 0,
            black: 0,
            red: 1,
            green: 1,
            colorless: 0,
            x_count: 0,
        };

        assert!(resolver.can_pay(&cost, &sources));
        let tap_order = resolver.compute_tap_order(&cost, &sources).unwrap();
        assert_eq!(tap_order.len(), 3);
    }

    #[test]
    fn test_greedy_resolver_insufficient_mana() {
        let resolver = GreedyManaResolver::new();

        let sources = vec![ManaSource {
            card_id: CardId::new(1),
            production: ManaProduction::free(ManaProductionKind::Fixed(ManaColor::Red)),
            is_tapped: false,
            has_summoning_sickness: false,
        }];

        // Cost: 1UU (needs blue)
        let cost = ManaCost {
            generic: 1,
            white: 0,
            blue: 2,
            black: 0,
            red: 0,
            green: 0,
            colorless: 0,
            x_count: 0,
        };

        assert!(!resolver.can_pay(&cost, &sources));
        assert!(resolver.compute_tap_order(&cost, &sources).is_none());
    }

    // Tests for PaymentResult::Maybe behavior

    #[test]
    fn test_simple_resolver_returns_maybe_for_complex_sources() {
        let resolver = SimpleManaResolver::new();

        let sources = vec![
            ManaSource {
                card_id: CardId::new(1),
                production: ManaProduction::free(ManaProductionKind::Fixed(ManaColor::Red)),
                is_tapped: false,
                has_summoning_sickness: false,
            },
            ManaSource {
                card_id: CardId::new(2),
                production: ManaProduction::free(ManaProductionKind::AnyColor), // Complex source
                is_tapped: false,
                has_summoning_sickness: false,
            },
        ];
        let cost = ManaCost {
            generic: 1,
            white: 0,
            blue: 0,
            black: 0,
            red: 0,
            green: 0,
            colorless: 0,
            x_count: 0,
        };

        // SimpleManaResolver returns Maybe when it encounters complex sources
        let result = resolver.check_payment(&cost, &sources);
        assert_eq!(result, PaymentResult::Maybe);

        // can_pay treats Maybe as No (pessimistic)
        assert!(!resolver.can_pay(&cost, &sources));
    }

    #[test]
    fn test_payment_result_yes_returns_tap_order() {
        let resolver = SimpleManaResolver::new();

        let sources = vec![ManaSource {
            card_id: CardId::new(1),
            production: ManaProduction::free(ManaProductionKind::Fixed(ManaColor::Red)),
            is_tapped: false,
            has_summoning_sickness: false,
        }];

        let cost = ManaCost {
            generic: 0,
            white: 0,
            blue: 0,
            black: 0,
            red: 1,
            green: 0,
            colorless: 0,
            x_count: 0,
        };

        let result = resolver.check_payment(&cost, &sources);
        match result {
            PaymentResult::Yes(tap_order) => {
                assert_eq!(tap_order.len(), 1);
                assert_eq!(tap_order[0], CardId::new(1));
            }
            _ => panic!("Expected PaymentResult::Yes, got {:?}", result),
        }
    }

    #[test]
    fn test_payment_result_no_for_insufficient_mana() {
        let resolver = SimpleManaResolver::new();

        let sources = vec![ManaSource {
            card_id: CardId::new(1),
            production: ManaProduction::free(ManaProductionKind::Fixed(ManaColor::Red)),
            is_tapped: false,
            has_summoning_sickness: false,
        }];

        // Need 2 red but only have 1
        let cost = ManaCost {
            generic: 0,
            white: 0,
            blue: 0,
            black: 0,
            red: 2,
            green: 0,
            colorless: 0,
            x_count: 0,
        };

        let result = resolver.check_payment(&cost, &sources);
        assert_eq!(result, PaymentResult::No);
    }

    #[test]
    fn test_greedy_resolver_returns_no_when_provably_impossible() {
        let resolver = GreedyManaResolver::new();

        // Have: 1 Mountain, 1 Taiga
        // Want: 2 blue mana
        // Even though Taiga could theoretically produce mana, it can't produce blue
        let sources = vec![
            ManaSource {
                card_id: CardId::new(1),
                production: ManaProduction::free(ManaProductionKind::Fixed(ManaColor::Red)),
                is_tapped: false,
                has_summoning_sickness: false,
            },
            ManaSource {
                card_id: CardId::new(2),
                production: ManaProduction::free(ManaProductionKind::Choice(vec![ManaColor::Red, ManaColor::Green])),
                is_tapped: false,
                has_summoning_sickness: false,
            },
        ];

        let cost = ManaCost {
            generic: 0,
            white: 0,
            blue: 2,
            black: 0,
            red: 0,
            green: 0,
            colorless: 0,
            x_count: 0,
        };

        // Should return No - provably impossible
        let result = resolver.check_payment(&cost, &sources);
        assert_eq!(result, PaymentResult::No);
    }

    #[test]
    fn test_quick_check_returns_maybe_not_yes() {
        let resolver = SimpleManaResolver::new();

        let sources = vec![ManaSource {
            card_id: CardId::new(1),
            production: ManaProduction::free(ManaProductionKind::Fixed(ManaColor::Red)),
            is_tapped: false,
            has_summoning_sickness: false,
        }];

        let cost = ManaCost {
            generic: 0,
            white: 0,
            blue: 0,
            black: 0,
            red: 1,
            green: 0,
            colorless: 0,
            x_count: 0,
        };

        // quick_check never returns Yes, even when payment is possible
        let result = resolver.quick_check(&cost, &sources);
        assert!(matches!(result, PaymentResult::Maybe | PaymentResult::No));
        assert_ne!(result, PaymentResult::Yes(vec![])); // Should not be Yes
    }

    // Tests for conditional mana sources (sources with activation costs)

    #[test]
    fn test_mana_production_net_delta() {
        // Free source: Mountain ({T}: Add {R})
        let free_source = ManaProduction::free(ManaProductionKind::Fixed(ManaColor::Red));
        assert_eq!(free_source.net_delta(), 1);

        // Positive delta: Sol Ring ({T}: Add {C}{C}) - produces 2, costs 0 = +2 delta
        // Note: We'll handle Amount$ later, for now each source produces 1
        let free_colorless = ManaProduction::free(ManaProductionKind::Colorless);
        assert_eq!(free_colorless.net_delta(), 1);

        // Zero delta: Mana Prism ({1}, {T}: Add one mana of any color) - produces 1, costs 1 = 0 delta
        let zero_delta = ManaProduction::with_cost(ManaProductionKind::AnyColor, ManaCost::from_string("1"));
        assert_eq!(zero_delta.net_delta(), 0);

        // Negative delta: Celestial Prism ({2}, {T}: Add one mana of any color) - produces 1, costs 2 = -1 delta
        let negative_delta = ManaProduction::with_cost(ManaProductionKind::AnyColor, ManaCost::from_string("2"));
        assert_eq!(negative_delta.net_delta(), -1);
    }

    #[test]
    fn test_greedy_resolver_conditional_source_positive_delta() {
        let resolver = GreedyManaResolver::new();

        // Hypothetical: A source that costs {1} to produce {2} (net +1)
        // For testing, we'll simulate this with multiple sources:
        // 2 Mountains + 1 Mana Prism (pay {1} to get any color)
        let sources = vec![
            ManaSource {
                card_id: CardId::new(1),
                production: ManaProduction::free(ManaProductionKind::Fixed(ManaColor::Red)),
                is_tapped: false,
                has_summoning_sickness: false,
            },
            ManaSource {
                card_id: CardId::new(2),
                production: ManaProduction::free(ManaProductionKind::Fixed(ManaColor::Red)),
                is_tapped: false,
                has_summoning_sickness: false,
            },
            ManaSource {
                card_id: CardId::new(3),
                production: ManaProduction::with_cost(ManaProductionKind::AnyColor, ManaCost::from_string("1")), // Zero delta
                is_tapped: false,
                has_summoning_sickness: false,
            },
        ];

        // With 2 free sources (delta +2) and 1 zero-delta source (delta 0), total delta = +2
        // We should be able to pay for costs up to 2

        // Cost: 1R should be possible (even though we'd need to use the conditional source)
        let cost = ManaCost {
            generic: 1,
            white: 0,
            blue: 0,
            black: 0,
            red: 1,
            green: 0,
            colorless: 0,
            x_count: 0,
        };

        // The bounds check should not reject this (total delta = 2, needed = 2)
        let result = resolver.check_payment(&cost, &sources);
        // Greedy might not find a solution (it doesn't use conditional sources yet),
        // but it shouldn't return No due to bounds
        assert_ne!(result, PaymentResult::No);
    }

    #[test]
    fn test_greedy_resolver_conditional_source_negative_delta() {
        let resolver = GreedyManaResolver::new();

        // Celestial Prism: pay {2} to get any color (net -1 delta)
        let sources = vec![
            ManaSource {
                card_id: CardId::new(1),
                production: ManaProduction::free(ManaProductionKind::Fixed(ManaColor::Red)),
                is_tapped: false,
                has_summoning_sickness: false,
            },
            ManaSource {
                card_id: CardId::new(2),
                production: ManaProduction::with_cost(ManaProductionKind::AnyColor, ManaCost::from_string("2")), // Negative delta (-1)
                is_tapped: false,
                has_summoning_sickness: false,
            },
        ];

        // With 1 free source (delta +1) and 1 negative-delta source (delta -1), total delta = 0
        // We can only pay for costs with total = 0

        // Cost: 1 should be impossible (delta = 0, needed = 1)
        let cost = ManaCost {
            generic: 1,
            white: 0,
            blue: 0,
            black: 0,
            red: 0,
            green: 0,
            colorless: 0,
            x_count: 0,
        };

        let result = resolver.check_payment(&cost, &sources);
        assert_eq!(result, PaymentResult::No); // Should be provably impossible
    }

    #[test]
    fn test_greedy_resolver_color_bounds_ignore_costs() {
        let resolver = GreedyManaResolver::new();

        // Signpost Scarecrow: {2}: Add one mana of any color
        // Even though it costs {2}, for color bounds checking we treat it as free
        let sources = vec![ManaSource {
            card_id: CardId::new(1),
            production: ManaProduction::with_cost(ManaProductionKind::AnyColor, ManaCost::from_string("2")),
            is_tapped: false,
            has_summoning_sickness: false,
        }];

        // Cost: {R} (need 1 red mana)
        let cost = ManaCost {
            generic: 0,
            white: 0,
            blue: 0,
            black: 0,
            red: 1,
            green: 0,
            colorless: 0,
            x_count: 0,
        };

        // The color bounds check should pass (we can produce red, ignoring cost)
        // But the total delta check should fail (delta = -1, needed = 1)
        let result = resolver.check_payment(&cost, &sources);
        assert_eq!(result, PaymentResult::No); // Fails on total delta, not color
    }

    #[test]
    fn test_quick_check_with_conditional_sources() {
        let resolver = GreedyManaResolver::new();

        let sources = vec![
            ManaSource {
                card_id: CardId::new(1),
                production: ManaProduction::free(ManaProductionKind::Fixed(ManaColor::Red)),
                is_tapped: false,
                has_summoning_sickness: false,
            },
            ManaSource {
                card_id: CardId::new(2),
                production: ManaProduction::with_cost(ManaProductionKind::AnyColor, ManaCost::from_string("1")), // Zero delta
                is_tapped: false,
                has_summoning_sickness: false,
            },
        ];

        // Total delta = 1 (one free + one zero-delta)
        let cost = ManaCost {
            generic: 2,
            white: 0,
            blue: 0,
            black: 0,
            red: 0,
            green: 0,
            colorless: 0,
            x_count: 0,
        };

        // quick_check should return No (delta = 1, needed = 2)
        let result = resolver.quick_check(&cost, &sources);
        assert_eq!(result, PaymentResult::No);
    }
}
