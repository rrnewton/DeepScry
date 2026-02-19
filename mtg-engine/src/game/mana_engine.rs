//! Mana availability computation and cost payment checking
//!
//! This module provides efficient querying of whether a player can produce
//! enough mana to pay a given cost. It maintains cached state of available
//! mana sources partitioned into simple and complex sources.
//!
//! # Architecture
//!
//! The mana engine operates in two phases:
//!
//! 1. **Update Phase**: Scans the battlefield to identify and cache mana-producing permanents
//! 2. **Query Phase**: Answers questions about whether specific costs can be paid
//!
//! ## Mana Source Classification
//!
//! - **Simple sources**: Lands that produce a single specific color (e.g., Mountain → R, Plains → W)
//!   - Cached as `ManaCapacity` counters (WUBRGC)
//!   - O(1) query time - just compare counts
//!   - Currently supports: Plains, Island, Swamp, Mountain, Forest, Wastes
//!
//! - **Complex sources**: Lands with choices or conditional costs (e.g., City of Brass → any color)
//!   - Stored as list of `CardId`s for future search
//!   - Not yet implemented - requires search algorithm
//!   - Examples: dual lands, fetch lands, City of Brass
//!
//! ## Performance Characteristics
//!
//! - **Update**: O(n) where n = number of battlefield permanents
//!   - Linear scan of battlefield
//!   - Should be called when permanents enter/leave or tap/untap
//!   - Not called on every mana payment - only when state changes
//!
//! - **Query (simple sources only)**: O(1)
//!   - Just arithmetic comparisons of cached counters
//!   - Critical path for spell selection AI
//!
//! - **Query (with complex sources)**: Not yet implemented
//!   - Will require small search (likely << 20 sources in practice)
//!
//! ## Integration with GameState
//!
//! The `ManaEngine` does not directly modify `GameState`. It is a read-only
//! cache layer that:
//!
//! 1. Reads battlefield state during `update()`
//! 2. Answers queries about mana availability
//! 3. Actual mana pool modification happens in `GameState::mana_pool`
//!
//! This separation allows the engine to be used speculatively (e.g., "what if
//! I had these lands?") without affecting the game state.
//!
//! # Usage Examples
//!
//! ## Basic Usage - Check if a spell is castable
//!
//! ```ignore
//! use mtg_forge_rs::game::{ManaEngine, GameState};
//! use mtg_forge_rs::core::{ManaCost, PlayerId};
//!
//! let game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
//! let alice_id = game.players[0].id;
//!
//! // Create and update the mana engine
//! let mut engine = ManaEngine::new();
//! engine.update(&game, alice_id);
//!
//! // Check if we can cast Lightning Bolt (R)
//! let mut bolt_cost = ManaCost::new();
//! bolt_cost.red = 1;
//! let can_cast = engine.can_pay(&bolt_cost);
//! ```
//!
//! ## Integrating with AI Controllers
//!
//! ```ignore
//! // In your controller's choose_spell_ability_to_play():
//! let mut engine = ManaEngine::new();
//! engine.update(&game, player_id);
//!
//! // Filter available spells to only those we can afford
//! let affordable_spells: Vec<_> = available_spells
//!     .into_iter()
//!     .filter(|spell| {
//!         let cost = get_spell_cost(spell);
//!         engine.can_pay(&cost)
//!     })
//!     .collect();
//! ```
//!
//! ## Maintaining the Engine Across Game Actions
//!
//! For efficiency, you can maintain a `ManaEngine` instance and update it
//! only when the battlefield changes:
//!
//! ```ignore
//! impl MyController {
//!     fn on_permanent_entered(&mut self, card_id: CardId, game: &GameState) {
//!         self.mana_engine.update(game, self.player_id);  // Rebuild cache
//!     }
//!
//!     fn on_permanent_tapped(&mut self, card_id: CardId, game: &GameState) {
//!         self.mana_engine.update(game, self.player_id);  // Rebuild cache
//!     }
//! }
//! ```
//!
//! # Future Enhancements
//!
//! - **Complex source handling**: Implement search algorithm for dual lands, City of Brass, etc.
//! - **Creature mana abilities**: Recognize Llanowar Elves, Birds of Paradise
//! - **Conditional sources**: Handle lands with tap conditions (e.g., "T: Add G if you control a Forest")
//! - **Mana filtering**: Track color identity restrictions (e.g., Commander format)
//! - **Cost reduction**: Handle effects like Goblin Electromancer that reduce spell costs

use crate::core::{CardId, ManaColor, ManaCost, ManaProduction, ManaProductionKind, PlayerId};
use crate::game::mana_payment::{GreedyManaResolver, ManaPaymentResolver, ManaSource, SimpleManaResolver};
use crate::game::GameState;

/// Maximum mana production capacity
///
/// Represents the maximum amount of mana of each color that can be produced
/// by tapping all available simple mana sources.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ManaCapacity {
    /// White mana
    pub white: u8,
    /// Blue mana
    pub blue: u8,
    /// Black mana
    pub black: u8,
    /// Red mana
    pub red: u8,
    /// Green mana
    pub green: u8,
    /// Colorless mana
    pub colorless: u8,
}

impl ManaCapacity {
    /// Create a new empty mana capacity
    pub fn new() -> Self {
        Self::default()
    }

    /// Get total mana available
    pub fn total(&self) -> u8 {
        self.white
            .saturating_add(self.blue)
            .saturating_add(self.black)
            .saturating_add(self.red)
            .saturating_add(self.green)
            .saturating_add(self.colorless)
    }

    /// Check if this capacity can pay for a mana cost.
    ///
    /// For simple costs (only specific colors, no hybrid/phyrexian), this is
    /// a straightforward comparison. Returns false if the cost requires more
    /// mana of any color than we can produce.
    ///
    /// This delegates to `ManaCost::is_affordable()` which is the canonical
    /// affordability check shared with `ManaPool::can_pay()`.
    #[inline]
    pub fn can_pay_simple(&self, cost: &ManaCost) -> bool {
        cost.is_affordable(self.white, self.blue, self.black, self.red, self.green, self.colorless)
    }
}

/// Per-player mana engine
///
/// Maintains cached information about a player's mana-producing capabilities
/// and provides efficient queries for whether costs can be paid.
///
/// ## Usage
///
/// ```ignore
/// let mut engine = ManaEngine::new();
/// engine.update(&game, player_id); // Scan battlefield and cache mana sources
/// let can_cast = engine.can_pay(&mana_cost);
/// ```
///
/// ## Memoization
///
/// The engine caches its results and uses `mana_state_version` from GameState
/// to detect when a rebuild is necessary. If the version hasn't changed since
/// the last update() for the same player, the rebuild is skipped.
pub struct ManaEngine {
    /// Simple mana sources (lands producing a single color)
    simple_sources: Vec<CardId>,
    /// Complex mana sources (lands with choices or conditions)
    complex_sources: Vec<CardId>,
    /// Cached capacity from simple sources
    simple_capacity: ManaCapacity,
    /// All mana sources as ManaSource structs (for resolver)
    mana_sources: Vec<ManaSource>,
    /// Simple payment resolver (for basic lands only)
    simple_resolver: SimpleManaResolver,
    /// Greedy payment resolver (for complex mana sources)
    greedy_resolver: GreedyManaResolver,
    /// Flag indicating which resolver to use (true = use greedy, false = use simple)
    use_greedy_resolver: bool,
    /// Cached player ID for memoization
    cached_player: Option<PlayerId>,
    /// Cached mana state version for memoization
    cached_version: u32,
    /// Debug mode: verify incremental computation against from-scratch computation
    ///
    /// When enabled, every cache read will be verified against a full battlefield
    /// scan to ensure the incremental computation is correct. This is expensive
    /// and should only be used for testing.
    debug_verify_incremental: bool,
}

