// Wildcards intentional: ManaProductionKind enum handling - some variants are
// processed in earlier passes, wildcards catch "shouldn't happen" cases.
#![allow(clippy::wildcard_enum_match_arm)]
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
/// This function performs fast rejection AND confirmation tests:
/// - `PaymentResult::Yes` if we can PROVE payment is possible (lower bounds sufficient)
/// - `PaymentResult::No` if we can PROVE it's impossible (upper bounds insufficient)
/// - `PaymentResult::Maybe` if bounds are inconclusive (need greedy/backtracking)
///
/// Lower bound = guaranteed mana (only Fixed sources contribute to specific colors)
/// Upper bound = potential mana (Fixed + Choice + AnyColor all contribute)
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

    // Compute both lower bounds (guaranteed) and upper bounds (potential) for each color
    // Lower bound: Only Fixed sources guarantee a specific color
    // Upper bound: Fixed + Choice + AnyColor all potentially produce each color
    let mut min_white = 0u8;
    let mut min_blue = 0u8;
    let mut min_black = 0u8;
    let mut min_red = 0u8;
    let mut min_green = 0u8;
    let mut min_colorless = 0u8;
    // Count of flexible sources (Choice/AnyColor) that can pay generic costs
    let mut flexible_sources = 0u8;

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
            ManaProductionKind::Fixed(color) => {
                // Fixed sources contribute to both lower and upper bounds
                match color {
                    ManaColor::White => {
                        min_white += 1;
                        max_white += 1;
                    }
                    ManaColor::Blue => {
                        min_blue += 1;
                        max_blue += 1;
                    }
                    ManaColor::Black => {
                        min_black += 1;
                        max_black += 1;
                    }
                    ManaColor::Red => {
                        min_red += 1;
                        max_red += 1;
                    }
                    ManaColor::Green => {
                        min_green += 1;
                        max_green += 1;
                    }
                }
            }
            ManaProductionKind::Colorless => {
                // Colorless contributes to both bounds for colorless requirement
                min_colorless += 1;
                max_colorless += 1;
            }
            ManaProductionKind::Choice(colors) => {
                // Choice lands only contribute to UPPER bounds (no guarantee of specific color)
                // But they do contribute to flexible sources for generic costs
                flexible_sources += 1;
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
                // Any-color only contributes to UPPER bounds
                flexible_sources += 1;
                max_white += 1;
                max_blue += 1;
                max_black += 1;
                max_red += 1;
                max_green += 1;
            }
        }
    }

    // If UPPER bounds can't meet the cost, it's provably impossible
    if cost.white > max_white
        || cost.blue > max_blue
        || cost.black > max_black
        || cost.red > max_red
        || cost.green > max_green
        || cost.colorless > max_colorless
    {
        return PaymentResult::No;
    }

    // If LOWER bounds meet or exceed the cost for all specific colors,
    // AND we have enough total sources (including flexible) for generic,
    // then we can prove payment is definitely possible

    // Check: do guaranteed sources cover all specific color requirements?
    // The excess from min_X over cost.X can be used for generic costs
    let excess_white = min_white.saturating_sub(cost.white);
    let excess_blue = min_blue.saturating_sub(cost.blue);
    let excess_black = min_black.saturating_sub(cost.black);
    let excess_red = min_red.saturating_sub(cost.red);
    let excess_green = min_green.saturating_sub(cost.green);
    let excess_colorless = min_colorless.saturating_sub(cost.colorless);
    let total_excess = excess_white + excess_blue + excess_black + excess_red + excess_green + excess_colorless;

    // Available for generic = flexible sources + excess from fixed sources
    let available_for_generic = flexible_sources + total_excess;

    if min_white >= cost.white
        && min_blue >= cost.blue
        && min_black >= cost.black
        && min_red >= cost.red
        && min_green >= cost.green
        && min_colorless >= cost.colorless
        && available_for_generic >= cost.generic
    {
        return PaymentResult::Yes;
    }

    // Bounds check passed but not conclusive - might be possible
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
        let bounds_result = bounds_check_payment(cost, sources);
        if bounds_result == PaymentResult::No {
            return PaymentResult::No;
        }

        // Bounds check passed (either Yes or Maybe with simple sources = definite Yes)
        // If no output buffer provided, we can return Yes now
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
        // Use shared bounds checking for quick rejection
        match bounds_check_payment(cost, sources) {
            PaymentResult::No => return PaymentResult::No,
            PaymentResult::Yes | PaymentResult::Maybe => {
                // Bounds check passed - but we must ALWAYS verify with greedy algorithm
                // to ensure consistency between can_pay() and compute_tap_order().
                //
                // BUG FIX: Previously, when tap_order_out was None, we would return Yes
                // immediately based on bounds check alone. But try_greedy_payment() can
                // fail even when bounds check passes (e.g., color allocation conflicts).
                // This caused can_pay() to return true but compute_tap_order() to fail,
                // allowing spells to be offered that couldn't actually be cast.
            }
        }

        // Always verify with greedy algorithm for consistency
        if self.try_greedy_payment(cost, sources, tap_order_out) {
            PaymentResult::Yes
        } else {
            // Greedy failed - bounds check says maybe possible with backtracking
            // but for now we conservatively return Maybe
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
    ///
    /// Internal tracking uses SmallVec to avoid heap allocation. When tap_order_out
    /// is provided, results are copied to it at the end.
    ///
    /// OPT-7: Reuses a single `candidates` buffer across all color iterations
    /// to avoid repeated allocations (was creating 5 SmallVecs per payment).
    fn try_greedy_payment(
        &self,
        cost: &ManaCost,
        sources: &[ManaSource],
        tap_order_out: Option<&mut Vec<CardId>>,
    ) -> bool {
        // Always use SmallVec for tracking - avoids heap allocation for typical mana costs
        // (up to 8 sources covers most spells: 2-3 colored + 4-5 generic)
        let mut tap_order: SmallVec<[CardId; 8]> = SmallVec::new();

        // OPT-7: Reusable buffer for candidates - cleared and reused for each color
        // instead of creating a new SmallVec 5 times per payment
        let mut candidates: SmallVec<[(usize, u8); 8]> = SmallVec::new();

        let mut remaining_cost = *cost;

        // Pay specific color requirements first
        if remaining_cost.white > 0
            && !Self::tap_for_color(
                sources,
                &mut tap_order,
                &mut candidates,
                ManaColor::White,
                remaining_cost.white,
            )
        {
            return false;
        }
        remaining_cost.white = 0;

        if remaining_cost.blue > 0
            && !Self::tap_for_color(
                sources,
                &mut tap_order,
                &mut candidates,
                ManaColor::Blue,
                remaining_cost.blue,
            )
        {
            return false;
        }
        remaining_cost.blue = 0;

        if remaining_cost.black > 0
            && !Self::tap_for_color(
                sources,
                &mut tap_order,
                &mut candidates,
                ManaColor::Black,
                remaining_cost.black,
            )
        {
            return false;
        }
        remaining_cost.black = 0;

        if remaining_cost.red > 0
            && !Self::tap_for_color(
                sources,
                &mut tap_order,
                &mut candidates,
                ManaColor::Red,
                remaining_cost.red,
            )
        {
            return false;
        }
        remaining_cost.red = 0;

        if remaining_cost.green > 0
            && !Self::tap_for_color(
                sources,
                &mut tap_order,
                &mut candidates,
                ManaColor::Green,
                remaining_cost.green,
            )
        {
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

        // If caller wants the tap order, copy it to their buffer
        if let Some(output) = tap_order_out {
            output.clear();
            output.extend(tap_order.iter().copied());
        }

        true
    }

    /// Helper to tap sources for a specific color
    ///
    /// OPT-7: Takes a reusable `candidates` buffer to avoid allocation per color.
    /// The buffer is cleared at the start and reused.
    #[inline]
    fn tap_for_color(
        sources: &[ManaSource],
        tap_order: &mut SmallVec<[CardId; 8]>,
        candidates: &mut SmallVec<[(usize, u8); 8]>,
        color: ManaColor,
        amount: u8,
    ) -> bool {
        // Clear and reuse the candidates buffer (retains capacity)
        candidates.clear();

        // Collect available sources that can produce this color
        for (idx, source) in sources.iter().enumerate() {
            let can_produce = Self::can_produce_color(&source.production, color);
            if !source.is_tapped
                && !source.has_summoning_sickness
                && !tap_order.contains(&source.card_id)
                && can_produce
            {
                candidates.push((idx, Self::score_for_color(&source.production, color)));
            }
        }

        // Sort by score (lower = more specific = tap first)
        candidates.sort_by_key(|(_, score)| *score);

        // Tap sources in priority order
        let mut tapped = 0u8;
        for &(idx, _score) in candidates.iter() {
            if tapped >= amount {
                break;
            }
            tap_order.push(sources[idx].card_id);
            tapped += 1;
        }

        tapped >= amount
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
    fn test_quick_check_can_return_yes_with_lower_bounds() {
        let resolver = SimpleManaResolver::new();

        // With a fixed source (Mountain), we GUARANTEE 1 red mana
        // Lower bound: min_red = 1
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

        // quick_check CAN return Yes when lower bounds prove payment is possible
        let result = resolver.quick_check(&cost, &sources);
        assert_eq!(result, PaymentResult::Yes); // Lower bound min_red=1 >= cost.red=1
    }

    #[test]
    fn test_quick_check_returns_maybe_for_dual_lands() {
        let resolver = GreedyManaResolver::new();

        // With a Choice source (Breeding Pool), we can't guarantee either color
        // Lower bounds: min_blue = 0, min_green = 0
        // Upper bounds: max_blue = 1, max_green = 1
        let sources = vec![ManaSource {
            card_id: CardId::new(1),
            production: ManaProduction::free(ManaProductionKind::Choice(
                ManaColors::new().with(ManaColor::Blue).with(ManaColor::Green),
            )),
            is_tapped: false,
            has_summoning_sickness: false,
        }];

        // Cost: 1 blue
        let cost = ManaCost {
            generic: 0,
            white: 0,
            blue: 1,
            black: 0,
            red: 0,
            green: 0,
            colorless: 0,
            x_count: 0,
        };

        // quick_check returns Maybe because lower bounds don't prove it
        // (but the greedy algorithm will find a solution)
        let result = resolver.quick_check(&cost, &sources);
        assert_eq!(result, PaymentResult::Maybe);

        // But check_payment should return Yes after running greedy
        let result2 = resolver.check_payment(&cost, &sources, None);
        assert_eq!(result2, PaymentResult::Yes);
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

    #[test]
    fn test_dual_land_upper_bound_contributes_to_both_colors() {
        let resolver = GreedyManaResolver::new();

        // Breeding Pool (dual land: U or G)
        // Upper bound: +1 to blue, +1 to green
        // Lower bound: 0 to any specific color (can only produce one at a time)
        let breeding_pool = ManaSource {
            card_id: CardId::new(1),
            production: ManaProduction::free(ManaProductionKind::Choice(
                ManaColors::new().with(ManaColor::Blue).with(ManaColor::Green),
            )),
            is_tapped: false,
            has_summoning_sickness: false,
        };

        // Test 1: Can pay UG with just Breeding Pool? NO - only produces 1 mana total
        // The dual land contributes +1 to blue upper bound AND +1 to green upper bound,
        // but it can only produce ONE mana total (net delta = 1)
        let cost_ug = ManaCost {
            generic: 0,
            white: 0,
            blue: 1,
            black: 0,
            red: 0,
            green: 1,
            colorless: 0,
            x_count: 0,
        };

        let result = resolver.check_payment(&cost_ug, std::slice::from_ref(&breeding_pool), None);
        // Should fail: even though upper bounds for both colors pass (max_blue=1, max_green=1),
        // total mana available is only 1, but cost needs 2
        assert_eq!(result, PaymentResult::No);

        // Test 2: With 2 Breeding Pools, can pay UG? YES
        let breeding_pool_2 = ManaSource {
            card_id: CardId::new(2),
            production: ManaProduction::free(ManaProductionKind::Choice(
                ManaColors::new().with(ManaColor::Blue).with(ManaColor::Green),
            )),
            is_tapped: false,
            has_summoning_sickness: false,
        };
        let sources = vec![breeding_pool.clone(), breeding_pool_2];

        assert!(resolver.can_pay(&cost_ug, &sources));
        let mut tap_order = Vec::new();
        assert!(resolver.compute_tap_order(&cost_ug, &sources, &mut tap_order));
        assert_eq!(tap_order.len(), 2);

        // Test 3: With 1 Breeding Pool + 1 Island, can pay UG? YES
        let sources = vec![
            breeding_pool.clone(),
            ManaSource {
                card_id: CardId::new(3),
                production: ManaProduction::free(ManaProductionKind::Fixed(ManaColor::Blue)),
                is_tapped: false,
                has_summoning_sickness: false,
            },
        ];

        assert!(resolver.can_pay(&cost_ug, &sources));
        let mut tap_order = Vec::new();
        assert!(resolver.compute_tap_order(&cost_ug, &sources, &mut tap_order));
        assert_eq!(tap_order.len(), 2);
        // Island should be tapped for Blue, Breeding Pool for Green
        assert_eq!(tap_order[0], CardId::new(3)); // Island for Blue (more specific)
        assert_eq!(tap_order[1], CardId::new(1)); // Breeding Pool for Green
    }

    /// Test for mana payment bug: 3 green sources cannot pay 3G cost
    ///
    /// Regression test for bug where AI offered uncastable spells.
    /// A cost of "3G" means 3 generic + 1 green = 4 total mana.
    /// With only 3 green sources, we can only produce 3 mana, which is insufficient.
    #[test]
    fn test_insufficient_sources_for_generic_plus_color_cost() {
        let resolver = GreedyManaResolver::new();

        // 3 green mana sources (e.g., 3 Thriving Groves or Forests)
        let sources = vec![
            ManaSource {
                card_id: CardId::new(1),
                production: ManaProduction::free(ManaProductionKind::Fixed(ManaColor::Green)),
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
                production: ManaProduction::free(ManaProductionKind::Fixed(ManaColor::Green)),
                is_tapped: false,
                has_summoning_sickness: false,
            },
        ];

        // Cost: 3G = 3 generic + 1 green = 4 total mana
        // We only have 3 sources, so this should NOT be payable
        let cost = ManaCost::from_string("3G");
        assert_eq!(cost.generic, 3, "3G should have generic=3");
        assert_eq!(cost.green, 1, "3G should have green=1");
        assert_eq!(cost.cmc(), 4, "3G should have cmc=4");

        // This MUST return false - we only have 3 mana but need 4
        assert!(
            !resolver.can_pay(&cost, &sources),
            "3 green sources should NOT be able to pay 3G (needs 4 mana, has 3)"
        );

        // Verify with check_payment as well
        let mut tap_order = Vec::new();
        let result = resolver.check_payment(&cost, &sources, Some(&mut tap_order));
        assert_eq!(
            result,
            PaymentResult::No,
            "check_payment should return No for insufficient sources"
        );

        // Now verify 4 sources CAN pay for 3G
        let sources_4 = vec![
            ManaSource {
                card_id: CardId::new(1),
                production: ManaProduction::free(ManaProductionKind::Fixed(ManaColor::Green)),
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
                production: ManaProduction::free(ManaProductionKind::Fixed(ManaColor::Green)),
                is_tapped: false,
                has_summoning_sickness: false,
            },
            ManaSource {
                card_id: CardId::new(4),
                production: ManaProduction::free(ManaProductionKind::Fixed(ManaColor::Green)),
                is_tapped: false,
                has_summoning_sickness: false,
            },
        ];

        assert!(
            resolver.can_pay(&cost, &sources_4),
            "4 green sources should be able to pay 3G (needs 4 mana, has 4)"
        );

        let mut tap_order = Vec::new();
        assert!(
            resolver.compute_tap_order(&cost, &sources_4, &mut tap_order),
            "compute_tap_order should succeed with 4 sources"
        );
        assert_eq!(tap_order.len(), 4, "All 4 sources should be tapped to pay 3G");
    }
}
