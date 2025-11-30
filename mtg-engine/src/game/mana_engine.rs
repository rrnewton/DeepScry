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

    /// Check if this capacity can pay for a mana cost
    ///
    /// For simple costs (only specific colors, no hybrid/phyrexian), this is
    /// a straightforward comparison. Returns false if the cost requires more
    /// mana of any color than we can produce.
    pub fn can_pay_simple(&self, cost: &ManaCost) -> bool {
        // Check specific color requirements
        if cost.white > self.white {
            return false;
        }
        if cost.blue > self.blue {
            return false;
        }
        if cost.black > self.black {
            return false;
        }
        if cost.red > self.red {
            return false;
        }
        if cost.green > self.green {
            return false;
        }
        if cost.colorless > self.colorless {
            return false;
        }

        // Check if we have enough total mana for generic requirement
        // Generic can be paid with any color or colorless mana
        let remaining_capacity = self
            .total()
            .saturating_sub(cost.white)
            .saturating_sub(cost.blue)
            .saturating_sub(cost.black)
            .saturating_sub(cost.red)
            .saturating_sub(cost.green)
            .saturating_sub(cost.colorless);

        remaining_capacity >= cost.generic
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
                if card.owner != player_id || !card.cache.is_mana_source {
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

                let production = &card.cache.mana_production;

                // Classify source and update capacity
                match &production.kind {
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

                // Add to full source list
                scratch_mana_sources.push(ManaSource {
                    card_id,
                    production: *production,
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
            panic!(
                "ManaEngine incremental computation error: simple_sources count mismatch\n\
                 Cached: {} sources\n\
                 From-scratch: {} sources\n\
                 Player: {:?}\n\
                 Turn: {}\n\
                 Cached sources: {:?}\n\
                 Scratch sources: {:?}",
                self.simple_sources.len(),
                scratch_simple.len(),
                player_id,
                game.turn.turn_number,
                self.simple_sources,
                scratch_simple
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
            panic!(
                "ManaEngine incremental computation error: capacity mismatch\n\
                 Cached: W={} U={} B={} R={} G={} C={} (total={})\n\
                 From-scratch: W={} U={} B={} R={} G={} C={} (total={})\n\
                 Player: {:?}\n\
                 Turn: {}",
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
                game.turn.turn_number
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
                if card.owner != player_id || !card.cache.is_mana_source {
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

                let production = &card.cache.mana_production;

                // Classify source and update capacity
                match &production.kind {
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

                // Add to full source list
                self.mana_sources.push(ManaSource {
                    card_id,
                    production: *production,
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

        // Helper closure to process simple sources
        let process_simple = |color: ManaColor, sources: &[CardId]| -> (Vec<CardId>, Vec<ManaSource>) {
            let mut ids = Vec::new();
            let mut sources_vec = Vec::new();
            for &card_id in sources {
                ids.push(card_id);
                if let Some(card) = game.cards.try_get(card_id) {
                    sources_vec.push(ManaSource {
                        card_id,
                        production: ManaProduction::free(ManaProductionKind::Fixed(color)),
                        is_tapped: card.tapped,
                        has_summoning_sickness: false, // Lands don't have summoning sickness
                    });
                }
            }
            (ids, sources_vec)
        };

        // Read simple sources from cache - White
        let (white_ids, white_sources) = process_simple(ManaColor::White, cache.white_sources());
        self.simple_sources.extend(white_ids);
        self.mana_sources.extend(white_sources);

        // Read simple sources from cache - Blue
        let (blue_ids, blue_sources) = process_simple(ManaColor::Blue, cache.blue_sources());
        self.simple_sources.extend(blue_ids);
        self.mana_sources.extend(blue_sources);

        // Read simple sources from cache - Black
        let (black_ids, black_sources) = process_simple(ManaColor::Black, cache.black_sources());
        self.simple_sources.extend(black_ids);
        self.mana_sources.extend(black_sources);

        // Read simple sources from cache - Red
        let (red_ids, red_sources) = process_simple(ManaColor::Red, cache.red_sources());
        self.simple_sources.extend(red_ids);
        self.mana_sources.extend(red_sources);

        // Read simple sources from cache - Green
        let (green_ids, green_sources) = process_simple(ManaColor::Green, cache.green_sources());
        self.simple_sources.extend(green_ids);
        self.mana_sources.extend(green_sources);

        // Read simple sources from cache - Colorless
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
                let production = card.cache.mana_production;
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
    pub fn can_pay(&self, cost: &ManaCost) -> bool {
        // Use the appropriate resolver based on source complexity
        if self.use_greedy_resolver {
            self.greedy_resolver.can_pay(cost, &self.mana_sources)
        } else {
            self.simple_resolver.can_pay(cost, &self.mana_sources)
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
        mountain.types.push(CardType::Land);
        mountain.controller = p1_id;
        game.cards.insert(mountain_id, mountain);
        game.battlefield.add(mountain_id);

        let island_id = game.next_card_id();
        let mut island = Card::new(island_id, "Island".to_string(), p1_id);
        island.types.push(CardType::Land);
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
            land.types.push(CardType::Land);
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
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Add a Forest and Llanowar Elves
        let forest_id = game.next_card_id();
        let mut forest = Card::new(forest_id, "Forest".to_string(), p1_id);
        forest.types.push(CardType::Land);
        forest.controller = p1_id;
        game.cards.insert(forest_id, forest);
        game.battlefield.add(forest_id);

        let elf_id = game.next_card_id();
        let mut elf = Card::new(elf_id, "Llanowar Elves".to_string(), p1_id);
        elf.types.push(CardType::Creature);
        elf.controller = p1_id;
        elf.set_text("{T}: Add {G}.".to_string());
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
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Add a Forest
        let forest_id = game.next_card_id();
        let mut forest = Card::new(forest_id, "Forest".to_string(), p1_id);
        forest.types.push(CardType::Land);
        forest.controller = p1_id;
        game.cards.insert(forest_id, forest);
        game.battlefield.add(forest_id);

        // Add Llanowar Elves with summoning sickness (entered this turn)
        let elf_id = game.next_card_id();
        let mut elf = Card::new(elf_id, "Llanowar Elves".to_string(), p1_id);
        elf.types.push(CardType::Creature);
        elf.controller = p1_id;
        elf.set_text("{T}: Add {G}.".to_string());
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
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Create Mishra's Factory - a land that produces colorless mana
        let factory_id = game.next_card_id();
        let mut factory = Card::new(factory_id, "Mishra's Factory".to_string(), p1_id);
        factory.types.push(CardType::Land);
        factory.controller = p1_id;
        // This is the oracle text that should trigger colorless mana detection
        factory.set_text("{T}: Add {C}.\n{1}: Mishra's Factory becomes a 2/2 Assembly-Worker artifact creature until end of turn. It's still a land.".to_string());

        // Verify the cache detects colorless mana production
        assert!(
            factory.cache.mana_production.produces_mana(),
            "Mishra's Factory cache should detect mana production"
        );
        assert_eq!(
            factory.cache.mana_production.kind,
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
}