/// Default capacity for mana source vectors.
/// Most MTG games have 4-8 lands on battlefield, so 8 is a good starting point.
const DEFAULT_MANA_SOURCE_CAPACITY: usize = 8;

impl Default for ManaEngine {
    fn default() -> Self {
        Self {
            // Pre-allocate vectors to avoid allocation on first update().
            // Typical MTG games have 4-8 mana sources on battlefield.
            simple_sources: Vec::with_capacity(DEFAULT_MANA_SOURCE_CAPACITY),
            complex_sources: Vec::with_capacity(DEFAULT_MANA_SOURCE_CAPACITY),
            simple_capacity: ManaCapacity::new(),
            mana_sources: Vec::with_capacity(DEFAULT_MANA_SOURCE_CAPACITY),
            simple_resolver: SimpleManaResolver::new(),
            greedy_resolver: GreedyManaResolver::new(),
            use_greedy_resolver: false,
            cached_player: None,
            cached_version: u32::MAX, // Start with invalid version to force first update
            debug_verify_incremental: false,
        }
    }
}

/// Get the effective mana production for a card, including granted abilities.
///
/// This handles cards like Chromatic Lantern which grant "{T}: Add any color" to lands.
/// When a land has a granted mana ability, we merge the granted production with
/// the card's cached production using OR semantics.
///
/// Returns the merged ManaProduction if the card has granted mana abilities,
/// or None if the card's original production should be used unchanged.
fn get_effective_mana_production(
    game: &GameState,
    card_id: CardId,
    cached_production: &ManaProduction,
) -> Option<ManaProduction> {
    use crate::core::CardCache;

    // Get granted abilities from continuous effects (e.g., Chromatic Lantern)
    let granted_abilities = game.get_granted_abilities(card_id);

    // Filter to only mana abilities
    let mana_abilities: Vec<_> = granted_abilities.into_iter().filter(|a| a.is_mana_ability).collect();

    if mana_abilities.is_empty() {
        return None;
    }

    // Derive mana production from granted abilities using existing infrastructure
    let granted_production = CardCache::derive_mana_production_from_abilities(&mana_abilities);

    if !granted_production.produces_mana() {
        return None;
    }

    // Merge productions: the result should be the OR of both
    // If either produces AnyColor, result is AnyColor
    // Otherwise, combine the colors from both
    let merged_kind = merge_mana_production_kinds(&cached_production.kind, &granted_production.kind);

    Some(ManaProduction::free(merged_kind))
}

/// Merge two ManaProductionKind values using OR semantics.
///
/// The result represents what colors can be produced by either ability.
fn merge_mana_production_kinds(a: &ManaProductionKind, b: &ManaProductionKind) -> ManaProductionKind {
    use crate::game::mana_colors::ManaColors;

    // If either is AnyColor, result is AnyColor
    if matches!(a, ManaProductionKind::AnyColor) || matches!(b, ManaProductionKind::AnyColor) {
        return ManaProductionKind::AnyColor;
    }

    // Collect all colors from both productions
    let mut colors = ManaColors::new();
    let mut has_colorless = false;

    fn add_colors(kind: &ManaProductionKind, colors: &mut ManaColors, has_colorless: &mut bool) {
        match kind {
            ManaProductionKind::Fixed(c) => {
                colors.insert(*c);
            }
            ManaProductionKind::Choice(cs) => {
                for c in cs.iter() {
                    colors.insert(c);
                }
            }
            ManaProductionKind::Colorless => {
                *has_colorless = true;
            }
            ManaProductionKind::AnyColor => {
                // Handled above
            }
        }
    }

    add_colors(a, &mut colors, &mut has_colorless);
    add_colors(b, &mut colors, &mut has_colorless);

    // Build result
    let count = colors.len();
    if count == 0 {
        if has_colorless {
            ManaProductionKind::Colorless
        } else {
            ManaProductionKind::default()
        }
    } else if count == 1 && !has_colorless {
        ManaProductionKind::Fixed(colors.iter().next().unwrap())
    } else {
        // Multiple colors or colorless + color = complex choice
        ManaProductionKind::Choice(colors)
    }
}

impl ManaEngine {
    /// Create a new mana engine
    pub fn new() -> Self {
        Self::default()
    }

    /// Enable debug mode for from-scratch consistency verification
    ///
    /// When enabled, every cache read will be verified against a full battlefield
    /// scan to ensure the incremental computation matches the from-scratch result.
    ///
    /// This is expensive (defeats the purpose of caching) and should only be used
    /// in stress tests to verify correctness of the incremental computation.
    pub fn with_debug_verification(mut self) -> Self {
        self.debug_verify_incremental = true;
        self
    }

    /// Compute mana sources from scratch by scanning the battlefield
    ///
    /// This is the gold standard for correctness. It performs a full scan of the
    /// battlefield and computes all mana sources without using any cached state.
    ///
    /// Used for debug verification of the incremental cache-based computation.
    fn compute_from_scratch(
        &self,
        game: &GameState,
        player_id: PlayerId,
    ) -> (Vec<CardId>, Vec<CardId>, ManaCapacity, Vec<ManaSource>) {
        use crate::core::ManaProductionKind;

        let mut scratch_simple_sources = Vec::new();
        let mut scratch_complex_sources = Vec::new();
        let mut scratch_capacity = ManaCapacity::new();
        let mut scratch_mana_sources = Vec::new();

        // Scan battlefield for mana-producing permanents owned by this player
        for &card_id in &game.battlefield.cards {
            if let Some(card) = game.cards.try_get(card_id) {
                // Only process mana sources owned by this player
                if card.owner != player_id || !card.definition.cache.is_mana_source {
                    continue;
                }

                // Determine if this source has summoning sickness (for creatures)
                let has_summoning_sickness = if card.is_creature() {
                    if let Some(entered_turn) = card.turn_entered_battlefield {
                        entered_turn == game.turn.turn_number && !card.has_keyword(crate::core::Keyword::Haste)
                    } else {
                        false
                    }
                } else {
                    false
                };

                let cached_production = &card.definition.cache.mana_production;

                // Check for granted mana abilities (e.g., from Chromatic Lantern)
                let effective_production =
                    get_effective_mana_production(game, card_id, cached_production).unwrap_or(*cached_production);

                // Creatures with mana abilities are always complex sources
                // (due to summoning sickness and other creature-specific rules)
                if card.is_creature() {
                    scratch_complex_sources.push(card_id);
                } else {
                    // Classify source and update capacity based on effective production
                    match &effective_production.kind {
                        ManaProductionKind::Fixed(color) => {
                            use crate::core::ManaColor;
                            scratch_simple_sources.push(card_id);
                            if !card.tapped {
                                match color {
                                    ManaColor::White => scratch_capacity.white += 1,
                                    ManaColor::Blue => scratch_capacity.blue += 1,
                                    ManaColor::Black => scratch_capacity.black += 1,
                                    ManaColor::Red => scratch_capacity.red += 1,
                                    ManaColor::Green => scratch_capacity.green += 1,
                                }
                            }
                        }
                        ManaProductionKind::Colorless => {
                            scratch_simple_sources.push(card_id);
                            if !card.tapped {
                                scratch_capacity.colorless += 1;
                            }
                        }
                        ManaProductionKind::Choice(_) | ManaProductionKind::AnyColor => {
                            // Complex source - will be evaluated during payment
                            scratch_complex_sources.push(card_id);
                        }
                    }
                }

                // Add to full source list with effective production
                scratch_mana_sources.push(ManaSource {
                    card_id,
                    production: effective_production,
                    is_tapped: card.tapped,
                    has_summoning_sickness,
                });
            }
        }

        (
            scratch_simple_sources,
            scratch_complex_sources,
            scratch_capacity,
            scratch_mana_sources,
        )
    }

