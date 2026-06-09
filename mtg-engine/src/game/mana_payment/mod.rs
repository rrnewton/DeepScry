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
//! use mtg_engine::game::mana_payment::{ManaSource, ManaPaymentResolver, SimpleManaResolver};
//! use mtg_engine::core::ManaCost;
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

// Mana-payment EXECUTION on the live `GameState` (tap-for-mana, pay-ability-cost).
// Split out of `game/actions/mod.rs`; see `README.md` for the resolver-vs-execution
// split. The methods are inherent `impl GameState` methods, so there is nothing to
// re-export here — they attach to `GameState` directly.
mod payment_execution;

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
    // Compute both lower bounds (guaranteed) and upper bounds (potential) for each color,
    // plus total available delta - all in a single pass over sources.
    // Lower bound: Only Fixed sources guarantee a specific color
    // Upper bound: Fixed + Choice + AnyColor all potentially produce each color
    let mut available_delta: i16 = 0;
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

        // Accumulate net delta for total mana check
        available_delta += i16::from(source.production.net_delta());

        // Each activation of this source produces `n` mana. For Plains/Mox this is
        // 1; for Sol Ring it is 2; for Black Lotus it is 3. We must multiply the
        // bounds contribution by `n`, otherwise a single Black Lotus would only
        // appear to provide one mana of any color and a 3-cost spell would look
        // unpayable.
        let n = source.production.amount;
        match &source.production.kind {
            ManaProductionKind::Fixed(color) => {
                // Fixed sources contribute to both lower and upper bounds
                match color {
                    ManaColor::White => {
                        min_white += n;
                        max_white += n;
                    }
                    ManaColor::Blue => {
                        min_blue += n;
                        max_blue += n;
                    }
                    ManaColor::Black => {
                        min_black += n;
                        max_black += n;
                    }
                    ManaColor::Red => {
                        min_red += n;
                        max_red += n;
                    }
                    ManaColor::Green => {
                        min_green += n;
                        max_green += n;
                    }
                }
            }
            ManaProductionKind::Colorless => {
                // Colorless contributes to both bounds for colorless requirement
                min_colorless += n;
                max_colorless += n;
            }
            ManaProductionKind::Choice(colors) => {
                // Choice lands only contribute to UPPER bounds (no guarantee of specific color)
                // But they do contribute to flexible sources for generic costs.
                // A single activation provides `n` mana of one chosen colour from the set.
                flexible_sources = flexible_sources.saturating_add(n);
                for color in colors.iter() {
                    match color {
                        ManaColor::White => max_white += n,
                        ManaColor::Blue => max_blue += n,
                        ManaColor::Black => max_black += n,
                        ManaColor::Red => max_red += n,
                        ManaColor::Green => max_green += n,
                    }
                }
            }
            ManaProductionKind::AnyColor => {
                // Any-color only contributes to UPPER bounds
                flexible_sources = flexible_sources.saturating_add(n);
                max_white += n;
                max_blue += n;
                max_black += n;
                max_red += n;
                max_green += n;
            }
        }
    }

    // Check total available mana (accounting for activation costs via net delta)
    let needed = cost
        .white
        .saturating_add(cost.blue)
        .saturating_add(cost.black)
        .saturating_add(cost.red)
        .saturating_add(cost.green)
        .saturating_add(cost.colorless)
        .saturating_add(cost.generic);

    // Can only prove "No" if the total delta is insufficient
    if available_delta < i16::from(needed) {
        return PaymentResult::No;
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
    /// Note: Wildcard is intentional - SimpleManaResolver only handles Fixed/Colorless
    /// kinds; other ManaProductionKind variants are skipped (they indicate complex sources).
    #[allow(clippy::wildcard_enum_match_arm)]
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

        // Multi-mana sources (Sol Ring → 2) can over-produce when tapped for a
        // smaller colored/colorless requirement. We track the surplus and
        // consume it before tapping additional sources for generic.
        let mut extra_generic: u8 = 0;

        // Helper to tap sources of a specific color and accumulate any surplus.
        // Returns the total mana actually tapped (may exceed `amount`).
        let mut tap_color = |color: ManaColor, amount: u8, sources: &[ManaSource]| -> u8 {
            if amount == 0 {
                return 0;
            }
            let mut tapped: u8 = 0;
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
                        tapped = tapped.saturating_add(source.production.amount);
                    }
                }
            }
            tapped
        };

        // Tap sources for specific color requirements first; bank any surplus
        // into extra_generic.
        let t = tap_color(ManaColor::White, remaining_cost.white, sources);
        extra_generic = extra_generic.saturating_add(t.saturating_sub(remaining_cost.white));
        remaining_cost.white = 0;

        let t = tap_color(ManaColor::Blue, remaining_cost.blue, sources);
        extra_generic = extra_generic.saturating_add(t.saturating_sub(remaining_cost.blue));
        remaining_cost.blue = 0;

        let t = tap_color(ManaColor::Black, remaining_cost.black, sources);
        extra_generic = extra_generic.saturating_add(t.saturating_sub(remaining_cost.black));
        remaining_cost.black = 0;

        let t = tap_color(ManaColor::Red, remaining_cost.red, sources);
        extra_generic = extra_generic.saturating_add(t.saturating_sub(remaining_cost.red));
        remaining_cost.red = 0;

        let t = tap_color(ManaColor::Green, remaining_cost.green, sources);
        extra_generic = extra_generic.saturating_add(t.saturating_sub(remaining_cost.green));
        remaining_cost.green = 0;

        // Tap colorless sources for colorless requirement. Each Sol Ring tap
        // contributes 2 colorless; the surplus drops into extra_generic.
        let mut tapped_colorless: u8 = 0;
        for source in sources {
            if tapped_colorless >= remaining_cost.colorless {
                break;
            }
            if source.is_tapped || source.has_summoning_sickness || tap_order.contains(&source.card_id) {
                continue;
            }
            if source.production.kind == ManaProductionKind::Colorless {
                tap_order.push(source.card_id);
                tapped_colorless = tapped_colorless.saturating_add(source.production.amount);
            }
        }
        extra_generic = extra_generic.saturating_add(tapped_colorless.saturating_sub(remaining_cost.colorless));
        remaining_cost.colorless = 0;

        // Apply banked surplus to generic before tapping more sources.
        if remaining_cost.generic > 0 {
            let from_extra = remaining_cost.generic.min(extra_generic);
            remaining_cost.generic -= from_extra;
        }

        // Tap any remaining sources for generic cost; each tap contributes its
        // full per-activation amount.
        let mut tapped_generic: u8 = 0;
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
                    tapped_generic = tapped_generic.saturating_add(source.production.amount);
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

    /// Score a source for a specific color (lower = better = more specific).
    ///
    /// The score combines two dimensions:
    ///
    /// - **Color specificity** (low byte): exact `Fixed(color)` is best, then
    ///   dual/multi-color `Choice` (smaller choice = better), then `AnyColor`.
    /// - **Side cost** (high byte): plain lands < utility lands < pain lands
    ///   < sacrifice sources. We multiply the side-cost severity into the
    ///   score so it dominates color specificity. That way Mox Emerald (Fixed
    ///   Green, no side cost) is always preferred over Black Lotus (AnyColor,
    ///   sacrifices the source) when both can pay a green pip.
    ///
    /// This is the score used by `tap_for_color` — see also `generic_score`
    /// which orders the generic-pip phase.
    fn score_for_color(production: &ManaProduction, color: ManaColor) -> u16 {
        let kind_score: u16 = match &production.kind {
            ManaProductionKind::Fixed(c) if *c == color => 0, // Best: exact match
            ManaProductionKind::Choice(colors) if colors.contains(color) => {
                colors.len() as u16 // Better: dual land (prefer fewer options)
            }
            ManaProductionKind::AnyColor => 100, // Worst: save for last resort
            // Anything else can't produce this color and shouldn't be scored.
            ManaProductionKind::Fixed(_) | ManaProductionKind::Choice(_) | ManaProductionKind::Colorless => u16::MAX,
        };
        // Side cost dominates color specificity: a plain land of any kind
        // beats every pain/sacrifice source. We saturate to keep things
        // monotone.
        production.side_cost_score().saturating_add(kind_score)
    }

    /// Score a source for the **generic** phase. Used to deprioritize
    /// expensive sources (sacrifice / pain / utility lands / multi-mana
    /// over-tap) when paying generic pips.
    ///
    /// Ordering rules (lowest = tap first):
    /// 1. Plain free sources (basic lands, Moxen, dual lands).
    /// 2. Utility lands (Mishra's Factory, Strip Mine).
    /// 3. Pain lands (City of Brass).
    /// 4. Multi-mana sources with `amount > 1` are weighted up so we don't
    ///    waste a Sol Ring / Black Lotus on a single generic pip when single-
    ///    mana sources suffice.
    /// 5. Sacrifice sources (Black Lotus, Treasure tokens) absolutely last.
    fn generic_score(production: &ManaProduction) -> u16 {
        let mut score = production.side_cost_score();
        // Penalize over-production: every "extra" mana beyond 1 adds a small
        // surcharge so a 1-mana source beats a 3-mana source for a single
        // generic pip. The side-cost score already swamps this for
        // sacrifice/pain sources, so this only matters between equals.
        if production.amount > 1 {
            score = score.saturating_add(u16::from(production.amount - 1) * 2);
        }
        // Mild preference: Fixed/Colorless (single-color) over Choice/AnyColor
        // for generic, so we save flexible sources for later colored needs.
        let kind_bias: u16 = match &production.kind {
            ManaProductionKind::Fixed(_) | ManaProductionKind::Colorless => 0,
            ManaProductionKind::Choice(colors) => colors.len() as u16,
            ManaProductionKind::AnyColor => 4,
        };
        score.saturating_add(kind_bias)
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
        // instead of creating a new SmallVec 5 times per payment.
        // Score is u16 because we now combine kind specificity (low byte) with
        // side-cost severity (high byte) — see `score_for_color`.
        let mut candidates: SmallVec<[(usize, u16); 8]> = SmallVec::new();

        let mut remaining_cost = *cost;

        // Multi-mana sources (Sol Ring → 2, Black Lotus → 3) can over-produce
        // when tapped to satisfy a smaller colored or colorless requirement;
        // the leftover pips can then cover generic. We accumulate these spares
        // in `extra_generic` and apply them before tapping additional sources
        // for generic.
        let mut extra_generic: u8 = 0;

        // Pay specific color requirements first
        if remaining_cost.white > 0 {
            match Self::tap_for_color(
                sources,
                &mut tap_order,
                &mut candidates,
                ManaColor::White,
                remaining_cost.white,
            ) {
                Some(tapped) => {
                    extra_generic = extra_generic.saturating_add(tapped.saturating_sub(remaining_cost.white));
                }
                None => return false,
            }
        }
        remaining_cost.white = 0;

        if remaining_cost.blue > 0 {
            match Self::tap_for_color(
                sources,
                &mut tap_order,
                &mut candidates,
                ManaColor::Blue,
                remaining_cost.blue,
            ) {
                Some(tapped) => {
                    extra_generic = extra_generic.saturating_add(tapped.saturating_sub(remaining_cost.blue));
                }
                None => return false,
            }
        }
        remaining_cost.blue = 0;

        if remaining_cost.black > 0 {
            match Self::tap_for_color(
                sources,
                &mut tap_order,
                &mut candidates,
                ManaColor::Black,
                remaining_cost.black,
            ) {
                Some(tapped) => {
                    extra_generic = extra_generic.saturating_add(tapped.saturating_sub(remaining_cost.black));
                }
                None => return false,
            }
        }
        remaining_cost.black = 0;

        if remaining_cost.red > 0 {
            match Self::tap_for_color(
                sources,
                &mut tap_order,
                &mut candidates,
                ManaColor::Red,
                remaining_cost.red,
            ) {
                Some(tapped) => {
                    extra_generic = extra_generic.saturating_add(tapped.saturating_sub(remaining_cost.red));
                }
                None => return false,
            }
        }
        remaining_cost.red = 0;

        if remaining_cost.green > 0 {
            match Self::tap_for_color(
                sources,
                &mut tap_order,
                &mut candidates,
                ManaColor::Green,
                remaining_cost.green,
            ) {
                Some(tapped) => {
                    extra_generic = extra_generic.saturating_add(tapped.saturating_sub(remaining_cost.green));
                }
                None => return false,
            }
        }
        remaining_cost.green = 0;

        // Pay colorless requirement with colorless sources. We sort the
        // colorless candidates by `generic_score` so utility/sacrifice
        // colorless sources (rare but possible) tap last among colorless.
        // Each tap contributes `source.production.amount` pips (Sol Ring → 2);
        // surplus drops into `extra_generic` for the generic phase below.
        if remaining_cost.colorless > 0 {
            candidates.clear();
            for (idx, source) in sources.iter().enumerate() {
                if source.is_tapped || source.has_summoning_sickness || tap_order.contains(&source.card_id) {
                    continue;
                }
                if source.production.kind == ManaProductionKind::Colorless {
                    candidates.push((idx, Self::generic_score(&source.production)));
                }
            }
            candidates.sort_by_key(|(_, score)| *score);

            let mut tapped: u8 = 0;
            for &(idx, _) in candidates.iter() {
                if tapped >= remaining_cost.colorless {
                    break;
                }
                tap_order.push(sources[idx].card_id);
                tapped = tapped.saturating_add(sources[idx].production.amount);
            }
            if tapped < remaining_cost.colorless {
                return false;
            }
            extra_generic = extra_generic.saturating_add(tapped.saturating_sub(remaining_cost.colorless));
        }
        remaining_cost.colorless = 0;

        // Pay generic cost with any remaining sources, first consuming spare
        // pips already produced by over-tapping for colored/colorless above.
        if remaining_cost.generic > 0 {
            let from_extra = remaining_cost.generic.min(extra_generic);
            remaining_cost.generic -= from_extra;
            // (extra_generic bookkeeping not needed past this point)
        }
        if remaining_cost.generic > 0 {
            // Use the priority-aware helper instead of iterating in source
            // order — that's what causes Black Lotus / Mishra's Factory to be
            // tapped wastefully. The helper sorts by `generic_score`:
            //   plain land < utility land < pain land < sacrifice source,
            // and prefers single-mana over multi-mana within each tier so we
            // don't burn a Sol Ring on one generic pip when a Forest will do.
            if Self::tap_for_generic(sources, &mut tap_order, &mut candidates, remaining_cost.generic).is_none() {
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
    ///
    /// Returns `Some(total_mana_tapped)` if the requested amount was satisfied
    /// (the value may exceed `amount` when a multi-mana source like Black Lotus
    /// produces more pips than needed — caller credits the surplus to generic),
    /// or `None` if not enough sources of the requested colour are available.
    #[inline]
    fn tap_for_color(
        sources: &[ManaSource],
        tap_order: &mut SmallVec<[CardId; 8]>,
        candidates: &mut SmallVec<[(usize, u16); 8]>,
        color: ManaColor,
        amount: u8,
    ) -> Option<u8> {
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

        // Sort by score (lower = more specific / cheaper = tap first).
        // The score now bakes in side-cost severity (sacrifice / pain) so
        // a Mountain beats a Black Lotus when both can pay {R}.
        candidates.sort_by_key(|(_, score)| *score);

        // Tap sources in priority order. Each activation contributes
        // `source.production.amount` mana of the chosen colour (Black Lotus = 3).
        let mut tapped: u8 = 0;
        for &(idx, _score) in candidates.iter() {
            if tapped >= amount {
                break;
            }
            let n = sources[idx].production.amount;
            tap_order.push(sources[idx].card_id);
            tapped = tapped.saturating_add(n);
        }

        if tapped >= amount {
            Some(tapped)
        } else {
            None
        }
    }

    /// Helper to tap sources for **generic** mana, in priority order.
    ///
    /// Unlike the colored phases, this iterated sources in their input order
    /// before — which led to the Black Lotus / Mishra's Factory bugs where
    /// expensive sources got tapped early just because they appeared first in
    /// the cache. Now we collect candidates and sort by `generic_score` so
    /// plain free lands tap before utility/pain/sacrifice sources, and small
    /// sources are spent before multi-mana ones.
    ///
    /// Returns `Some(total_mana_tapped)` if the requested generic amount is
    /// satisfied (may exceed `amount` if a multi-mana source over-pays), or
    /// `None` otherwise.
    #[inline]
    fn tap_for_generic(
        sources: &[ManaSource],
        tap_order: &mut SmallVec<[CardId; 8]>,
        candidates: &mut SmallVec<[(usize, u16); 8]>,
        amount: u8,
    ) -> Option<u8> {
        candidates.clear();

        for (idx, source) in sources.iter().enumerate() {
            if source.is_tapped || source.has_summoning_sickness || tap_order.contains(&source.card_id) {
                continue;
            }
            // Sources that produce *no* mana shouldn't appear in `sources` in
            // the first place, but be defensive.
            if !source.production.produces_mana() {
                continue;
            }
            candidates.push((idx, Self::generic_score(&source.production)));
        }

        candidates.sort_by_key(|(_, score)| *score);

        let mut tapped: u8 = 0;
        for &(idx, _score) in candidates.iter() {
            if tapped >= amount {
                break;
            }
            let n = sources[idx].production.amount;
            tap_order.push(sources[idx].card_id);
            tapped = tapped.saturating_add(n);
        }

        if tapped >= amount {
            Some(tapped)
        } else {
            None
        }
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

        // Positive delta: Sol Ring ({T}: Add {C}{C}) — amount=2, costs 0 = +2.
        // After the multi-mana fix, `net_delta` reflects the per-activation
        // amount instead of always returning 1.
        let sol_ring = ManaProduction::with_amount(ManaProductionKind::Colorless, 2);
        assert_eq!(sol_ring.net_delta(), 2);

        // Black Lotus: AnyColor with amount=3, no cost = +3 delta.
        let black_lotus = ManaProduction::with_amount(ManaProductionKind::AnyColor, 3);
        assert_eq!(black_lotus.net_delta(), 3);

        // Default amount is 1.
        let free_colorless = ManaProduction::free(ManaProductionKind::Colorless);
        assert_eq!(free_colorless.net_delta(), 1);

        // Zero delta: Mana Prism ({1}, {T}: Add one mana of any color) - produces 1, costs 1 = 0 delta
        let zero_delta = ManaProduction::with_cost(ManaProductionKind::AnyColor, ManaCost::from_string("1"));
        assert_eq!(zero_delta.net_delta(), 0);

        // Negative delta: Celestial Prism ({2}, {T}: Add one mana of any color) - produces 1, costs 2 = -1 delta
        let negative_delta = ManaProduction::with_cost(ManaProductionKind::AnyColor, ManaCost::from_string("2"));
        assert_eq!(negative_delta.net_delta(), -1);
    }

    /// Regression: Black Lotus must satisfy a 3-mana spell on its own.
    /// Pre-fix, `bounds_check_payment` only counted Black Lotus as 1 mana of
    /// any color, so casting Su-Chi (`{4}` ≥ 3) or Psionic Blast (`{2}{U}`)
    /// off a single Black Lotus appeared impossible — the spell never
    /// surfaced as a castable action. Two assertions:
    ///   1. `{2}{U}` is payable from a single AnyColor Amount=3 source.
    ///   2. `{3}` (pure generic) is also payable.
    ///
    /// And the negative case: `{4}` is NOT payable from one Black Lotus alone.
    #[test]
    fn test_greedy_resolver_black_lotus_pays_three_mana_spell() {
        let resolver = GreedyManaResolver::new();
        let lotus = vec![ManaSource {
            card_id: CardId::new(1),
            production: ManaProduction::with_amount(ManaProductionKind::AnyColor, 3),
            is_tapped: false,
            has_summoning_sickness: false,
        }];

        // Psionic Blast: {2}{U} = 1 blue + 2 generic
        let psionic = ManaCost {
            generic: 2,
            white: 0,
            blue: 1,
            black: 0,
            red: 0,
            green: 0,
            colorless: 0,
            x_count: 0,
        };
        assert_eq!(resolver.check_payment(&psionic, &lotus, None), PaymentResult::Yes);

        // Su-Chi: {3} = pure generic; one Lotus tap covers it.
        let su_chi = ManaCost {
            generic: 3,
            white: 0,
            blue: 0,
            black: 0,
            red: 0,
            green: 0,
            colorless: 0,
            x_count: 0,
        };
        assert_eq!(resolver.check_payment(&su_chi, &lotus, None), PaymentResult::Yes);

        // {4} is NOT payable from a single Black Lotus (only 3 mana available).
        let four_cost = ManaCost {
            generic: 4,
            white: 0,
            blue: 0,
            black: 0,
            red: 0,
            green: 0,
            colorless: 0,
            x_count: 0,
        };
        assert_eq!(resolver.check_payment(&four_cost, &lotus, None), PaymentResult::No);
    }

    /// Regression: Sol Ring's `Amount$ 2` colorless production must be counted
    /// as 2 mana, not 1. A single Sol Ring should be able to pay {2}.
    #[test]
    fn test_simple_resolver_sol_ring_pays_two_generic() {
        let resolver = SimpleManaResolver::new();
        let sol_ring = vec![ManaSource {
            card_id: CardId::new(1),
            production: ManaProduction::with_amount(ManaProductionKind::Colorless, 2),
            is_tapped: false,
            has_summoning_sickness: false,
        }];

        let two_cost = ManaCost {
            generic: 2,
            white: 0,
            blue: 0,
            black: 0,
            red: 0,
            green: 0,
            colorless: 0,
            x_count: 0,
        };
        assert_eq!(resolver.check_payment(&two_cost, &sol_ring, None), PaymentResult::Yes);
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
            breeding_pool,
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

    /// Regression: with Underground Sea + Tundra + Mox Emerald + Black Lotus,
    /// casting Psionic Blast `{2}{U}` MUST NOT sacrifice the Black Lotus.
    /// The greedy resolver should prefer cheaper, non-sacrifice sources first.
    ///
    /// Source ordering used by ManaEngine::read_from_cache puts simple (Fixed)
    /// sources before complex sources, so the iteration order here mirrors what
    /// the engine actually sees:
    ///   1. Mox Emerald (Fixed Green, simple)
    ///   2. Underground Sea (Choice U|B, complex)
    ///   3. Tundra (Choice W|U, complex)
    ///   4. Black Lotus (AnyColor amount=3, sacrifice cost, complex)
    #[test]
    fn test_greedy_resolver_prefers_non_sacrifice_sources() {
        let resolver = GreedyManaResolver::new();

        let mox_emerald = ManaSource {
            card_id: CardId::new(1),
            production: ManaProduction::free(ManaProductionKind::Fixed(ManaColor::Green)),
            is_tapped: false,
            has_summoning_sickness: false,
        };
        let underground_sea = ManaSource {
            card_id: CardId::new(2),
            production: ManaProduction::free(ManaProductionKind::Choice(
                ManaColors::new().with(ManaColor::Blue).with(ManaColor::Black),
            )),
            is_tapped: false,
            has_summoning_sickness: false,
        };
        let tundra = ManaSource {
            card_id: CardId::new(3),
            production: ManaProduction::free(ManaProductionKind::Choice(
                ManaColors::new().with(ManaColor::White).with(ManaColor::Blue),
            )),
            is_tapped: false,
            has_summoning_sickness: false,
        };
        // Sacrifice cost on the Lotus — encoded by adding a non-mana payment cost
        // (we model it by giving the source a non-zero activation cost).
        // For now, the resolver only sees the production kind/amount, so we
        // simply mark the production as a multi-mana AnyColor source with a
        // large `amount` and rely on the resolver's preference logic.
        let black_lotus = ManaSource {
            card_id: CardId::new(4),
            production: ManaProduction::with_amount(ManaProductionKind::AnyColor, 3),
            is_tapped: false,
            has_summoning_sickness: false,
        };

        // Order matches what ManaSourceCache produces: simple (green) first,
        // then complex sources in cache insertion order.
        let sources = vec![mox_emerald, underground_sea, tundra, black_lotus];

        // Psionic Blast: {2}{U} = 1 blue + 2 generic
        let psionic = ManaCost {
            generic: 2,
            white: 0,
            blue: 1,
            black: 0,
            red: 0,
            green: 0,
            colorless: 0,
            x_count: 0,
        };

        let mut tap_order = Vec::new();
        assert!(
            resolver.compute_tap_order(&psionic, &sources, &mut tap_order),
            "should be payable without Black Lotus"
        );

        // CRITICAL: Black Lotus (CardId 4) MUST NOT be in the tap order.
        assert!(
            !tap_order.contains(&CardId::new(4)),
            "Black Lotus should not be tapped when cheaper sources suffice. Got: {:?}",
            tap_order
        );

        // Tap order should be exactly 3 sources (one per pip).
        assert_eq!(tap_order.len(), 3, "Got tap order: {:?}", tap_order);
    }

    /// Regression: when *only* Underground Sea, Tundra and Black Lotus are
    /// available (no Mox), the resolver previously tapped Black Lotus for the
    /// last generic pip (because complex sources were iterated in cache order
    /// and Lotus came after the dual lands). With the side-cost-aware
    /// `tap_for_generic` ordering, Black Lotus must still be preserved — the
    /// dual lands cover both the colored and generic phases.
    #[test]
    fn test_greedy_resolver_avoids_lotus_when_duals_suffice() {
        let resolver = GreedyManaResolver::new();
        let underground_sea = ManaSource {
            card_id: CardId::new(1),
            production: ManaProduction::free(ManaProductionKind::Choice(
                ManaColors::new().with(ManaColor::Blue).with(ManaColor::Black),
            )),
            is_tapped: false,
            has_summoning_sickness: false,
        };
        let tundra = ManaSource {
            card_id: CardId::new(2),
            production: ManaProduction::free(ManaProductionKind::Choice(
                ManaColors::new().with(ManaColor::White).with(ManaColor::Blue),
            )),
            is_tapped: false,
            has_summoning_sickness: false,
        };
        // Black Lotus produces 3 mana of any color but sacrifices itself.
        let black_lotus = ManaSource {
            card_id: CardId::new(3),
            production: ManaProduction::with_amount(ManaProductionKind::AnyColor, 3)
                .with_side_cost(crate::core::ManaSideCost::Sacrifice),
            is_tapped: false,
            has_summoning_sickness: false,
        };
        let sources = vec![underground_sea, tundra, black_lotus];

        // {1}{U}: 1 blue + 1 generic = 2 mana total. Both duals are needed.
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

        let mut tap_order = Vec::new();
        assert!(resolver.compute_tap_order(&cost, &sources, &mut tap_order));
        assert!(
            !tap_order.contains(&CardId::new(3)),
            "Black Lotus must NOT be tapped when 2 dual lands cover {{1}}{{U}}. Got: {:?}",
            tap_order
        );
    }

    /// Regression: Mishra's Factory should be tapped LAST among free lands so
    /// its `{1}: become a 2/2 Assembly-Worker creature` ability stays usable.
    /// With the side-cost-aware `generic_score`, Utility lands rank above
    /// plain lands (Forest) but well below pain/sacrifice sources.
    #[test]
    fn test_greedy_resolver_prefers_basic_over_utility_land() {
        let resolver = GreedyManaResolver::new();

        let forest = ManaSource {
            card_id: CardId::new(1),
            production: ManaProduction::free(ManaProductionKind::Fixed(ManaColor::Green)),
            is_tapped: false,
            has_summoning_sickness: false,
        };
        let mishras_factory = ManaSource {
            card_id: CardId::new(2),
            production: ManaProduction::free(ManaProductionKind::Colorless)
                .with_side_cost(crate::core::ManaSideCost::Utility),
            is_tapped: false,
            has_summoning_sickness: false,
        };

        // Cost: {1} (one generic). Either land covers it; Forest should win.
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

        let mut tap_order = Vec::new();
        assert!(resolver.compute_tap_order(
            &cost,
            &sources(&[forest.clone(), mishras_factory.clone()]),
            &mut tap_order
        ));
        assert_eq!(
            tap_order,
            vec![CardId::new(1)],
            "Forest must tap before Mishra's Factory"
        );

        // Swap input order — Utility should still rank below plain.
        let mut tap_order = Vec::new();
        assert!(resolver.compute_tap_order(&cost, &sources(&[mishras_factory, forest]), &mut tap_order));
        assert_eq!(
            tap_order,
            vec![CardId::new(1)],
            "Forest must tap before Mishra's Factory regardless of source order"
        );

        fn sources(s: &[ManaSource]) -> Vec<ManaSource> {
            s.to_vec()
        }
    }

    /// Regression: pain lands (City of Brass) tap *after* plain lands. With
    /// a Forest and a City of Brass and a {1} cost, Forest wins.
    #[test]
    fn test_greedy_resolver_prefers_basic_over_pain_land() {
        let resolver = GreedyManaResolver::new();
        let forest = ManaSource {
            card_id: CardId::new(1),
            production: ManaProduction::free(ManaProductionKind::Fixed(ManaColor::Green)),
            is_tapped: false,
            has_summoning_sickness: false,
        };
        let city_of_brass = ManaSource {
            card_id: CardId::new(2),
            production: ManaProduction::free(ManaProductionKind::AnyColor)
                .with_side_cost(crate::core::ManaSideCost::PayLife(1)),
            is_tapped: false,
            has_summoning_sickness: false,
        };

        let cost = ManaCost::from_string("1");
        let mut tap_order = Vec::new();
        assert!(resolver.compute_tap_order(&cost, &[forest, city_of_brass], &mut tap_order));
        assert_eq!(tap_order, vec![CardId::new(1)], "Forest must tap before City of Brass");
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
