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
//!     let mut tap_order = Vec::new();
//!     if resolver.compute_tap_order(&cost, &sources, &mut tap_order) {
//!         // Use tap_order to actually tap the lands
//!     }
//! }
//! ```

use crate::core::{CardId, ManaColor, ManaCost, ManaProduction, ManaProductionKind};
use smallvec::SmallVec;

/// Result of checking whether a mana cost can be paid
///
/// This three-valued logic allows us to distinguish between:
/// - Definite success
/// - Definite failure (provably impossible)
/// - Uncertain (greedy failed but backtracking might succeed)
///
/// Note: The tap order is now written to an output buffer parameter
/// instead of being returned in the Yes variant to avoid allocations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaymentResult {
    /// We can definitely pay this cost
    /// (tap order written to output buffer if provided)
    Yes,

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

// ManaProduction, ManaProductionKind, and ManaColor are now defined in core::mana_production
// and re-exported above for backward compatibility

/// Shared bounds checking logic for mana payment
///
/// This function performs fast rejection tests to determine if a cost is
/// provably impossible to pay with the given sources. It returns:
/// - `PaymentResult::No` if we can prove it's impossible
/// - `PaymentResult::Maybe` if bounds check passes (might be possible)
///
/// This never returns `Yes` - that requires constructing an actual solution.
fn bounds_check_payment(cost: &ManaCost, sources: &[ManaSource]) -> PaymentResult {
    // Check total available mana (accounting for activation costs via net delta)
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

    // Can only prove "No" if the total delta is insufficient
    if available_delta < needed as i16 {
        return PaymentResult::No;
    }

    // Check if we can produce enough of each required color
    // NOTE: For color checking, we IGNORE activation costs (optimistic approximation)
    // This lets us prove impossibility when color requirements can't be met even with free mana
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
                for color in colors.iter() {
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
    if cost.white > max_white
        || cost.blue > max_blue
        || cost.black > max_black
        || cost.red > max_red
        || cost.green > max_green
        || cost.colorless > max_colorless
    {
        return PaymentResult::No;
    }

    // Bounds check passed - might be possible
    PaymentResult::Maybe
}

/// Trait for mana payment resolution strategies
///
/// Different implementations can provide different algorithms for determining
/// how to pay mana costs. The interface is kept minimal to allow flexibility.
///
/// # Output Buffer Pattern
///
/// To avoid allocations in the hot path, this API uses an output buffer pattern:
/// - Pass `Some(&mut vec)` to compute and write the tap order to the buffer
/// - Pass `None` to only check if payment is possible (no tap order computation)
///
/// This allows `can_pay()` to avoid allocations entirely while `compute_tap_order()`
/// can reuse a pre-allocated buffer.
pub trait ManaPaymentResolver {
    /// Check if a cost can be paid and optionally compute the tap order
    ///
    /// # Parameters
    /// - `cost`: The mana cost to pay
    /// - `sources`: Available mana sources
    /// - `tap_order_out`: Optional output buffer for tap order
    ///   - `Some(&mut vec)`: Compute tap order and write to this buffer (cleared first)
    ///   - `None`: Just check if payment is possible (no tap order computation)
    ///
    /// # Returns
    /// - `PaymentResult::Yes` if we found a solution (tap order written to buffer if provided)
    /// - `PaymentResult::No` if we can prove it's impossible
    /// - `PaymentResult::Maybe` if our algorithm couldn't find a solution but one might exist
    fn check_payment(
        &self,
        cost: &ManaCost,
        sources: &[ManaSource],
        tap_order_out: Option<&mut Vec<CardId>>,
    ) -> PaymentResult;

    /// Quick bounds check without attempting to construct a solution
    ///
    /// This is a fast pessimistic check that returns:
    /// - `PaymentResult::No` if we can prove it's impossible (insufficient mana, wrong colors)
    /// - `PaymentResult::Maybe` otherwise (might be possible, need full check)
    ///
    /// This never returns `Yes` - use `check_payment()` for that.
    fn quick_check(&self, cost: &ManaCost, sources: &[ManaSource]) -> PaymentResult {
        // Default implementation uses shared bounds checking
        bounds_check_payment(cost, sources)
    }

    /// Check if a mana cost can be paid with the given sources
    ///
    /// This is pessimistic: `Maybe` is treated as `No`.
    /// Returns `true` only if we have a definite solution.
    ///
    /// This method does NOT allocate - it passes `None` for the tap order buffer.
    fn can_pay(&self, cost: &ManaCost, sources: &[ManaSource]) -> bool {
        matches!(self.check_payment(cost, sources, None), PaymentResult::Yes)
    }

    /// Compute the actual tap order for paying a cost
    ///
    /// Writes the tap order to the provided buffer if payment is possible.
    /// The buffer is cleared before writing.
    ///
    /// Returns `true` if payment is possible (tap order written to buffer),
    /// or `false` if the cost cannot be paid or is uncertain.
    fn compute_tap_order(&self, cost: &ManaCost, sources: &[ManaSource], tap_order_out: &mut Vec<CardId>) -> bool {
        tap_order_out.clear();
        matches!(
            self.check_payment(cost, sources, Some(tap_order_out)),
            PaymentResult::Yes
        )
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
    fn check_payment(
        &self,
        cost: &ManaCost,
        sources: &[ManaSource],
        tap_order_out: Option<&mut Vec<CardId>>,
    ) -> PaymentResult {
        // Check if we have any complex sources - SimpleManaResolver doesn't handle them
        let has_complex = sources.iter().any(|s| {
            !s.is_tapped
                && !s.has_summoning_sickness
                && !matches!(
                    s.production.kind,
                    ManaProductionKind::Fixed(_) | ManaProductionKind::Colorless
                )
        });

        if has_complex {
            return PaymentResult::Maybe;
        }

        // Use shared bounds checking (this handles the detailed color/total checks)
        match bounds_check_payment(cost, sources) {
            PaymentResult::No => return PaymentResult::No,
            PaymentResult::Maybe => {} // Continue to tap order computation
            PaymentResult::Yes => unreachable!("bounds_check never returns Yes"),
        }

        // Bounds check passed and we have only simple sources
        // If no output buffer provided, we can return Yes now (just checking, not computing)
        let Some(tap_order) = tap_order_out else {
            return PaymentResult::Yes;
        };

        // Clear output buffer and compute tap order
        tap_order.clear();
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

        PaymentResult::Yes
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
            ManaProductionKind::Choice(colors) => colors.contains(color),
            ManaProductionKind::AnyColor => true,
            ManaProductionKind::Colorless => false,
        }
    }

    /// Score a source for a specific color (lower = better = more specific)
    /// This helps us tap the most specific sources first
    fn score_for_color(production: &ManaProduction, color: ManaColor) -> u8 {
        match &production.kind {
            ManaProductionKind::Fixed(c) if *c == color => 0, // Best: exact match
            ManaProductionKind::Choice(colors) if colors.contains(color) => {
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
    fn check_payment(
        &self,
        cost: &ManaCost,
        sources: &[ManaSource],
        tap_order_out: Option<&mut Vec<CardId>>,
    ) -> PaymentResult {
        // Use shared bounds checking to see if we can prove "No"
        match bounds_check_payment(cost, sources) {
            PaymentResult::No => return PaymentResult::No,
            PaymentResult::Maybe => {} // Continue to greedy algorithm
            PaymentResult::Yes => unreachable!("bounds_check never returns Yes"),
        }

        // Bounds check passed, now try greedy algorithm
        if self.try_greedy_payment(cost, sources, tap_order_out) {
            PaymentResult::Yes
        } else {
            // Greedy failed but bounds check says it might be possible
            // A backtracking search might find a solution
            PaymentResult::Maybe
        }
    }

    // Note: quick_check uses the default implementation (shared bounds_check_payment)
}

impl GreedyManaResolver {
    /// Try to pay using greedy algorithm
    ///
    /// Returns `true` if payment is possible (tap order written to buffer if provided),
    /// or `false` if greedy algorithm couldn't find a solution.
    fn try_greedy_payment(
        &self,
        cost: &ManaCost,
        sources: &[ManaSource],
        tap_order_out: Option<&mut Vec<CardId>>,
    ) -> bool {
        // If no output buffer provided, we need a temporary one for the algorithm
        // (greedy algorithm needs to track tapped sources even if we don't return the order)
        let mut temp_buffer = Vec::new();
        let tap_order = tap_order_out.unwrap_or(&mut temp_buffer);
        tap_order.clear();

        let mut remaining_cost = *cost;

        // Helper to tap sources for a specific color
        let mut tap_for_color = |color: ManaColor, amount: u8| {
            let mut tapped = 0u8;

            // Create list of available sources that can produce this color
            // Use SmallVec to avoid heap allocation for typical mana source counts (up to 8)
            // Note: (usize, u8) is 16 bytes due to alignment, so 8 items = 128 bytes inline
            let mut candidates: SmallVec<[(usize, u8); 8]> = sources
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
            return false;
        }
        remaining_cost.white = 0;

        if remaining_cost.blue > 0 && !tap_for_color(ManaColor::Blue, remaining_cost.blue) {
            return false;
        }
        remaining_cost.blue = 0;

        if remaining_cost.black > 0 && !tap_for_color(ManaColor::Black, remaining_cost.black) {
            return false;
        }
        remaining_cost.black = 0;

        if remaining_cost.red > 0 && !tap_for_color(ManaColor::Red, remaining_cost.red) {
            return false;
        }
        remaining_cost.red = 0;

        if remaining_cost.green > 0 && !tap_for_color(ManaColor::Green, remaining_cost.green) {
            return false;
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
                return false;
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
                return false;
            }
        }

        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::ManaColors;

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

        let mut tap_order = Vec::new();
        assert!(resolver.compute_tap_order(&cost, &sources, &mut tap_order));
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
        let mut tap_order = Vec::new();
        assert!(!resolver.compute_tap_order(&cost, &sources, &mut tap_order));
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
                production: ManaProduction::free(ManaProductionKind::Choice(
                    ManaColors::new().with(ManaColor::Red).with(ManaColor::Green),
                )),
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

        let mut tap_order = Vec::new();
        assert!(resolver.compute_tap_order(&cost, &sources, &mut tap_order));
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
        let mut tap_order = Vec::new();
        assert!(resolver.compute_tap_order(&cost, &sources, &mut tap_order));
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
                production: ManaProduction::free(ManaProductionKind::Choice(
                    ManaColors::new().with(ManaColor::Red).with(ManaColor::Green),
                )), // Taiga
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

        let mut tap_order = Vec::new();
        assert!(resolver.compute_tap_order(&cost, &sources, &mut tap_order));
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
                production: ManaProduction::free(ManaProductionKind::Choice(
                    ManaColors::new().with(ManaColor::Red).with(ManaColor::Green),
                )), // Taiga
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
        let mut tap_order = Vec::new();
        assert!(resolver.compute_tap_order(&cost, &sources, &mut tap_order));
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
        let mut tap_order = Vec::new();
        assert!(!resolver.compute_tap_order(&cost, &sources, &mut tap_order));
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
        let mut tap_order = Vec::new();
        let result = resolver.check_payment(&cost, &sources, Some(&mut tap_order));
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

        let mut tap_order = Vec::new();
        let result = resolver.check_payment(&cost, &sources, Some(&mut tap_order));
        assert_eq!(result, PaymentResult::Yes);
        assert_eq!(tap_order.len(), 1);
        assert_eq!(tap_order[0], CardId::new(1));
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

        let mut tap_order = Vec::new();
        let result = resolver.check_payment(&cost, &sources, Some(&mut tap_order));
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
                production: ManaProduction::free(ManaProductionKind::Choice(
                    ManaColors::new().with(ManaColor::Red).with(ManaColor::Green),
                )),
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
        let mut tap_order = Vec::new();
        let result = resolver.check_payment(&cost, &sources, Some(&mut tap_order));
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
        assert_ne!(result, PaymentResult::Yes); // Should not be Yes
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
        let result = resolver.check_payment(&cost, &sources, None);
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

        let result = resolver.check_payment(&cost, &sources, None);
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
        let result = resolver.check_payment(&cost, &sources, None);
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