    /// Verify that the cached state matches the from-scratch computation
    ///
    /// Panics if there's a mismatch, with detailed diagnostic information.
    fn verify_incremental_correctness(&self, game: &GameState, player_id: PlayerId) {
        let (scratch_simple, scratch_complex, scratch_capacity, scratch_sources) =
            self.compute_from_scratch(game, player_id);

        // Check simple sources count
        if self.simple_sources.len() != scratch_simple.len() {
            // Find missing cards
            let mut missing_in_cache = Vec::new();
            let mut missing_in_scratch = Vec::new();

            for &card_id in &scratch_simple {
                if !self.simple_sources.contains(&card_id) {
                    if let Some(card) = game.cards.try_get(card_id) {
                        missing_in_cache.push(format!(
                            "{:?} ({}, tapped={}, owner={:?})",
                            card_id, card.name, card.tapped, card.owner
                        ));
                    } else {
                        missing_in_cache.push(format!("{:?} (not found)", card_id));
                    }
                }
            }

            for &card_id in &self.simple_sources {
                if !scratch_simple.contains(&card_id) {
                    if let Some(card) = game.cards.try_get(card_id) {
                        missing_in_scratch.push(format!(
                            "{:?} ({}, tapped={}, owner={:?})",
                            card_id, card.name, card.tapped, card.owner
                        ));
                    } else {
                        missing_in_scratch.push(format!("{:?} (not found)", card_id));
                    }
                }
            }

            panic!(
                "ManaEngine incremental computation error: simple_sources count mismatch\n\
                 Cached: {} sources\n\
                 From-scratch: {} sources\n\
                 Player: {:?}\n\
                 Turn: {}\n\
                 Cached sources: {:?}\n\
                 Scratch sources: {:?}\n\
                 \n\
                 Missing in cache: {:?}\n\
                 Missing in scratch: {:?}",
                self.simple_sources.len(),
                scratch_simple.len(),
                player_id,
                game.turn.turn_number,
                self.simple_sources,
                scratch_simple,
                missing_in_cache,
                missing_in_scratch
            );
        }

        // Check complex sources count
        if self.complex_sources.len() != scratch_complex.len() {
            panic!(
                "ManaEngine incremental computation error: complex_sources count mismatch\n\
                 Cached: {} sources\n\
                 From-scratch: {} sources\n\
                 Player: {:?}\n\
                 Turn: {}",
                self.complex_sources.len(),
                scratch_complex.len(),
                player_id,
                game.turn.turn_number
            );
        }

        // Check capacity (this is the most critical - it affects can_pay() queries)
        if self.simple_capacity != scratch_capacity {
            // Gather detailed diagnostics about the cards
            let mut cached_card_info = String::new();
            for &card_id in &self.simple_sources {
                if let Some(card) = game.cards.try_get(card_id) {
                    cached_card_info.push_str(&format!(
                        "\n    Card {:?} ({}): tapped={}, owner={:?}, production={:?}",
                        card_id, card.name, card.tapped, card.owner, card.definition.cache.mana_production.kind
                    ));
                }
            }

            let mut scratch_card_info = String::new();
            for &card_id in &scratch_simple {
                if let Some(card) = game.cards.try_get(card_id) {
                    scratch_card_info.push_str(&format!(
                        "\n    Card {:?} ({}): tapped={}, owner={:?}, production={:?}",
                        card_id, card.name, card.tapped, card.owner, card.definition.cache.mana_production.kind
                    ));
                }
            }

            panic!(
                "ManaEngine incremental computation error: capacity mismatch\n\
                 Cached: W={} U={} B={} R={} G={} C={} (total={})\n\
                 From-scratch: W={} U={} B={} R={} G={} C={} (total={})\n\
                 Player: {:?}\n\
                 Turn: {}\n\
                 \n\
                 Cached simple sources ({}): {}\n\
                 \n\
                 Scratch simple sources ({}): {}",
                self.simple_capacity.white,
                self.simple_capacity.blue,
                self.simple_capacity.black,
                self.simple_capacity.red,
                self.simple_capacity.green,
                self.simple_capacity.colorless,
                self.simple_capacity.total(),
                scratch_capacity.white,
                scratch_capacity.blue,
                scratch_capacity.black,
                scratch_capacity.red,
                scratch_capacity.green,
                scratch_capacity.colorless,
                scratch_capacity.total(),
                player_id,
                game.turn.turn_number,
                self.simple_sources.len(),
                cached_card_info,
                scratch_simple.len(),
                scratch_card_info
            );
        }

        // Check total mana sources count
        if self.mana_sources.len() != scratch_sources.len() {
            panic!(
                "ManaEngine incremental computation error: mana_sources count mismatch\n\
                 Cached: {} sources\n\
                 From-scratch: {} sources\n\
                 Player: {:?}\n\
                 Turn: {}",
                self.mana_sources.len(),
                scratch_sources.len(),
                player_id,
                game.turn.turn_number
            );
        }
    }

    /// Update the engine by reading from the ManaSourceCache (immutable version)
    ///
    /// This uses memoization: if the game's mana_state_version hasn't changed
    /// since the last update for this player, the cache read is skipped entirely.
    ///
    /// This version assumes the cache is already up-to-date and does not attempt
    /// to rebuild it. Use `update_mut()` if you need to ensure cache consistency.
    ///
    /// The player_id parameter specifies which player's mana sources to query.
    pub fn update(&mut self, game: &GameState, player_id: PlayerId) {
        // Memoization: skip rebuild if nothing has changed
        if self.cached_player == Some(player_id) && self.cached_version == game.mana_state_version {
            return;
        }

        // Clear previous state (but retain capacity for reuse)
        self.simple_sources.clear();
        self.complex_sources.clear();
        self.simple_capacity = ManaCapacity::new();
        self.mana_sources.clear();

        // Get the ManaSourceCache for this player (immutable version)
        let cache = match game.get_mana_cache(player_id) {
            Some(c) => c,
            None => {
                // No cache for this player - update memoization and return empty
                self.cached_player = Some(player_id);
                self.cached_version = game.mana_state_version;
                return;
            }
        };

        // Check if cache needs rebuild (e.g., after undo) or is empty (tests that bypass events)
        if cache.needs_rebuild() || cache.is_empty() {
            // Cache is stale or empty - fall back to scanning battlefield
            // This supports tests that add cards directly without triggering events
            self.scan_battlefield_fallback(game, player_id);
        } else {
            // Cache is valid - read from it
            self.read_from_cache(game, cache, player_id);
        }

        // Debug mode: verify incremental computation against from-scratch result
        if self.debug_verify_incremental {
            self.verify_incremental_correctness(game, player_id);
        }
    }

    /// Fallback method that scans battlefield directly (for tests that bypass events)
    ///
    /// This is the old implementation used before the ManaSourceCache optimization.
    /// It's kept as a fallback for tests that add cards directly to the battlefield
    /// without triggering the event handlers that populate the cache.
    fn scan_battlefield_fallback(&mut self, game: &GameState, player_id: PlayerId) {
        use crate::core::ManaProductionKind;

        // Scan battlefield for mana-producing permanents owned by this player
        for &card_id in &game.battlefield.cards {
            if let Some(card) = game.cards.try_get(card_id) {
                // Only process mana sources owned by this player
                if card.owner != player_id || !card.definition.cache.is_mana_source {
                    continue;
                }

                // Determine if this source has summoning sickness (for creatures)
                let has_summoning_sickness = if card.is_creature() {
                    if let Some(entered_turn) = card.turn_entered_battlefield {
                        entered_turn == game.turn.turn_number && !card.has_keyword(crate::core::Keyword::Haste)
                    } else {
                        false
                    }
                } else {
                    false
                };

                let cached_production = &card.definition.cache.mana_production;

                // Check for granted mana abilities (e.g., from Chromatic Lantern)
                let effective_production =
                    get_effective_mana_production(game, card_id, cached_production).unwrap_or(*cached_production);

                // Creatures with mana abilities are always complex sources
                // (due to summoning sickness and other creature-specific rules)
                if card.is_creature() {
                    self.complex_sources.push(card_id);
                } else {
                    // Classify source and update capacity based on effective production
                    match &effective_production.kind {
                        ManaProductionKind::Fixed(color) => {
                            use crate::core::ManaColor;
                            self.simple_sources.push(card_id);
                            if !card.tapped {
                                match color {
                                    ManaColor::White => self.simple_capacity.white += 1,
                                    ManaColor::Blue => self.simple_capacity.blue += 1,
                                    ManaColor::Black => self.simple_capacity.black += 1,
                                    ManaColor::Red => self.simple_capacity.red += 1,
                                    ManaColor::Green => self.simple_capacity.green += 1,
                                }
                            }
                        }
                        ManaProductionKind::Colorless => {
                            self.simple_sources.push(card_id);
                            if !card.tapped {
                                self.simple_capacity.colorless += 1;
                            }
                        }
                        ManaProductionKind::Choice(_) | ManaProductionKind::AnyColor => {
                            // Complex source - will be evaluated during payment
                            self.complex_sources.push(card_id);
                        }
                    }
                }

                // Add to full source list with effective production
                self.mana_sources.push(ManaSource {
                    card_id,
                    production: effective_production,
                    is_tapped: card.tapped,
                    has_summoning_sickness,
                });
            }
        }

        // Update memoization state
        self.cached_player = Some(player_id);
        self.cached_version = game.mana_state_version;
    }

    /// Update the engine by reading from the ManaSourceCache (mutable version)
    ///
    /// This uses memoization: if the game's mana_state_version hasn't changed
    /// since the last update for this player, the cache read is skipped entirely.
    ///
    /// This version can rebuild the cache if needed (after undo/rewind).
    ///
    /// The player_id parameter specifies which player's mana sources to query.
    ///
    /// # Panics
    ///
    /// Panics if the mana cache does not exist after rebuild (indicates internal error).
    pub fn update_mut(&mut self, game: &mut GameState, player_id: PlayerId) {
        // Memoization: skip rebuild if nothing has changed
        if self.cached_player == Some(player_id) && self.cached_version == game.mana_state_version {
            return;
        }

        // Clear previous state (but retain capacity for reuse)
        self.simple_sources.clear();
        self.complex_sources.clear();
        self.simple_capacity = ManaCapacity::new();
        self.mana_sources.clear();

        // Phase 1: Check if cache needs rebuild and rebuild if necessary
        // Also rebuild if cache is empty (handles tests/examples that bypass events)
        let cache_needs_init = game
            .get_mana_cache(player_id)
            .map(|c| {
                c.white_sources().is_empty()
                    && c.blue_sources().is_empty()
                    && c.black_sources().is_empty()
                    && c.red_sources().is_empty()
                    && c.green_sources().is_empty()
                    && c.colorless_sources().is_empty()
                    && c.complex_sources().is_empty()
                    && !game.battlefield.cards.is_empty()
            })
            .unwrap_or(false);

        if cache_needs_init {
            // Force rebuild if cache is empty but battlefield has cards
            if let Some(cache) = game
                .mana_caches
                .iter_mut()
                .find(|(id, _)| *id == player_id)
                .map(|(_, c)| c)
            {
                cache.mark_dirty();
            }
        }

        game.rebuild_mana_cache_if_needed(player_id);

        // Phase 2: Read from cache (immutable borrow)
        let cache = game.get_mana_cache(player_id).expect("Cache exists after rebuild");
        self.read_from_cache(game, cache, player_id);

        // Debug mode: verify incremental computation against from-scratch result
        if self.debug_verify_incremental {
            self.verify_incremental_correctness(game, player_id);
        }
    }

    /// Read mana sources from the cache and populate internal vectors
    ///
    /// This is the core cache-reading logic shared by both update() and update_mut().
    ///
    /// ## Optimization: Zero-Allocation Design
    ///
    /// This function pushes directly into `self.simple_sources` and `self.mana_sources`
    /// without creating intermediate temporary vectors. This eliminates ~22% of allocations
    /// (per DHAT profiling 2025-12-01) that were caused by the previous closure-based design.
    fn read_from_cache(&mut self, game: &GameState, cache: &crate::game::ManaSourceCache, player_id: PlayerId) {
        // Pre-allocate capacity based on expected source count
        let expected_sources = cache.white_sources().len()
            + cache.blue_sources().len()
            + cache.black_sources().len()
            + cache.red_sources().len()
            + cache.green_sources().len()
            + cache.colorless_sources().len()
            + cache.complex_sources().len();
        self.simple_sources.reserve(expected_sources);
        self.complex_sources.reserve(expected_sources);
        self.mana_sources.reserve(expected_sources);

        // Process simple sources by pushing directly to self vectors (no intermediate allocation)
        // This pattern is repeated for each color to avoid closure overhead and temp vecs

        // White sources
        for &card_id in cache.white_sources() {
            self.simple_sources.push(card_id);
            if let Some(card) = game.cards.try_get(card_id) {
                self.mana_sources.push(ManaSource {
                    card_id,
                    production: ManaProduction::free(ManaProductionKind::Fixed(ManaColor::White)),
                    is_tapped: card.tapped,
                    has_summoning_sickness: false, // Lands don't have summoning sickness
                });
            }
        }

        // Blue sources
        for &card_id in cache.blue_sources() {
            self.simple_sources.push(card_id);
            if let Some(card) = game.cards.try_get(card_id) {
                self.mana_sources.push(ManaSource {
                    card_id,
                    production: ManaProduction::free(ManaProductionKind::Fixed(ManaColor::Blue)),
                    is_tapped: card.tapped,
                    has_summoning_sickness: false,
                });
            }
        }

        // Black sources
        for &card_id in cache.black_sources() {
            self.simple_sources.push(card_id);
            if let Some(card) = game.cards.try_get(card_id) {
                self.mana_sources.push(ManaSource {
                    card_id,
                    production: ManaProduction::free(ManaProductionKind::Fixed(ManaColor::Black)),
                    is_tapped: card.tapped,
                    has_summoning_sickness: false,
                });
            }
        }

        // Red sources
        for &card_id in cache.red_sources() {
            self.simple_sources.push(card_id);
            if let Some(card) = game.cards.try_get(card_id) {
                self.mana_sources.push(ManaSource {
                    card_id,
                    production: ManaProduction::free(ManaProductionKind::Fixed(ManaColor::Red)),
                    is_tapped: card.tapped,
                    has_summoning_sickness: false,
                });
            }
        }

        // Green sources
        for &card_id in cache.green_sources() {
            self.simple_sources.push(card_id);
            if let Some(card) = game.cards.try_get(card_id) {
                self.mana_sources.push(ManaSource {
                    card_id,
                    production: ManaProduction::free(ManaProductionKind::Fixed(ManaColor::Green)),
                    is_tapped: card.tapped,
                    has_summoning_sickness: false,
                });
            }
        }

        // Colorless sources
        for &card_id in cache.colorless_sources() {
            self.simple_sources.push(card_id);
            if let Some(card) = game.cards.try_get(card_id) {
                self.mana_sources.push(ManaSource {
                    card_id,
                    production: ManaProduction::free(ManaProductionKind::Colorless),
                    is_tapped: card.tapped,
                    has_summoning_sickness: false,
                });
            }
        }

        // Read pre-computed untapped counts from cache
        self.simple_capacity.white = cache.untapped_white() as u8;
        self.simple_capacity.blue = cache.untapped_blue() as u8;
        self.simple_capacity.black = cache.untapped_black() as u8;
        self.simple_capacity.red = cache.untapped_red() as u8;
        self.simple_capacity.green = cache.untapped_green() as u8;
        self.simple_capacity.colorless = cache.untapped_colorless() as u8;

        // Read complex sources from cache
        let current_turn = game.turn.turn_number;
        for &card_id in cache.complex_sources() {
            self.complex_sources.push(card_id);
            if let Some(card) = game.cards.try_get(card_id) {
                // Determine if this source has summoning sickness (for creatures with mana abilities)
                let has_summoning_sickness = if card.is_creature() {
                    if let Some(entered_turn) = card.turn_entered_battlefield {
                        entered_turn == current_turn && !card.has_keyword(crate::core::Keyword::Haste)
                    } else {
                        false
                    }
                } else {
                    false
                };

                // Get production from card's cached mana_production
                let production = card.definition.cache.mana_production;
                self.mana_sources.push(ManaSource {
                    card_id,
                    production,
                    is_tapped: card.tapped,
                    has_summoning_sickness,
                });
            }
        }

        // Switch resolver flag based on whether we have complex sources
        self.use_greedy_resolver = !self.complex_sources.is_empty();

        // Update memoization cache
        self.cached_player = Some(player_id);
        self.cached_version = game.mana_state_version;
    }

    /// Invalidate the memoization cache
    ///
    /// Call this when the mana state may have changed but the version wasn't
    /// incremented (e.g., tap events that bypass the normal tracking).
    #[inline]
    pub fn invalidate_cache(&mut self) {
        self.cached_version = u32::MAX;
    }

    /// Check if the player can pay for a mana cost
    ///
    /// This considers all mana sources (simple and complex) and determines
    /// whether there exists a way to tap them to produce the required mana.
    ///
    /// NOTE: This method does NOT consider mana already in the player's mana pool.
    /// Use `can_pay_with_pool()` to include floating mana from rituals like Dark Ritual.
    pub fn can_pay(&self, cost: &ManaCost) -> bool {
        // Use the appropriate resolver based on source complexity
        if self.use_greedy_resolver {
            self.greedy_resolver.can_pay(cost, &self.mana_sources)
        } else {
            self.simple_resolver.can_pay(cost, &self.mana_sources)
        }
    }

    /// Check if the player can pay for a mana cost, considering both the mana pool and mana sources
    ///
    /// This method considers:
    /// 1. Mana already in the player's mana pool (floating mana from rituals like Dark Ritual)
    /// 2. Mana that can be produced by tapping available mana sources
    ///
    /// The algorithm:
    /// 1. First use mana from the pool to satisfy colored requirements
    /// 2. Then check if remaining requirements can be satisfied by tapping sources
    pub fn can_pay_with_pool(&self, cost: &ManaCost, pool: &crate::core::ManaPool) -> bool {
        // If the pool alone can pay, return true immediately
        if pool.can_pay(cost) {
            return true;
        }

        // Calculate remaining cost after using pool mana
        // First satisfy colored requirements from pool
        let remaining_white = cost.white.saturating_sub(pool.white);
        let remaining_blue = cost.blue.saturating_sub(pool.blue);
        let remaining_black = cost.black.saturating_sub(pool.black);
        let remaining_red = cost.red.saturating_sub(pool.red);
        let remaining_green = cost.green.saturating_sub(pool.green);
        let remaining_colorless = cost.colorless.saturating_sub(pool.colorless);

        // Calculate how much pool mana was used for colored requirements
        let used_white = cost.white.min(pool.white);
        let used_blue = cost.blue.min(pool.blue);
        let used_black = cost.black.min(pool.black);
        let used_red = cost.red.min(pool.red);
        let used_green = cost.green.min(pool.green);
        let used_colorless = cost.colorless.min(pool.colorless);

        // Calculate remaining pool mana that can be used for generic costs
        let pool_for_generic = (pool.white.saturating_sub(used_white))
            + (pool.blue.saturating_sub(used_blue))
            + (pool.black.saturating_sub(used_black))
            + (pool.red.saturating_sub(used_red))
            + (pool.green.saturating_sub(used_green))
            + (pool.colorless.saturating_sub(used_colorless));

        // Calculate remaining generic cost after using pool's surplus
        let remaining_generic = cost.generic.saturating_sub(pool_for_generic);

        // Create a new cost with the remaining requirements
        let remaining_cost = ManaCost {
            generic: remaining_generic,
            white: remaining_white,
            blue: remaining_blue,
            black: remaining_black,
            red: remaining_red,
            green: remaining_green,
            colorless: remaining_colorless,
            x_count: 0, // X costs are handled separately
        };

        // If there's nothing remaining to pay, we're done
        if remaining_cost.cmc() == 0 {
            return true;
        }

        // Check if mana sources can pay the remaining cost
        if self.use_greedy_resolver {
            self.greedy_resolver.can_pay(&remaining_cost, &self.mana_sources)
        } else {
            self.simple_resolver.can_pay(&remaining_cost, &self.mana_sources)
        }
    }

    /// Get the current mana capacity from simple sources only
    pub fn simple_capacity(&self) -> ManaCapacity {
        self.simple_capacity
    }

    /// Get the maximum mana capacity considering all sources (simple and complex)
    ///
    /// This computes the maximum of each color that could be produced if all
    /// untapped sources were tapped optimistically:
    /// - Fixed(R) sources add 1 to R
    /// - Choice([R, B]) sources add 1 to both R and B
    /// - AnyColor sources add 1 to all colors
    ///
    /// Note: The total returned is the count of untapped sources, not the sum
    /// of all colors (since dual lands contribute to multiple colors but only
    /// count as 1 source).
    pub fn max_mana_capacity(&self) -> ManaCapacity {
        let mut capacity = ManaCapacity::new();

        for source in &self.mana_sources {
            // Skip tapped sources and sources with summoning sickness
            if source.is_tapped || source.has_summoning_sickness {
                continue;
            }

            match &source.production.kind {
                ManaProductionKind::Fixed(color) => match color {
                    ManaColor::White => capacity.white += 1,
                    ManaColor::Blue => capacity.blue += 1,
                    ManaColor::Black => capacity.black += 1,
                    ManaColor::Red => capacity.red += 1,
                    ManaColor::Green => capacity.green += 1,
                },
                ManaProductionKind::Choice(colors) => {
                    // Dual lands: add 1 to each color in the choice
                    for color in colors.iter() {
                        match color {
                            ManaColor::White => capacity.white += 1,
                            ManaColor::Blue => capacity.blue += 1,
                            ManaColor::Black => capacity.black += 1,
                            ManaColor::Red => capacity.red += 1,
                            ManaColor::Green => capacity.green += 1,
                        }
                    }
                }
                ManaProductionKind::AnyColor => {
                    // Any-color lands: add 1 to all colors
                    capacity.white += 1;
                    capacity.blue += 1;
                    capacity.black += 1;
                    capacity.red += 1;
                    capacity.green += 1;
                }
                ManaProductionKind::Colorless => {
                    capacity.colorless += 1;
                }
            }
        }

        capacity
    }

    /// Get the list of simple mana sources
    pub fn simple_sources(&self) -> &[CardId] {
        &self.simple_sources
    }

    /// Get the list of complex mana sources
    pub fn complex_sources(&self) -> &[CardId] {
        &self.complex_sources
    }

    /// Get all mana sources with their production information
    ///
    /// This returns the complete list of ManaSource structs that were identified
    /// during the last update(). This is useful for getting a tap order from the
    /// payment resolver without rebuilding the source list.
    pub fn all_sources(&self) -> &[ManaSource] {
        &self.mana_sources
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{Card, CardType};

    #[test]
    fn test_mana_capacity_total() {
        let capacity = ManaCapacity {
            white: 2,
            blue: 1,
            black: 0,
            red: 3,
            green: 1,
            colorless: 0,
        };
        assert_eq!(capacity.total(), 7);
    }

    #[test]
    fn test_can_pay_simple_exact() {
        let capacity = ManaCapacity {
            white: 2,
            blue: 1,
            black: 1,
            red: 2,
            green: 1,
            colorless: 0,
        };

        // Exact match
        let cost = ManaCost {
            generic: 0,
            white: 2,
            blue: 1,
            black: 1,
            red: 2,
            green: 1,
            colorless: 0,
            x_count: 0,
        };
        assert!(capacity.can_pay_simple(&cost));
    }

    #[test]
    fn test_can_pay_simple_insufficient_color() {
        let capacity = ManaCapacity {
            white: 1,
            blue: 1,
            black: 1,
            red: 1,
            green: 1,
            colorless: 0,
        };

        // Requires 2 red, but we only have 1
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
        assert!(!capacity.can_pay_simple(&cost));
    }

    #[test]
    fn test_can_pay_simple_with_generic() {
        let capacity = ManaCapacity {
            white: 1,
            blue: 1,
            black: 1,
            red: 2,
            green: 1,
            colorless: 0,
        };

        // Cost: 1R (1 generic + 1 red)
        // We have 2 red, so 1 can be used for the red requirement
        // and we have 5 other mana for the generic
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
        assert!(capacity.can_pay_simple(&cost));
    }

    #[test]
    fn test_mana_engine_update_simple_sources() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Add some basic lands to the battlefield
        let mountain_id = game.next_card_id();
        let mut mountain = Card::new(mountain_id, "Mountain".to_string(), p1_id);
        mountain.add_type(CardType::Land);
        mountain.controller = p1_id;
        game.cards.insert(mountain_id, mountain);
        game.battlefield.add(mountain_id);

        let island_id = game.next_card_id();
        let mut island = Card::new(island_id, "Island".to_string(), p1_id);
        island.add_type(CardType::Land);
        island.controller = p1_id;
        game.cards.insert(island_id, island);
        game.battlefield.add(island_id);

        // Create engine and update
        let mut engine = ManaEngine::new();
        engine.update_mut(&mut game, p1_id);

        // Should have detected 2 simple sources
        assert_eq!(engine.simple_sources().len(), 2);
        assert_eq!(engine.complex_sources().len(), 0);

        // Should have correct capacity
        assert_eq!(engine.simple_capacity().red, 1);
        assert_eq!(engine.simple_capacity().blue, 1);
        assert_eq!(engine.simple_capacity().total(), 2);
    }

    #[test]
    fn test_mana_engine_can_pay() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Add 3 mountains
        for _ in 0..3 {
            let land_id = game.next_card_id();
            let mut land = Card::new(land_id, "Mountain".to_string(), p1_id);
            land.add_type(CardType::Land);
            land.controller = p1_id;
            game.cards.insert(land_id, land);
            game.battlefield.add(land_id);
        }

        let mut engine = ManaEngine::new();
        engine.update_mut(&mut game, p1_id);

        // Should be able to pay for 2R (Lightning Bolt)
        let bolt_cost = ManaCost::from_string("2R");
        assert!(engine.can_pay(&bolt_cost));

        // Should not be able to pay for 4R
        let expensive_cost = ManaCost::from_string("4R");
        assert!(!engine.can_pay(&expensive_cost));

        // Should not be able to pay for 1U (requires blue)
        let blue_cost = ManaCost::from_string("1U");
        assert!(!engine.can_pay(&blue_cost));
    }

    #[test]
    fn test_mana_engine_with_llanowar_elves() {
        use crate::core::{ActivatedAbility, Cost, Effect};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Add a Forest and Llanowar Elves
        let forest_id = game.next_card_id();
        let mut forest = Card::new(forest_id, "Forest".to_string(), p1_id);
        forest.add_type(CardType::Land);
        forest.controller = p1_id;
        game.cards.insert(forest_id, forest);
        game.battlefield.add(forest_id);

        let elf_id = game.next_card_id();
        let mut elf = Card::new(elf_id, "Llanowar Elves".to_string(), p1_id);
        elf.add_type(CardType::Creature);
        elf.controller = p1_id;
        elf.set_text("{T}: Add {G}.".to_string());
        // Add explicit mana ability (mana production is derived from abilities, not text)
        elf.activated_abilities.push(ActivatedAbility::new(
            Cost::Tap,
            vec![Effect::AddMana {
                player: p1_id,
                mana: ManaCost::from_string("G"),
                produces_chosen_color: false,
                amount_var: None,
            }],
            "Add {G}".to_string(),
            true, // is_mana_ability
        ));
        elf.definition.cache.update_from_abilities(&elf.activated_abilities);
        elf.turn_entered_battlefield = Some(game.turn.turn_number - 1); // Not summoning sick
        game.cards.insert(elf_id, elf);
        game.battlefield.add(elf_id);

        let mut engine = ManaEngine::new();
        engine.update_mut(&mut game, p1_id);

        // Should have 1 simple source (Forest) and 1 complex source (Llanowar Elves)
        assert_eq!(engine.simple_sources().len(), 1);
        assert_eq!(engine.complex_sources().len(), 1);

        // Should be able to pay for GG (2 green mana)
        let gg_cost = ManaCost::from_string("GG");
        assert!(engine.can_pay(&gg_cost));

        // Should be able to pay for 1G (1 generic + 1 green)
        let one_g_cost = ManaCost::from_string("1G");
        assert!(engine.can_pay(&one_g_cost));
    }

    #[test]
    fn test_mana_engine_summoning_sickness() {
        use crate::core::{ActivatedAbility, Cost, Effect};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Add a Forest
        let forest_id = game.next_card_id();
        let mut forest = Card::new(forest_id, "Forest".to_string(), p1_id);
        forest.add_type(CardType::Land);
        forest.controller = p1_id;
        game.cards.insert(forest_id, forest);
        game.battlefield.add(forest_id);

        // Add Llanowar Elves with summoning sickness (entered this turn)
        let elf_id = game.next_card_id();
        let mut elf = Card::new(elf_id, "Llanowar Elves".to_string(), p1_id);
        elf.add_type(CardType::Creature);
        elf.controller = p1_id;
        elf.set_text("{T}: Add {G}.".to_string());
        // Add explicit mana ability (mana production is derived from abilities, not text)
        elf.activated_abilities.push(ActivatedAbility::new(
            Cost::Tap,
            vec![Effect::AddMana {
                player: p1_id,
                mana: ManaCost::from_string("G"),
                produces_chosen_color: false,
                amount_var: None,
            }],
            "Add {G}".to_string(),
            true, // is_mana_ability
        ));
        elf.definition.cache.update_from_abilities(&elf.activated_abilities);
        elf.turn_entered_battlefield = Some(game.turn.turn_number); // Summoning sick!
        game.cards.insert(elf_id, elf);
        game.battlefield.add(elf_id);

        let mut engine = ManaEngine::new();
        engine.update_mut(&mut game, p1_id);

        // Should detect the creature as complex source
        assert_eq!(engine.complex_sources().len(), 1);

        // The mana source should have summoning_sickness flag set
        let creature_source = engine
            .mana_sources
            .iter()
            .find(|s| s.card_id == elf_id)
            .expect("Should find Llanowar Elves");
        assert!(creature_source.has_summoning_sickness);

        // Should only be able to pay for G (from Forest), not GG
        let g_cost = ManaCost::from_string("G");
        assert!(engine.can_pay(&g_cost));

        let gg_cost = ManaCost::from_string("GG");
        assert!(!engine.can_pay(&gg_cost)); // Can't use summoning-sick creature
    }

    /// Test that non-basic lands with {T}: Add {C} are correctly identified as mana sources
    #[test]
    fn test_mishras_factory_colorless_mana() {
        use crate::core::{ActivatedAbility, Cost, Effect};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Create Mishra's Factory - a land that produces colorless mana
        let factory_id = game.next_card_id();
        let mut factory = Card::new(factory_id, "Mishra's Factory".to_string(), p1_id);
        factory.add_type(CardType::Land);
        factory.controller = p1_id;
        factory.set_text("{T}: Add {C}.\n{1}: Mishra's Factory becomes a 2/2 Assembly-Worker artifact creature until end of turn. It's still a land.".to_string());
        // Add explicit mana ability (mana production is derived from abilities, not text)
        factory.activated_abilities.push(ActivatedAbility::new(
            Cost::Tap,
            vec![Effect::AddMana {
                player: p1_id,
                mana: ManaCost::from_string("C"),
                produces_chosen_color: false,
                amount_var: None,
            }],
            "Add {C}".to_string(),
            true, // is_mana_ability
        ));
        factory
            .definition
            .cache
            .update_from_abilities(&factory.activated_abilities);

        // Verify the cache detects colorless mana production
        assert!(
            factory.definition.cache.mana_production.produces_mana(),
            "Mishra's Factory cache should detect mana production"
        );
        assert_eq!(
            factory.definition.cache.mana_production.kind,
            ManaProductionKind::Colorless,
            "Mishra's Factory should produce Colorless mana"
        );

        // Add to battlefield
        game.cards.insert(factory_id, factory);
        game.battlefield.add(factory_id);

        // Test that ManaEngine finds it
        let mut engine = ManaEngine::new();
        engine.update(&game, p1_id);

        // Should have 1 source
        assert_eq!(
            engine.all_sources().len(),
            1,
            "Should find Mishra's Factory as a mana source"
        );

        // Check the source produces colorless
        let source = &engine.all_sources()[0];
        assert_eq!(source.card_id, factory_id);
        assert_eq!(
            source.production.kind,
            ManaProductionKind::Colorless,
            "Mishra's Factory should be tracked as producing Colorless"
        );

        // Verify max_mana_capacity includes colorless
        let capacity = engine.max_mana_capacity();
        assert_eq!(capacity.colorless, 1, "Should have 1 colorless mana available");
    }

    /// Test can_pay_with_pool - pool alone can pay
    #[test]
    fn test_can_pay_with_pool_pool_alone() {
        let game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Create engine with no mana sources
        let mut engine = ManaEngine::new();
        engine.update(&game, p1_id);

        // Pool with BBB (like after Dark Ritual)
        let pool = crate::core::ManaPool {
            white: 0,
            blue: 0,
            black: 3,
            red: 0,
            green: 0,
            colorless: 0,
        };

        // Should be able to pay BB (Black Knight cost)
        let bb_cost = ManaCost::from_string("BB");
        assert!(engine.can_pay_with_pool(&bb_cost, &pool));

        // Should be able to pay 1BB (Hypnotic Specter cost)
        let cost_1bb = ManaCost::from_string("1BB");
        assert!(engine.can_pay_with_pool(&cost_1bb, &pool));

        // Should NOT be able to pay 2BB (Juzam Djinn cost) - only 3 mana in pool
        let cost_2bb = ManaCost::from_string("2BB");
        assert!(!engine.can_pay_with_pool(&cost_2bb, &pool));
    }

    /// Test can_pay_with_pool - pool + sources combined
    #[test]
    fn test_can_pay_with_pool_combined() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Add one Swamp to battlefield
        let swamp_id = game.next_card_id();
        let mut swamp = Card::new(swamp_id, "Swamp".to_string(), p1_id);
        swamp.types.push(CardType::Land);
        swamp.controller = p1_id;
        game.cards.insert(swamp_id, swamp);
        game.battlefield.add(swamp_id);

        let mut engine = ManaEngine::new();
        engine.update_mut(&mut game, p1_id);

        // Pool with BBB (like after Dark Ritual, land already tapped)
        let pool = crate::core::ManaPool {
            white: 0,
            blue: 0,
            black: 3,
            red: 0,
            green: 0,
            colorless: 0,
        };

        // Should be able to pay 2BB - 3B from pool + 1B from tapping Swamp = 4 total
        let cost_2bb = ManaCost::from_string("2BB");
        assert!(engine.can_pay_with_pool(&cost_2bb, &pool));

        // Should be able to pay 3BB - exactly 4 mana available (3 pool + 1 source)
        let cost_3bb = ManaCost::from_string("3BB");
        assert!(!engine.can_pay_with_pool(&cost_3bb, &pool)); // Need 5, have 4

        // Pool has 3 black, sources have 1 black = 4 black total, but generic needs more
    }

    /// Test can_pay_with_pool - mixed colors
    #[test]
    fn test_can_pay_with_pool_mixed_colors() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Add one Mountain to battlefield
        let mountain_id = game.next_card_id();
        let mut mountain = Card::new(mountain_id, "Mountain".to_string(), p1_id);
        mountain.types.push(CardType::Land);
        mountain.controller = p1_id;
        game.cards.insert(mountain_id, mountain);
        game.battlefield.add(mountain_id);

        let mut engine = ManaEngine::new();
        engine.update_mut(&mut game, p1_id);

        // Pool with BBB
        let pool = crate::core::ManaPool {
            white: 0,
            blue: 0,
            black: 3,
            red: 0,
            green: 0,
            colorless: 0,
        };

        // Can pay 2BR - 2 generic + 1 black from pool, 1 red from Mountain
        let cost_2br = ManaCost::from_string("2BR");
        assert!(engine.can_pay_with_pool(&cost_2br, &pool));

        // Can't pay RR - need 2 red, only have 1 Mountain
        let cost_rr = ManaCost::from_string("RR");
        assert!(!engine.can_pay_with_pool(&cost_rr, &pool));
    }

    /// Test can_pay_with_pool - empty pool falls back to sources only
    #[test]
    fn test_can_pay_with_pool_empty_pool() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Add two Swamps
        for _ in 0..2 {
            let swamp_id = game.next_card_id();
            let mut swamp = Card::new(swamp_id, "Swamp".to_string(), p1_id);
            swamp.types.push(CardType::Land);
            swamp.controller = p1_id;
            game.cards.insert(swamp_id, swamp);
            game.battlefield.add(swamp_id);
        }

        let mut engine = ManaEngine::new();
        engine.update_mut(&mut game, p1_id);

        // Empty pool
        let pool = crate::core::ManaPool::new();

        // Can pay BB with just sources
        let bb_cost = ManaCost::from_string("BB");
        assert!(engine.can_pay_with_pool(&bb_cost, &pool));

        // Equivalent to can_pay without pool
        assert!(engine.can_pay(&bb_cost));
    }

    /// Regression test: 3 sources cannot pay for 3G cost (needs 4 mana)
    ///
    /// This tests the exact scenario from the AI bug where can_pay_with_pool
    /// was incorrectly returning true for costs that couldn't be paid.
    #[test]
    fn test_can_pay_with_pool_insufficient_for_generic_plus_color() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Add 3 Forests (green mana sources)
        for i in 0..3 {
            let forest_id = game.next_card_id();
            let mut forest = Card::new(forest_id, format!("Forest{}", i), p1_id);
            forest.types.push(CardType::Land);
            forest.controller = p1_id;
            forest.definition.cache.is_mana_source = true;
            forest.definition.cache.mana_production = crate::core::ManaProduction::free(
                crate::core::ManaProductionKind::Fixed(crate::core::ManaColor::Green),
            );
            game.cards.insert(forest_id, forest);
            game.battlefield.add(forest_id);
        }

        let mut engine = ManaEngine::new();
        engine.update_mut(&mut game, p1_id);

        // Empty pool (no floating mana)
        let pool = crate::core::ManaPool::new();

        // Cost: 3G = 3 generic + 1 green = 4 total mana
        // With only 3 sources, this MUST NOT be payable
        let cost_3g = ManaCost::from_string("3G");
        assert_eq!(cost_3g.generic, 3, "3G should have generic=3");
        assert_eq!(cost_3g.green, 1, "3G should have green=1");
        assert_eq!(cost_3g.cmc(), 4, "3G should have cmc=4");

        // Verify engine has exactly 3 sources
        assert_eq!(
            engine.all_sources().len(),
            3,
            "Engine should see exactly 3 mana sources"
        );

        // This MUST return false - only 3 mana available for a 4-mana cost
        assert!(
            !engine.can_pay_with_pool(&cost_3g, &pool),
            "3 green sources + empty pool should NOT be able to pay 3G (needs 4, has 3)"
        );

        // Also verify can_pay (without pool consideration)
        assert!(
            !engine.can_pay(&cost_3g),
            "3 green sources should NOT be able to pay 3G"
        );

        // But 2G SHOULD be payable (3 mana for 3-mana cost)
        let cost_2g = ManaCost::from_string("2G");
        assert_eq!(cost_2g.cmc(), 3, "2G should have cmc=3");
        assert!(
            engine.can_pay_with_pool(&cost_2g, &pool),
            "3 green sources should be able to pay 2G (needs 3, has 3)"
        );

        // And 3 colorless SHOULD be payable
        let cost_3 = ManaCost::from_string("3");
        assert_eq!(cost_3.cmc(), 3, "3 should have cmc=3");
        assert!(
            engine.can_pay_with_pool(&cost_3, &pool),
            "3 green sources should be able to pay 3 generic (needs 3, has 3)"
        );
    }
}
