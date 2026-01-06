//! Main game state structure

use crate::core::{
    Card, CardId, Color, DelayedTriggerStore, EntityId, EntityStore, PersistentEffectStore, Player, PlayerId,
};
use crate::game::{CombatState, GameLogger, ManaSourceCache, TurnStructure};
use crate::undo::UndoLog;
use crate::zones::{CardZone, PlayerZones, Zone};
use crate::Result;
use bumpalo::Bump;
use rand::SeedableRng;
use rand_chacha::ChaCha12Rng;
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use std::cell::RefCell;

/// Complete game state
///
/// This is the central structure that holds all game information.
/// It's designed to be efficiently clonable for tree search.
///
/// Note: Clone is manually implemented because Bump doesn't implement Clone.
/// Each clone gets a fresh empty Bump allocator.
#[derive(Debug, Serialize, Deserialize)]
pub struct GameState {
    /// All cards in the game
    pub cards: EntityStore<Card>,

    /// All players in the game (Vec for stable ordering, small count)
    pub players: Vec<Player>,

    /// Zones for each player
    pub player_zones: Vec<(PlayerId, PlayerZones)>,

    /// Mana source caches for incremental mana tracking (one per player)
    /// Not serialized - rebuilt from battlefield after load
    #[serde(skip)]
    pub mana_caches: Vec<(PlayerId, ManaSourceCache)>,

    /// Shared battlefield (all players)
    pub battlefield: CardZone,

    /// The stack (for spells and abilities)
    pub stack: CardZone,

    /// Turn structure
    pub turn: TurnStructure,

    /// Combat state (active during combat phase)
    pub combat: CombatState,

    /// Random number generator for gameplay (serializable for deterministic replay)
    /// This RNG is used by controllers and game logic for random decisions.
    /// Unlike the initial seed, this captures the CURRENT RNG state.
    ///
    /// Wrapped in RefCell to allow interior mutability - this lets us get mutable
    /// access to the RNG even when GameState is borrowed immutably (e.g., for GameStateView).
    pub rng: RefCell<ChaCha12Rng>,

    /// Unified entity ID generator (shared across all entity types)
    next_entity_id: u32,

    /// Undo log for tracking all game actions
    pub undo_log: UndoLog,

    /// Centralized logger for game events
    pub logger: GameLogger,

    /// Token definitions cache (loaded at game initialization)
    /// Maps token script name (e.g., "c_a_food_sac") to card definition
    /// Not serialized - will be empty when loading from snapshot
    /// For native builds, loaded from tokenscripts/ directory.
    /// For WASM builds, loaded from bundled deck token data.
    #[serde(skip)]
    pub token_definitions: std::collections::HashMap<String, std::sync::Arc<crate::loader::CardDefinition>>,

    /// Per-game bump allocator for temporary allocations
    ///
    /// This arena is used for short-lived allocations during gameplay (e.g.,
    /// temporary Vecs for creature queries). Using a per-game allocator
    /// eliminates allocator contention in parallel simulations.
    ///
    /// Not serialized - recreated fresh when loading from snapshot.
    #[serde(skip)]
    pub bump: Bump,

    /// Mana state version counter for ManaEngine memoization
    ///
    /// Incremented when cards enter/leave battlefield or tap/untap.
    /// ManaEngine can compare against its cached version to skip
    /// redundant battlefield scans when nothing has changed.
    ///
    /// Separate versions per player would be more precise but this
    /// global version is simpler and still effective - if nothing
    /// changed for either player, the version stays the same.
    pub mana_state_version: u32,

    /// Persistent effects that last beyond a single spell/ability resolution.
    ///
    /// # Design Note: NOT the Command Zone
    ///
    /// Unlike Java Forge, which stores persistent effects as "virtual cards"
    /// in the command zone, we use dedicated typed storage. This is cleaner:
    /// - Game zones contain only actual game objects (cards, emblems)
    /// - Effect semantics are explicit in the type system
    /// - No confusion between "real" command zone cards and bookkeeping
    ///
    /// Examples of persistent effects:
    /// - Airbend: "While exiled, you may cast it for {2}"
    /// - Imprint: "Exile a card from your hand. This remembers that card."
    /// - Suspend: Track time counters on suspended cards
    ///
    /// Effects are automatically cleaned up when their tracked cards change
    /// zones or when their source permanents leave the battlefield.
    pub persistent_effects: PersistentEffectStore,

    /// Delayed triggers waiting to fire on specific events.
    ///
    /// Delayed triggers are created by effects and fire when conditions are met:
    /// - Zone changes: "When this dies, return it to the battlefield tapped"
    /// - Phase changes: "At the beginning of the next end step, sacrifice it"
    ///
    /// Examples:
    /// - Earthbend: Return earthbent land to battlefield when it dies/is exiled
    /// - Flicker: Return exiled creature at end of turn
    /// - Suspend: Cast spell when last time counter is removed
    pub delayed_triggers: DelayedTriggerStore,
}

impl GameState {
    /// Create a new game with two players
    pub fn new_two_player(player1_name: String, player2_name: String, starting_life: i32) -> Self {
        Self::new_two_player_with_capacity(player1_name, player2_name, starting_life, 0)
    }

    /// Create a new game with two players and pre-allocated card storage
    ///
    /// The `deck_capacity` parameter specifies the expected total number of cards
    /// (typically 60 cards per deck × 2 players = 120 cards for standard constructed).
    /// Pre-sizing avoids HashMap resizes during deck loading.
    pub fn new_two_player_with_capacity(
        player1_name: String,
        player2_name: String,
        starting_life: i32,
        deck_capacity: usize,
    ) -> Self {
        let mut next_id = 0;

        // Create players with unified IDs
        let p1_id = PlayerId::new(next_id);
        next_id += 1;
        let p2_id = PlayerId::new(next_id);
        next_id += 1;

        let player1 = Player::new(p1_id, player1_name, starting_life);
        let player2 = Player::new(p2_id, player2_name, starting_life);

        let players = vec![player1, player2];

        let player_zones = vec![(p1_id, PlayerZones::new(p1_id)), (p2_id, PlayerZones::new(p2_id))];

        // Initialize mana source caches (one per player)
        let mana_caches = vec![
            (p1_id, ManaSourceCache::new(p1_id)),
            (p2_id, ManaSourceCache::new(p2_id)),
        ];

        // Use a unified PlayerId for shared zones (battlefield, stack)
        // These don't belong to a specific player, but we need an ID for the zone
        let shared_id = PlayerId::new(next_id);
        next_id += 1;

        // Pre-size EntityStore if capacity is specified
        let cards = if deck_capacity > 0 {
            EntityStore::with_capacity(deck_capacity)
        } else {
            EntityStore::new()
        };

        GameState {
            cards,
            players,
            player_zones,
            mana_caches,
            battlefield: CardZone::new(Zone::Battlefield, shared_id),
            stack: CardZone::new(Zone::Stack, shared_id),
            turn: TurnStructure::new_with_idx(p1_id, 0), // Player 1 starts at index 0
            combat: CombatState::new(),
            rng: RefCell::new(ChaCha12Rng::seed_from_u64(0)), // Default seed, will be reseeded by game initialization
            next_entity_id: next_id,
            undo_log: UndoLog::new(),
            logger: GameLogger::new(),
            token_definitions: std::collections::HashMap::new(),
            bump: Bump::new(),
            mana_state_version: 0,
            persistent_effects: PersistentEffectStore::new(),
            delayed_triggers: DelayedTriggerStore::new(),
        }
    }

    /// Set the RNG seed for deterministic gameplay
    ///
    /// This should be called during game initialization to set a specific seed
    /// for reproducible games. The seed affects all random decisions made by
    /// controllers and game logic.
    pub fn seed_rng(&mut self, seed: u64) {
        *self.rng.borrow_mut() = ChaCha12Rng::seed_from_u64(seed);
    }

    /// Get the action count (undo log length)
    ///
    /// Returns the number of reversible actions that have been performed.
    /// Used for network synchronization verification.
    #[inline]
    pub fn action_count(&self) -> u64 {
        self.undo_log.len() as u64
    }

    /// Increment the mana state version to invalidate ManaEngine cache
    ///
    /// Call this whenever cards enter/leave the battlefield or tap/untap.
    /// This is called automatically by move_card() for battlefield changes
    /// and by tap_permanent()/untap_permanent() for tap state changes.
    #[inline]
    pub fn increment_mana_version(&mut self) {
        self.mana_state_version = self.mana_state_version.wrapping_add(1);
    }

    /// Tap a permanent and log the action for undo
    ///
    /// This is the preferred way to tap permanents - it handles:
    /// - Setting the tapped state
    /// - Logging the undo action
    /// - Incrementing the mana state version for cache invalidation
    ///
    /// Returns Ok(()) if successful, Err if the card doesn't exist.
    pub fn tap_permanent(&mut self, card_id: CardId) -> Result<()> {
        let card = self.cards.get_mut(card_id)?;
        if card.tapped {
            return Ok(()); // Already tapped, no-op
        }
        card.tap();

        let prior_log_size = self.logger.log_count();
        self.undo_log.log(
            crate::undo::GameAction::TapCard { card_id, tapped: true },
            prior_log_size,
        );

        // Update mana caches (event-driven incremental update)
        // Read card data first to avoid borrow conflicts
        if let Some(card) = self.cards.try_get(card_id) {
            for (_, cache) in &mut self.mana_caches {
                cache.on_tap(card_id, card);
            }
        }

        self.increment_mana_version();
        Ok(())
    }

    /// Untap a permanent and log the action for undo
    ///
    /// This is the preferred way to untap permanents - it handles:
    /// - Setting the untapped state
    /// - Logging the undo action
    /// - Incrementing the mana state version for cache invalidation
    ///
    /// Returns Ok(()) if successful, Err if the card doesn't exist.
    pub fn untap_permanent(&mut self, card_id: CardId) -> Result<()> {
        let card = self.cards.get_mut(card_id)?;
        if !card.tapped {
            return Ok(()); // Already untapped, no-op
        }
        card.untap();

        let prior_log_size = self.logger.log_count();
        self.undo_log.log(
            crate::undo::GameAction::TapCard { card_id, tapped: false },
            prior_log_size,
        );

        // Update mana caches (event-driven incremental update)
        // Read card data first to avoid borrow conflicts
        if let Some(card) = self.cards.try_get(card_id) {
            for (_, cache) in &mut self.mana_caches {
                cache.on_untap(card_id, card);
            }
        }

        self.increment_mana_version();
        Ok(())
    }

    /// Shuffle a player's library using the game's RNG
    ///
    /// This is a convenience method to avoid borrow checker issues when
    /// accessing both the RNG and player zones.
    pub fn shuffle_library(&mut self, player_id: PlayerId) {
        use rand::seq::SliceRandom;
        // First, get a mutable reference to the library cards
        if let Some(zones) = self
            .player_zones
            .iter_mut()
            .find(|(id, _)| *id == player_id)
            .map(|(_, z)| z)
        {
            zones.library.cards.shuffle(&mut *self.rng.borrow_mut());
        }
    }

    /// Get next entity ID (unified across all entity types)
    /// Generic version that can return any `EntityId<T>` type
    pub fn next_id<T>(&mut self) -> EntityId<T> {
        let id = EntityId::new(self.next_entity_id);
        self.next_entity_id += 1;
        id
    }

    /// Convenience method for getting next card ID
    pub fn next_card_id(&mut self) -> CardId {
        self.next_id()
    }

    /// Convenience method for getting next player ID
    pub fn next_player_id(&mut self) -> PlayerId {
        self.next_id()
    }

    /// Legacy method for compatibility (deprecated)
    #[allow(dead_code)]
    pub fn next_entity_id(&mut self) -> CardId {
        self.next_card_id()
    }

    /// Get player zones for a specific player
    pub fn get_player_zones(&self, player_id: PlayerId) -> Option<&PlayerZones> {
        self.player_zones
            .iter()
            .find(|(id, _)| *id == player_id)
            .map(|(_, zones)| zones)
    }

    /// Get mutable player zones for a specific player
    pub fn get_player_zones_mut(&mut self, player_id: PlayerId) -> Option<&mut PlayerZones> {
        self.player_zones
            .iter_mut()
            .find(|(id, _)| *id == player_id)
            .map(|(_, zones)| zones)
    }

    /// Check if a card is in any player's exile zone
    ///
    /// Used by persistent effects like Airbend to verify a card is still exiled
    /// before allowing it to be cast.
    pub fn is_card_in_exile(&self, card_id: CardId) -> bool {
        self.player_zones
            .iter()
            .any(|(_, zones)| zones.exile.cards.contains(&card_id))
    }

    /// Get mana source cache for a specific player
    pub fn get_mana_cache(&self, player_id: PlayerId) -> Option<&ManaSourceCache> {
        self.mana_caches
            .iter()
            .find(|(id, _)| *id == player_id)
            .map(|(_, cache)| cache)
    }

    /// Get mutable mana source cache for a specific player
    pub fn get_mana_cache_mut(&mut self, player_id: PlayerId) -> Option<&mut ManaSourceCache> {
        self.mana_caches
            .iter_mut()
            .find(|(id, _)| *id == player_id)
            .map(|(_, cache)| cache)
    }

    /// Rebuild mana cache for a player if it needs rebuilding
    ///
    /// This is a helper for ManaEngine that handles borrow checker issues
    /// when calling rebuild_from_battlefield (which needs &mut cache and &GameState).
    pub fn rebuild_mana_cache_if_needed(&mut self, player_id: PlayerId) {
        // Check if rebuild is needed
        let needs_rebuild = self
            .get_mana_cache(player_id)
            .map(|c| c.needs_rebuild())
            .unwrap_or(false);

        if !needs_rebuild {
            return;
        }

        // Find cache index to rebuild
        let cache_idx = self.mana_caches.iter().position(|(id, _)| *id == player_id);

        if let Some(idx) = cache_idx {
            // SAFETY: We use raw pointers to split the borrow:
            // - cache_ptr: mutable access to the cache
            // - game_ptr: immutable access to GameState
            // This is safe because:
            // 1. We're accessing non-overlapping parts (cache vs rest of game)
            // 2. The cache is a distinct field in a Vec element
            // 3. rebuild_from_battlefield only reads from GameState
            let cache_ptr = &mut self.mana_caches[idx].1 as *mut ManaSourceCache;
            let game_ptr = self as *const GameState;
            unsafe {
                (*cache_ptr).rebuild_from_battlefield(&*game_ptr);
            }
        }
    }

    /// Get a player by ID
    pub fn get_player(&self, id: PlayerId) -> Result<&Player> {
        self.players
            .iter()
            .find(|p| p.id == id)
            .ok_or(crate::MtgError::EntityNotFound(id.as_u32()))
    }

    /// Get a mutable player by ID
    pub fn get_player_mut(&mut self, id: PlayerId) -> Result<&mut Player> {
        self.players
            .iter_mut()
            .find(|p| p.id == id)
            .ok_or(crate::MtgError::EntityNotFound(id.as_u32()))
    }

    /// Get player by index (for stable turn order)
    pub fn get_player_by_idx(&self, idx: usize) -> Option<&Player> {
        self.players.get(idx)
    }

    /// Get mutable player by index
    pub fn get_player_by_idx_mut(&mut self, idx: usize) -> Option<&mut Player> {
        self.players.get_mut(idx)
    }

    /// Get the index of a player by ID
    pub fn get_player_idx(&self, id: PlayerId) -> Option<usize> {
        self.players.iter().position(|p| p.id == id)
    }

    /// Get the next player in turn order (for 2+ players)
    pub fn get_next_player_idx(&self, current_idx: usize) -> usize {
        (current_idx + 1) % self.players.len()
    }

    /// For 2-player games, get the other player's index
    pub fn get_other_player_idx(&self, player_idx: usize) -> Option<usize> {
        if self.players.len() == 2 {
            Some(1 - player_idx)
        } else {
            None
        }
    }

    /// For 2-player games, get the other player's ID
    pub fn get_other_player_id(&self, player_id: PlayerId) -> Option<PlayerId> {
        if self.players.len() == 2 {
            self.players.iter().find(|p| p.id != player_id).map(|p| p.id)
        } else {
            None
        }
    }

    /// Move a card from one zone to another
    pub fn move_card(&mut self, card_id: CardId, from: Zone, to: Zone, owner: PlayerId) -> Result<()> {
        // Debug log card movement
        if let Ok(card) = self.cards.get(card_id) {
            log::debug!(target: "zone", "Moving card {} (id={}) from {:?} to {:?} (owner: player {})",
                card.name, card_id.as_u32(), from, to, owner.as_u32());
        } else {
            log::debug!(target: "zone", "Moving unknown card (id={}) from {:?} to {:?} (owner: player {})",
                card_id.as_u32(), from, to, owner.as_u32());
        }

        // State-based action: If a creature is leaving the battlefield, detach all Equipment from it
        if from == Zone::Battlefield {
            if let Ok(card) = self.cards.get(card_id) {
                if card.is_creature() {
                    // Collect Equipment to detach (to avoid borrow issues)
                    let equipment_to_detach: Vec<CardId> = self
                        .battlefield
                        .cards
                        .iter()
                        .filter_map(|&equip_id| {
                            let equip = self.cards.get(equip_id).ok()?;
                            if equip.is_equipment() && equip.attached_to == Some(card_id) {
                                Some(equip_id)
                            } else {
                                None
                            }
                        })
                        .collect();

                    // Detach all Equipment
                    for equip_id in equipment_to_detach {
                        if let Ok(equip) = self.cards.get_mut(equip_id) {
                            equip.attached_to = None;
                            self.logger
                                .verbose(&format!("{} detaches (creature left battlefield)", equip.name));
                        }
                    }
                }
            }
        }

        // Remove from source zone
        let removed = match from {
            Zone::Battlefield => self.battlefield.remove(card_id),
            Zone::Stack => self.stack.remove(card_id),
            Zone::Library | Zone::Hand | Zone::Graveyard | Zone::Exile | Zone::Command => {
                if let Some(zones) = self.get_player_zones_mut(owner) {
                    if let Some(zone) = zones.get_zone_mut(from) {
                        zone.remove(card_id)
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
        };

        if !removed {
            return Err(crate::MtgError::InvalidAction(format!(
                "Card {card_id} not found in source zone"
            )));
        }

        // Add to destination zone
        match to {
            Zone::Battlefield => self.battlefield.add(card_id),
            Zone::Stack => self.stack.add(card_id),
            Zone::Library | Zone::Hand | Zone::Graveyard | Zone::Exile | Zone::Command => {
                if let Some(zones) = self.get_player_zones_mut(owner) {
                    if let Some(zone) = zones.get_zone_mut(to) {
                        zone.add(card_id);
                    }
                }
            }
        }

        // Handle cards that enter the battlefield tapped (e.g., Thriving lands)
        if to == Zone::Battlefield {
            if let Ok(card) = self.cards.get(card_id) {
                if card.cache.enters_tapped {
                    // Must drop the immutable borrow before getting mutable borrow
                    let card_name = card.name.clone();
                    if let Ok(card_mut) = self.cards.get_mut(card_id) {
                        card_mut.tapped = true;
                        self.logger
                            .verbose(&format!("{} ({}) enters the battlefield tapped", card_name, card_id));
                    }
                }
            }

            // Handle ETB color choice (e.g., Thriving lands)
            if let Ok(card) = self.cards.get(card_id) {
                if card.cache.etb_choose_color {
                    let exclude_colors = card.cache.etb_exclude_colors.clone();
                    let card_name = card.name.clone();

                    // Pick the most prominent color in the player's deck (excluding excluded colors)
                    let chosen = self.pick_prominent_color(owner, &exclude_colors);

                    if let Ok(card_mut) = self.cards.get_mut(card_id) {
                        card_mut.chosen_color = Some(chosen);
                        self.logger
                            .normal(&format!("{} ({}) - chose {:?}", card_name, card_id, chosen));
                    }
                }
            }
        }

        // Log the action with prior log size for undo synchronization
        let prior_log_size = self.logger.log_count();
        self.undo_log.log(
            crate::undo::GameAction::MoveCard {
                card_id,
                from_zone: from,
                to_zone: to,
                owner,
            },
            prior_log_size,
        );

        // Log significant zone transitions to the gamelog
        // This ensures visibility into all card movements for debugging and replay
        if let Ok(card) = self.cards.get(card_id) {
            let card_name = &card.name;
            match (from, to) {
                (Zone::Battlefield, Zone::Graveyard) => {
                    // Creature died or permanent destroyed (not from lethal damage - that's logged elsewhere)
                    // Only log if not already logged by state-based actions
                    self.logger
                        .verbose(&format!("{} ({}) goes to graveyard", card_name, card_id));
                }
                (Zone::Battlefield, Zone::Exile) => {
                    self.logger.normal(&format!("{} ({}) is exiled", card_name, card_id));
                }
                (Zone::Battlefield, Zone::Hand) => {
                    self.logger
                        .normal(&format!("{} ({}) is returned to hand", card_name, card_id));
                }
                (Zone::Stack, Zone::Graveyard) => {
                    // Spell resolved or was countered - logged elsewhere
                }
                (Zone::Hand, Zone::Graveyard) => {
                    self.logger.normal(&format!("{} is discarded", card_name));
                }
                (Zone::Library, Zone::Graveyard) => {
                    // Mill - don't spam, this is logged by mill effect
                }
                _ => {
                    // Other moves are either logged elsewhere or not significant
                }
            }
        }

        // Update mana caches (event-driven incremental update)
        // Read card data first to avoid borrow conflicts
        if let Some(card) = self.cards.try_get(card_id) {
            if from == Zone::Battlefield {
                // Card left battlefield - remove from caches
                for (_, cache) in &mut self.mana_caches {
                    cache.on_card_left(card_id, card);
                }
            }
            if to == Zone::Battlefield {
                // Card entered battlefield - add to caches
                for (_, cache) in &mut self.mana_caches {
                    cache.on_card_entered(card_id, card);
                }
            }
        }

        // Increment mana state version if battlefield changed
        // This invalidates ManaEngine cache so next query rebuilds
        if from == Zone::Battlefield || to == Zone::Battlefield {
            self.mana_state_version = self.mana_state_version.wrapping_add(1);
        }

        // Clean up persistent effects when a tracked card leaves a zone
        // This handles effects like Airbend that track cards in exile
        let effects_to_remove = self
            .persistent_effects
            .find_effects_to_cleanup_on_zone_change(card_id, from);
        if !effects_to_remove.is_empty() {
            log::debug!(target: "persistent_effects", "Cleaning up {} effects on zone change for card {}", effects_to_remove.len(), card_id.as_u32());
            self.persistent_effects.remove_many(&effects_to_remove);
        }

        // Check and fire delayed triggers for this zone change
        // This handles effects like Earthbend that return cards to battlefield
        // Note: This may cause recursive move_card calls (e.g., return to battlefield)
        self.check_delayed_triggers_on_zone_change(card_id, from, to)?;

        Ok(())
    }

    /// Pick the most prominent color in a player's deck, excluding specified colors
    ///
    /// Used for "choose a color" ETB abilities like Thriving lands.
    /// Analyzes mana costs in hand, library, and graveyard to find the most needed color.
    fn pick_prominent_color(&self, player_id: PlayerId, exclude: &[Color]) -> Color {
        use std::collections::HashMap;

        let mut color_counts: HashMap<Color, u32> = HashMap::new();

        // Count colors from cards in hand, library, and graveyard
        let zones_to_check = if let Some(zones) = self.get_player_zones(player_id) {
            vec![&zones.hand, &zones.library, &zones.graveyard]
        } else {
            vec![]
        };

        for zone in zones_to_check {
            for &card_id in zone.cards.iter() {
                if let Ok(card) = self.cards.get(card_id) {
                    // Count mana symbols in the mana cost
                    if card.mana_cost.white > 0 {
                        *color_counts.entry(Color::White).or_insert(0) += card.mana_cost.white as u32;
                    }
                    if card.mana_cost.blue > 0 {
                        *color_counts.entry(Color::Blue).or_insert(0) += card.mana_cost.blue as u32;
                    }
                    if card.mana_cost.black > 0 {
                        *color_counts.entry(Color::Black).or_insert(0) += card.mana_cost.black as u32;
                    }
                    if card.mana_cost.red > 0 {
                        *color_counts.entry(Color::Red).or_insert(0) += card.mana_cost.red as u32;
                    }
                    if card.mana_cost.green > 0 {
                        *color_counts.entry(Color::Green).or_insert(0) += card.mana_cost.green as u32;
                    }
                }
            }
        }

        // Remove excluded colors
        for color in exclude {
            color_counts.remove(color);
        }

        // Return the most prominent color, or a default if none found
        color_counts
            .into_iter()
            .max_by_key(|(_color, count)| *count)
            .map(|(color, _)| color)
            .unwrap_or_else(|| {
                // Default: pick first non-excluded color
                [Color::White, Color::Blue, Color::Black, Color::Red, Color::Green]
                    .into_iter()
                    .find(|c| !exclude.contains(c))
                    .unwrap_or(Color::White)
            })
    }

    /// Print state hash to normal log output if debug mode is enabled
    ///
    /// This is called before logging game actions to help debug divergence.
    /// Prints format: [STATE:a3f7b2c1] message
    #[inline]
    pub fn debug_log_state_hash(&self, message: &str) {
        if self.logger.debug_state_hash_enabled() {
            use crate::game::{compute_state_hash, format_hash};
            let hash = compute_state_hash(self);
            // Use the logger's normal() method to output to stdout instead of stderr
            // This makes state hashes part of the deterministic game output
            self.logger
                .normal(&format!("[STATE:{}] {}", format_hash(hash), message));
        }
    }

    /// Draw a card for a player
    pub fn draw_card(&mut self, player_id: PlayerId) -> Result<Option<CardId>> {
        if let Some(zones) = self.get_player_zones_mut(player_id) {
            // Debug: check if library is in remote mode
            let is_remote = zones.library.is_remote_library();
            let pending_reveals = zones.library.pending_reveals_count();
            let lib_size = zones.library.len();
            log::debug!(
                "draw_card: player {} library is_remote={}, pending_reveals={}, cards_len={}",
                player_id.as_u32(),
                is_remote,
                pending_reveals,
                lib_size
            );

            // Try to draw from the library
            if let Some(card_id) = zones.library.draw_top() {
                // Normal draw with known card ID
                zones.hand.add(card_id);

                // Log the card movement for undo with prior log size
                let prior_log_size = self.logger.log_count();
                self.undo_log.log(
                    crate::undo::GameAction::MoveCard {
                        card_id,
                        from_zone: crate::zones::Zone::Library,
                        to_zone: crate::zones::Zone::Hand,
                        owner: player_id,
                    },
                    prior_log_size,
                );

                return Ok(Some(card_id));
            } else if is_remote && lib_size > 0 {
                // Remote library has cards but no pending reveal - this is a hidden draw
                // (opponent drawing a card, we don't know what card it is)
                //
                // We still need to:
                // 1. Decrement library size
                // 2. Increment hand's hidden_card_count
                // 3. Log HiddenDraw action to keep action_count in sync
                zones.library.decrement_size();
                zones.hand.increment_hidden_card_count();

                let prior_log_size = self.logger.log_count();
                self.undo_log
                    .log(crate::undo::GameAction::HiddenDraw { player_id }, prior_log_size);

                log::debug!(
                    "draw_card: player {} hidden draw (no reveal, opponent's card)",
                    player_id.as_u32()
                );

                // Return None since we don't know the card ID
                return Ok(None);
            }
        }
        Ok(None)
    }

    /// Discard a hidden card (network client-side only)
    ///
    /// Used when opponent discards a card but we don't know what card it is.
    /// This decrements hand's hidden_card_count and increments graveyard's hidden_card_count.
    ///
    /// Returns true if successful, false if no hidden cards to discard.
    pub fn discard_hidden(&mut self, player_id: PlayerId) -> bool {
        if let Some(zones) = self.get_player_zones_mut(player_id) {
            if zones.hand.hidden_card_count > 0 {
                zones.hand.decrement_hidden_card_count();
                zones.graveyard.increment_hidden_card_count();

                let prior_log_size = self.logger.log_count();
                self.undo_log
                    .log(crate::undo::GameAction::HiddenDiscard { player_id }, prior_log_size);

                log::debug!(
                    "discard_hidden: player {} hidden discard (opponent's card)",
                    player_id.as_u32()
                );

                return true;
            }
        }
        false
    }

    /// Mill cards from library to graveyard (used by mill effects)
    ///
    /// Returns SmallVec to avoid heap allocation for typical mill counts (up to 8 cards).
    /// Mill effects typically mill 1-7 cards (e.g., "mill 3 cards").
    pub fn mill_cards(&mut self, player_id: PlayerId, count: u8) -> Result<SmallVec<[CardId; 8]>> {
        let mut milled_cards: SmallVec<[CardId; 8]> = SmallVec::new();

        for _ in 0..count {
            // Try to draw from library
            let card_id = if let Some(zones) = self.get_player_zones(player_id) {
                zones.library.cards.last().copied()
            } else {
                None
            };

            if let Some(card_id) = card_id {
                // Move the card from library to graveyard
                self.move_card(
                    card_id,
                    crate::zones::Zone::Library,
                    crate::zones::Zone::Graveyard,
                    player_id,
                )?;
                milled_cards.push(card_id);
            } else {
                // Library is empty, can't mill more cards
                break;
            }
        }

        Ok(milled_cards)
    }

    /// Counter a spell on the stack
    /// This removes the spell from the stack and moves it to its owner's graveyard
    pub fn counter_spell(&mut self, spell_id: CardId) -> Result<()> {
        // Check if the spell is on the stack
        if !self.stack.contains(spell_id) {
            return Err(crate::MtgError::InvalidAction(
                "Cannot counter a spell that is not on the stack".to_string(),
            ));
        }

        // Get the spell's owner to determine which graveyard it goes to
        let owner_id = {
            let card = self.cards.get(spell_id)?;
            card.owner
        };

        // Remove from stack
        self.stack.remove(spell_id);

        // Move to owner's graveyard
        if let Some(zones) = self.get_player_zones_mut(owner_id) {
            zones.graveyard.add(spell_id);
        }

        // Log the counter action with prior log size
        let prior_log_size = self.logger.log_count();
        self.undo_log.log(
            crate::undo::GameAction::MoveCard {
                card_id: spell_id,
                from_zone: crate::zones::Zone::Stack,
                to_zone: crate::zones::Zone::Graveyard,
                owner: owner_id,
            },
            prior_log_size,
        );

        Ok(())
    }

    /// Untap all permanents controlled by a player
    pub fn untap_all(&mut self, player_id: PlayerId) -> Result<()> {
        for card_id in self.battlefield.cards.iter() {
            if let Ok(card) = self.cards.get_mut(*card_id) {
                if card.controller == player_id && card.tapped {
                    card.untap();
                    // Log the untap action with prior log size
                    let prior_log_size = self.logger.log_count();
                    self.undo_log.log(
                        crate::undo::GameAction::TapCard {
                            card_id: *card_id,
                            tapped: false,
                        },
                        prior_log_size,
                    );
                }
            }
        }
        Ok(())
    }

    /// Add counters to a card and log for undo
    pub fn add_counters(&mut self, card_id: CardId, counter_type: crate::core::CounterType, amount: u8) -> Result<()> {
        if let Ok(card) = self.cards.get_mut(card_id) {
            card.add_counter(counter_type, amount);

            // Log the action with prior log size
            let prior_log_size = self.logger.log_count();
            self.undo_log.log(
                crate::undo::GameAction::AddCounter {
                    card_id,
                    counter_type,
                    amount,
                },
                prior_log_size,
            );

            Ok(())
        } else {
            Err(crate::MtgError::EntityNotFound(card_id.as_u32()))
        }
    }

    /// Remove counters from a card and log for undo
    pub fn remove_counters(
        &mut self,
        card_id: CardId,
        counter_type: crate::core::CounterType,
        amount: u8,
    ) -> Result<u8> {
        if let Ok(card) = self.cards.get_mut(card_id) {
            let removed = card.remove_counter(counter_type, amount);

            // Log the action with prior log size
            let prior_log_size = self.logger.log_count();
            self.undo_log.log(
                crate::undo::GameAction::RemoveCounter {
                    card_id,
                    counter_type,
                    amount: removed,
                },
                prior_log_size,
            );

            Ok(removed)
        } else {
            Err(crate::MtgError::EntityNotFound(card_id.as_u32()))
        }
    }

    /// Check state-based actions for lethal damage (MTG CR 704.5g)
    ///
    /// If a creature has damage marked on it greater than or equal to its toughness,
    /// and it doesn't have indestructible, that creature's controller puts it into the graveyard.
    ///
    /// This should be called after damage is dealt or whenever state-based actions are checked.
    pub fn check_lethal_damage(&mut self) -> Result<()> {
        // Collect creatures that need to die (to avoid borrow checker issues)
        let creatures_to_destroy: Vec<(CardId, PlayerId)> = self
            .battlefield
            .cards
            .iter()
            .filter_map(|&card_id| {
                let card = self.cards.get(card_id).ok()?;
                if !card.is_creature() {
                    return None;
                }

                // MTG CR 704.5g: Creature has lethal damage if damage >= toughness
                let toughness = card.current_toughness();
                let has_lethal = card.damage >= toughness as i32;

                // Debug: Log SBA check for creatures with damage or low toughness
                if card.damage > 0 || toughness <= 0 || card.name.as_str().contains("Peter Porker") {
                    log::debug!(target: "sba", "SBA check: {} (id={}) damage={} toughness={} has_lethal={} indestructible={}",
                        card.name, card_id.as_u32(), card.damage, toughness, has_lethal, card.has_indestructible());
                }

                // MTG CR 702.12b: Indestructible permanents aren't destroyed by lethal damage
                if has_lethal && !card.has_indestructible() {
                    Some((card_id, card.owner))
                } else {
                    None
                }
            })
            .collect();

        // Destroy all creatures with lethal damage
        for (card_id, owner) in creatures_to_destroy {
            let card_name = self.cards.get(card_id).map(|c| c.name.clone()).ok();
            self.move_card(card_id, Zone::Battlefield, Zone::Graveyard, owner)?;
            if let Some(name) = card_name {
                self.logger
                    .gamelog(&format!("{} ({}) dies from lethal damage", name, card_id));
            }
        }

        Ok(())
    }

    /// Clear temporary effects at end of turn (Cleanup step)
    /// This resets power/toughness bonuses from pump spells and clears damage
    /// MTG CR 514.2: Damage marked on permanents is removed (CR 704.5f)
    pub fn cleanup_temporary_effects(&mut self) {
        for card_id in self.battlefield.cards.iter() {
            if let Ok(card) = self.cards.get_mut(*card_id) {
                // Reset temporary bonuses (pump effects last until end of turn)
                card.power_bonus = 0;
                card.toughness_bonus = 0;
                // Reset temporary base P/T overrides (from Animate effects)
                card.clear_temp_base_stats();
                // Clear damage marked on permanents (MTG CR 514.2, CR 704.5f)
                card.damage = 0;
            }
        }
    }

    /// Advance the game to the next step
    pub fn advance_step(&mut self) -> Result<()> {
        let from_step = self.turn.current_step;

        // If entering cleanup step, clean up temporary effects
        if from_step == crate::game::Step::End && self.turn.current_step.next() == Some(crate::game::Step::Cleanup) {
            self.cleanup_temporary_effects();
        }

        if !self.turn.advance_step() {
            // End of turn, move to next player
            let from_player = self.turn.active_player;
            let next_player = self.get_next_player(self.turn.active_player)?;
            let old_turn_number = self.turn.turn_number;

            // Serialize RNG state BEFORE changing turns
            // This captures the RNG state at the END of the current turn,
            // which will be the START of the next turn after next_turn() is called
            // Using bincode for compact serialization (56 bytes vs 152 bytes for JSON)
            // SmallVec<[u8; 64]> fits ChaCha12Rng serialization (56 bytes, no heap allocation)
            //
            // OPTIMIZATION: Use serialize_into with a fixed buffer to avoid Vec allocation.
            // ChaCha12Rng bincode serialization is exactly 56 bytes, so we use a [u8; 64] buffer.
            let rng_state = {
                let rng = self.rng.borrow();
                let mut buffer = [0u8; 64];
                let mut cursor = std::io::Cursor::new(&mut buffer[..]);
                if bincode::serialize_into(&mut cursor, &*rng).is_ok() {
                    let len = cursor.position() as usize;
                    // INVARIANT: ChaCha12Rng bincode serialization is exactly 56 bytes
                    debug_assert_eq!(
                        len, 56,
                        "ChaCha12Rng bincode serialization changed from 56 bytes to {} bytes - update buffer size!",
                        len
                    );
                    // Create SmallVec from the fixed buffer slice (no heap allocation)
                    Some(smallvec::SmallVec::from_slice(&buffer[..len]))
                } else {
                    None
                }
            };

            self.turn.next_turn(next_player);

            // Log the turn change with RNG state from before the turn change and prior log size
            let prior_log_size = self.logger.log_count();
            self.undo_log.log(
                crate::undo::GameAction::ChangeTurn {
                    from_player,
                    to_player: next_player,
                    turn_number: old_turn_number + 1,
                    rng_state,
                },
                prior_log_size,
            );

            // Log turn transfer indicator with life totals
            let new_turn_num = old_turn_number + 1;
            let active_player_name = self
                .get_player(next_player)
                .map(|p| p.name.as_str())
                .unwrap_or("Unknown");
            let active_player_life = self.get_player(next_player).map(|p| p.life).unwrap_or(0);
            let other_player_name = self
                .get_other_player_id(next_player)
                .and_then(|id| self.get_player(id).ok())
                .map(|p| p.name.as_str())
                .unwrap_or("Unknown");
            let other_player_life = self
                .get_other_player_id(next_player)
                .and_then(|id| self.get_player(id).ok())
                .map(|p| p.life)
                .unwrap_or(0);

            // Add a newline before the turn separator for visual separation
            self.logger.turn_separator("");

            let turn_msg = format!(
                "  >>> Turn {} - {} {} ({} {}) <<<<",
                new_turn_num, active_player_name, active_player_life, other_player_name, other_player_life
            );
            self.logger.turn_separator(&turn_msg);

            // Reset per-turn state
            if let Ok(player) = self.get_player_mut(next_player) {
                player.reset_lands_played();
            }
        } else {
            // Log the step advance with prior log size
            let prior_log_size = self.logger.log_count();
            self.undo_log.log(
                crate::undo::GameAction::AdvanceStep {
                    from_step,
                    to_step: self.turn.current_step,
                },
                prior_log_size,
            );
        }
        Ok(())
    }

    /// Get the next player in turn order
    fn get_next_player(&self, current_player: PlayerId) -> Result<PlayerId> {
        let current_idx = self
            .get_player_idx(current_player)
            .ok_or(crate::MtgError::EntityNotFound(current_player.as_u32()))?;
        let next_idx = self.get_next_player_idx(current_idx);
        Ok(self.players[next_idx].id)
    }

    /// Check if the game is over
    pub fn is_game_over(&self) -> bool {
        self.players.iter().filter(|p| !p.has_lost).count() <= 1
    }

    /// Get the winner (if game is over)
    pub fn get_winner(&self) -> Option<PlayerId> {
        if !self.is_game_over() {
            return None;
        }
        self.players.iter().find(|p| !p.has_lost).map(|p| p.id)
    }

    /// Undo back to the previous choice point for a specific player
    ///
    /// Keeps undoing actions until a ChoicePoint action for the specified player is found.
    /// This will undo ALL intervening choices (including other players' choices) until
    /// reaching the target player's previous choice.
    ///
    /// Returns (actions_undone, choice_log_size) where:
    /// - actions_undone: number of non-ChoicePoint actions undone
    /// - choice_log_size: the log size to truncate to (from the ChoicePoint)
    ///
    /// Returns Ok(None) if no ChoicePoint for the specified player is found.
    ///
    /// Note: Wildcard matches are intentional - we match ChoicePoint specially,
    /// then handle all other GameAction variants through detailed inner matching.
    #[allow(clippy::wildcard_enum_match_arm)]
    pub fn undo_to_previous_choice_point(&mut self, requesting_player: PlayerId) -> Result<Option<(usize, usize)>> {
        // Debug: Log initial state
        log::debug!(
            "[UNDO] Starting undo_to_previous_choice_point for player {}",
            requesting_player.as_u32()
        );
        log::debug!(
            "[UNDO]   Initial: undo_log.len()={}, logger.log_count()={}, logger.choice_count()={}",
            self.undo_log.len(),
            self.logger.log_count(),
            self.logger.choice_count()
        );

        // IMPORTANT: First check if there's a ChoicePoint for this player in the log
        // We must do this BEFORE undoing anything, otherwise we corrupt state if none exists
        let has_choice_point = self.undo_log.actions().iter().any(|action| {
            matches!(action, crate::undo::GameAction::ChoicePoint { player_id, .. } if *player_id == requesting_player)
        });

        if !has_choice_point {
            log::debug!(
                "[UNDO] No ChoicePoint found for player {} in undo log - returning early WITHOUT undoing",
                requesting_player.as_u32()
            );
            return Ok(None);
        }

        log::debug!(
            "[UNDO] Found at least one ChoicePoint for player {}, proceeding with undo",
            requesting_player.as_u32()
        );

        let mut actions_undone = 0;
        let mut choice_log_size = None;

        // Keep undoing until we hit a ChoicePoint for the requesting player
        while let Some((action, prior_log_size)) = self.undo_log.pop() {
            log::debug!(
                "[UNDO]   Popped action (prior_log_size={}): {:?}",
                prior_log_size,
                action
            );
            match action {
                crate::undo::GameAction::ChoicePoint {
                    player_id, choice_id, ..
                } => {
                    log::debug!(
                        "[UNDO]     ChoicePoint for player {}, choice_id={}. Current choice count: {}",
                        player_id.as_u32(),
                        choice_id,
                        self.logger.choice_count()
                    );

                    if player_id == requesting_player {
                        // Found a choice point for the requesting player! Save the log size and stop
                        // Set choice_count to reflect choices made BEFORE this point
                        // (If we're restoring to choice_id=3, then choices 1 and 2 were made, so count=2)
                        let target_choice_count = choice_id.saturating_sub(1) as usize;
                        log::debug!(
                            "[UNDO]     *** Found target ChoicePoint! Setting choice_count from {} to {}",
                            self.logger.choice_count(),
                            target_choice_count
                        );
                        // Directly set the choice count instead of decrementing
                        self.logger.set_choice_count(target_choice_count);
                        log::debug!("[UNDO]     *** prior_log_size={}", prior_log_size);
                        choice_log_size = Some(prior_log_size);
                        break;
                    }
                    // Otherwise, this was another player's choice - keep undoing
                    // Don't decrement here - we'll set the correct count when we find the target
                    log::debug!("[UNDO]     Different player's choice, continuing undo...");
                }
                _ => {
                    // Not a choice point - undo this action
                    match action {
                        crate::undo::GameAction::MoveCard {
                            card_id,
                            from_zone,
                            to_zone,
                            owner,
                        } => {
                            // Move card back from to_zone to from_zone
                            let removed = match to_zone {
                                Zone::Battlefield => self.battlefield.remove(card_id),
                                Zone::Stack => self.stack.remove(card_id),
                                _ => {
                                    if let Some(zones) = self.get_player_zones_mut(owner) {
                                        if let Some(zone) = zones.get_zone_mut(to_zone) {
                                            zone.remove(card_id)
                                        } else {
                                            false
                                        }
                                    } else {
                                        false
                                    }
                                }
                            };

                            if removed {
                                match from_zone {
                                    Zone::Battlefield => self.battlefield.add(card_id),
                                    Zone::Stack => self.stack.add(card_id),
                                    _ => {
                                        if let Some(zones) = self.get_player_zones_mut(owner) {
                                            if let Some(zone) = zones.get_zone_mut(from_zone) {
                                                zone.add(card_id);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        crate::undo::GameAction::TapCard { card_id, tapped } => {
                            if let Ok(card) = self.cards.get_mut(card_id) {
                                if tapped {
                                    card.untap();
                                } else {
                                    card.tap();
                                }
                            }
                        }
                        crate::undo::GameAction::ModifyLife { player_id, delta } => {
                            if let Ok(player) = self.get_player_mut(player_id) {
                                if delta > 0 {
                                    player.lose_life(delta);
                                } else {
                                    player.gain_life(-delta);
                                }
                                if player.life > 0 {
                                    player.has_lost = false;
                                }
                            }
                        }
                        crate::undo::GameAction::AddMana { player_id, mana } => {
                            if let Ok(player) = self.get_player_mut(player_id) {
                                if player.mana_pool.white >= mana.white {
                                    player.mana_pool.white -= mana.white;
                                }
                                if player.mana_pool.blue >= mana.blue {
                                    player.mana_pool.blue -= mana.blue;
                                }
                                if player.mana_pool.black >= mana.black {
                                    player.mana_pool.black -= mana.black;
                                }
                                if player.mana_pool.red >= mana.red {
                                    player.mana_pool.red -= mana.red;
                                }
                                if player.mana_pool.green >= mana.green {
                                    player.mana_pool.green -= mana.green;
                                }
                                if player.mana_pool.colorless >= mana.colorless {
                                    player.mana_pool.colorless -= mana.colorless;
                                }
                            }
                        }
                        crate::undo::GameAction::EmptyManaPool {
                            player_id,
                            prev_white,
                            prev_blue,
                            prev_black,
                            prev_red,
                            prev_green,
                            prev_colorless,
                        } => {
                            if let Ok(player) = self.get_player_mut(player_id) {
                                player.mana_pool.white = prev_white;
                                player.mana_pool.blue = prev_blue;
                                player.mana_pool.black = prev_black;
                                player.mana_pool.red = prev_red;
                                player.mana_pool.green = prev_green;
                                player.mana_pool.colorless = prev_colorless;
                            }
                        }
                        crate::undo::GameAction::AddCounter {
                            card_id,
                            counter_type,
                            amount,
                        } => {
                            if let Ok(card) = self.cards.get_mut(card_id) {
                                card.remove_counter(counter_type, amount);
                            }
                        }
                        crate::undo::GameAction::RemoveCounter {
                            card_id,
                            counter_type,
                            amount,
                        } => {
                            if let Ok(card) = self.cards.get_mut(card_id) {
                                card.add_counter(counter_type, amount);
                            }
                        }
                        crate::undo::GameAction::AdvanceStep { from_step, to_step: _ } => {
                            self.turn.current_step = from_step;
                        }
                        crate::undo::GameAction::ChangeTurn {
                            from_player,
                            to_player: _,
                            turn_number,
                            rng_state,
                        } => {
                            self.turn.active_player = from_player;
                            self.turn.turn_number = turn_number - 1;

                            if let Some(rng_bytes) = rng_state {
                                if let Ok(rng) = serde_json::from_slice::<ChaCha12Rng>(&rng_bytes) {
                                    *self.rng.borrow_mut() = rng;
                                }
                            }
                        }
                        crate::undo::GameAction::PumpCreature {
                            card_id,
                            power_delta,
                            toughness_delta,
                        } => {
                            if let Ok(card) = self.cards.get_mut(card_id) {
                                card.power_bonus -= power_delta;
                                card.toughness_bonus -= toughness_delta;
                            }
                        }
                        crate::undo::GameAction::SetTurnEnteredBattlefield {
                            card_id,
                            old_value,
                            new_value: _,
                        } => {
                            if let Ok(card) = self.cards.get_mut(card_id) {
                                card.turn_entered_battlefield = old_value;
                            }
                        }
                        crate::undo::GameAction::SetLandsPlayedThisTurn {
                            player_id,
                            old_value,
                            new_value: _,
                        } => {
                            if let Ok(player) = self.get_player_mut(player_id) {
                                player.lands_played_this_turn = old_value;
                            }
                        }
                        crate::undo::GameAction::SetAttachedTo {
                            equipment_id,
                            old_target,
                            new_target: _,
                        } => {
                            if let Ok(equipment) = self.cards.get_mut(equipment_id) {
                                equipment.attached_to = old_target;
                            }
                        }
                        _ => {}
                    }

                    actions_undone += 1;
                    // Truncate logger to the prior size
                    log::debug!(
                        "[UNDO]     Truncating logger from {} to {}",
                        self.logger.log_count(),
                        prior_log_size
                    );
                    self.logger.truncate_to(prior_log_size);
                    log::debug!(
                        "[UNDO]     Logger after truncate: log_count={}",
                        self.logger.log_count()
                    );
                }
            }
        }

        // After undo, mark all mana caches as needing rebuild
        // (Lazy rebuild on next query - cheaper than incrementally reversing events)
        for (_, cache) in &mut self.mana_caches {
            cache.mark_dirty();
        }

        // Increment mana state version to invalidate ManaEngine memoization
        // Without this, ManaEngine might use stale cached state from before the undo
        self.increment_mana_version();

        if let Some(log_size) = choice_log_size {
            log::debug!(
                "[UNDO] Undo complete: actions_undone={}, choice_log_size={}",
                actions_undone,
                log_size
            );
            log::debug!(
                "[UNDO]   Final: undo_log.len()={}, logger.log_count()={}, logger.choice_count()={}",
                self.undo_log.len(),
                self.logger.log_count(),
                self.logger.choice_count()
            );
            log::debug!("[UNDO]   Will truncate logger to {} in caller", log_size);
            Ok(Some((actions_undone, log_size)))
        } else {
            log::debug!(
                "[UNDO] Undo complete: No ChoicePoint found for player {}",
                requesting_player.as_u32()
            );
            log::debug!(
                "[UNDO]   Final: undo_log.len()={}, logger.log_count()={}, logger.choice_count()={}",
                self.undo_log.len(),
                self.logger.log_count(),
                self.logger.choice_count()
            );
            Ok(None)
        }
    }

    /// Undo the most recent action
    ///
    /// Pops the last action from the undo log and reverts it.
    /// Returns Ok(Some(prior_log_size)) to truncate logs to, Ok(None) if log is empty.
    pub fn undo(&mut self) -> Result<Option<usize>> {
        if let Some((action, prior_log_size)) = self.undo_log.pop() {
            match action {
                crate::undo::GameAction::MoveCard {
                    card_id,
                    from_zone,
                    to_zone,
                    owner,
                } => {
                    // Move card back from to_zone to from_zone
                    // Don't log this action since it's a revert
                    let removed = match to_zone {
                        Zone::Battlefield => self.battlefield.remove(card_id),
                        Zone::Stack => self.stack.remove(card_id),
                        Zone::Library | Zone::Hand | Zone::Graveyard | Zone::Exile | Zone::Command => {
                            if let Some(zones) = self.get_player_zones_mut(owner) {
                                if let Some(zone) = zones.get_zone_mut(to_zone) {
                                    zone.remove(card_id)
                                } else {
                                    eprintln!("UNDO BUG: Failed to get zone {to_zone:?} for undo");
                                    false
                                }
                            } else {
                                eprintln!("UNDO BUG: Failed to get player zones for {owner:?}");
                                false
                            }
                        }
                    };

                    if !removed {
                        // Find where the card actually is
                        let mut actual_zone = None;
                        if self.battlefield.contains(card_id) {
                            actual_zone = Some("Battlefield");
                        } else if self.stack.contains(card_id) {
                            actual_zone = Some("Stack");
                        } else if let Some(zones) = self.get_player_zones(owner) {
                            if zones.hand.contains(card_id) {
                                actual_zone = Some("Hand");
                            } else if zones.library.contains(card_id) {
                                actual_zone = Some("Library");
                            } else if zones.graveyard.contains(card_id) {
                                actual_zone = Some("Graveyard");
                            } else if zones.exile.contains(card_id) {
                                actual_zone = Some("Exile");
                            }
                        }
                        eprintln!("UNDO BUG: Card {} not found in to_zone {:?}, cannot undo move from {:?} → {:?}. Card is actually in: {:?}",
                                  card_id.as_u32(), to_zone, from_zone, to_zone, actual_zone);
                    } else {
                        match from_zone {
                            Zone::Battlefield => self.battlefield.add(card_id),
                            Zone::Stack => self.stack.add(card_id),
                            Zone::Library | Zone::Hand | Zone::Graveyard | Zone::Exile | Zone::Command => {
                                if let Some(zones) = self.get_player_zones_mut(owner) {
                                    if let Some(zone) = zones.get_zone_mut(from_zone) {
                                        zone.add(card_id);
                                    }
                                }
                            }
                        }
                    }
                }
                crate::undo::GameAction::TapCard { card_id, tapped } => {
                    // Reverse the tap state
                    if let Ok(card) = self.cards.get_mut(card_id) {
                        if tapped {
                            card.untap();
                        } else {
                            card.tap();
                        }
                    }
                }
                crate::undo::GameAction::ModifyLife { player_id, delta } => {
                    // Apply the negative of the delta
                    if let Ok(player) = self.get_player_mut(player_id) {
                        if delta > 0 {
                            player.lose_life(delta);
                        } else {
                            player.gain_life(-delta);
                        }
                        // Recheck has_lost status
                        if player.life > 0 {
                            player.has_lost = false;
                        }
                    }
                }
                crate::undo::GameAction::AddMana { player_id, mana } => {
                    // Remove the mana that was added
                    if let Ok(player) = self.get_player_mut(player_id) {
                        // Subtract each color that was added
                        if player.mana_pool.white >= mana.white {
                            player.mana_pool.white -= mana.white;
                        }
                        if player.mana_pool.blue >= mana.blue {
                            player.mana_pool.blue -= mana.blue;
                        }
                        if player.mana_pool.black >= mana.black {
                            player.mana_pool.black -= mana.black;
                        }
                        if player.mana_pool.red >= mana.red {
                            player.mana_pool.red -= mana.red;
                        }
                        if player.mana_pool.green >= mana.green {
                            player.mana_pool.green -= mana.green;
                        }
                        if player.mana_pool.colorless >= mana.colorless {
                            player.mana_pool.colorless -= mana.colorless;
                        }
                    }
                }
                crate::undo::GameAction::EmptyManaPool {
                    player_id,
                    prev_white,
                    prev_blue,
                    prev_black,
                    prev_red,
                    prev_green,
                    prev_colorless,
                } => {
                    // Restore previous mana pool state
                    if let Ok(player) = self.get_player_mut(player_id) {
                        player.mana_pool.white = prev_white;
                        player.mana_pool.blue = prev_blue;
                        player.mana_pool.black = prev_black;
                        player.mana_pool.red = prev_red;
                        player.mana_pool.green = prev_green;
                        player.mana_pool.colorless = prev_colorless;
                    }
                }
                crate::undo::GameAction::AddCounter {
                    card_id,
                    counter_type,
                    amount,
                } => {
                    // Remove the counters that were added
                    if let Ok(card) = self.cards.get_mut(card_id) {
                        card.remove_counter(counter_type, amount);
                    }
                }
                crate::undo::GameAction::RemoveCounter {
                    card_id,
                    counter_type,
                    amount,
                } => {
                    // Add back the counters that were removed
                    if let Ok(card) = self.cards.get_mut(card_id) {
                        card.add_counter(counter_type, amount);
                    }
                }
                crate::undo::GameAction::AdvanceStep { from_step, to_step: _ } => {
                    // Revert to previous step
                    self.turn.current_step = from_step;
                }
                crate::undo::GameAction::ChangeTurn {
                    from_player,
                    to_player: _,
                    turn_number,
                    rng_state,
                } => {
                    // Revert to previous turn
                    self.turn.active_player = from_player;
                    self.turn.turn_number = turn_number - 1;

                    // Restore RNG state if available (using bincode + SmallVec)
                    if let Some(rng_bytes) = rng_state {
                        // SmallVec derefs to &[u8], which is what bincode::deserialize expects
                        if let Ok(rng) = bincode::deserialize::<ChaCha12Rng>(&rng_bytes) {
                            *self.rng.borrow_mut() = rng;
                        }
                    }

                    // Note: We don't reset lands_played here as that state
                    // should be managed by separate actions if needed
                }
                crate::undo::GameAction::PumpCreature {
                    card_id,
                    power_delta,
                    toughness_delta,
                } => {
                    // Reverse the pump effect
                    if let Ok(card) = self.cards.get_mut(card_id) {
                        card.power_bonus -= power_delta;
                        card.toughness_bonus -= toughness_delta;
                    }
                }
                crate::undo::GameAction::SetTurnEnteredBattlefield {
                    card_id,
                    old_value,
                    new_value: _,
                } => {
                    // Restore the previous turn_entered_battlefield value
                    if let Ok(card) = self.cards.get_mut(card_id) {
                        card.turn_entered_battlefield = old_value;
                    }
                }
                crate::undo::GameAction::SetLandsPlayedThisTurn {
                    player_id,
                    old_value,
                    new_value: _,
                } => {
                    // Restore the previous lands_played_this_turn count
                    if let Ok(player) = self.get_player_mut(player_id) {
                        player.lands_played_this_turn = old_value;
                    }
                }
                crate::undo::GameAction::SetAttachedTo {
                    equipment_id,
                    old_target,
                    new_target: _,
                } => {
                    // Restore the previous attached_to value
                    if let Ok(equipment) = self.cards.get_mut(equipment_id) {
                        equipment.attached_to = old_target;
                    }
                }
                crate::undo::GameAction::ChoicePoint { .. } => {
                    // Choice points don't need to be undone
                }
                crate::undo::GameAction::HiddenDraw { player_id } => {
                    // Undo hidden draw: decrement hand's hidden_card_count and increment library size
                    // This is only used in client shadow states, never on server
                    //
                    // When draw happened: library.decrement_size(), hand.increment_hidden_card_count()
                    // To undo: hand.decrement_hidden_card_count(), library.increment_size()
                    if let Some(zones) = self.get_player_zones_mut(player_id) {
                        zones.hand.decrement_hidden_card_count();
                        if let Some(ref mut mode) = zones.library.library_mode {
                            mode.increment_size();
                        }
                    }
                }

                crate::undo::GameAction::HiddenDiscard { player_id } => {
                    // Undo hidden discard: increment hand's hidden_card_count and decrement graveyard
                    // This is only used in client shadow states, never on server
                    //
                    // When discard happened: hand.decrement_hidden_card_count(), graveyard.increment_hidden_card_count()
                    // To undo: hand.increment_hidden_card_count(), graveyard.decrement_hidden_card_count()
                    if let Some(zones) = self.get_player_zones_mut(player_id) {
                        zones.hand.increment_hidden_card_count();
                        zones.graveyard.decrement_hidden_card_count();
                    }
                }

                crate::undo::GameAction::RevealCard { card_id, card, .. } => {
                    // Undo reveal: clear the card from EntityStore (unreveal it)
                    // Only clear if we actually revealed a card (card was Some)
                    if card.is_some() {
                        self.cards.clear(card_id);
                    }
                    // If card was None, this was a dummy reveal (opponent perspective)
                    // and nothing needs to be undone
                }
            }

            // After undo, mark all mana caches as needing rebuild
            // (Lazy rebuild on next query - cheaper than incrementally reversing events)
            for (_, cache) in &mut self.mana_caches {
                cache.mark_dirty();
            }

            Ok(Some(prior_log_size))
        } else {
            Ok(None)
        }
    }

    /// Check and fire delayed triggers for a zone change.
    ///
    /// Called after a card moves between zones to fire any delayed triggers
    /// that were waiting for this event (e.g., Earthbend's return-to-battlefield).
    ///
    /// Returns the number of triggers that fired.
    pub fn check_delayed_triggers_on_zone_change(
        &mut self,
        card_id: CardId,
        from_zone: Zone,
        to_zone: Zone,
    ) -> Result<usize> {
        // Find all triggers that match this zone change
        let trigger_ids = self
            .delayed_triggers
            .find_zone_change_triggers(card_id, from_zone, to_zone);

        if trigger_ids.is_empty() {
            return Ok(0);
        }

        log::debug!(
            target: "delayed_triggers",
            "Found {} delayed triggers for card {} moving from {:?} to {:?}",
            trigger_ids.len(),
            card_id.as_u32(),
            from_zone,
            to_zone
        );

        let mut fired_count = 0;

        // Fire each trigger (remove it and execute its effect)
        for trigger_id in trigger_ids {
            if let Some(trigger) = self.delayed_triggers.remove(trigger_id) {
                self.fire_delayed_trigger(trigger)?;
                fired_count += 1;
            }
        }

        Ok(fired_count)
    }

    /// Fire a delayed trigger, executing its effect.
    fn fire_delayed_trigger(&mut self, trigger: crate::core::DelayedTrigger) -> Result<()> {
        use crate::core::DelayedEffect;

        let card_id = trigger.tracked_card;
        let controller = trigger.controller;

        match trigger.effect {
            DelayedEffect::ReturnToBattlefield { tapped, to_owner } => {
                // Get the card's current zone and owner
                let (current_zone, card_owner) = {
                    if let Ok(card) = self.cards.get(card_id) {
                        let owner = card.owner;
                        // Find where the card currently is
                        let zone = self.find_card_zone(card_id).unwrap_or(Zone::Graveyard);
                        (zone, owner)
                    } else {
                        return Ok(()); // Card no longer exists
                    }
                };

                // Determine who controls the returned card
                let return_controller = if to_owner { card_owner } else { controller };

                // Get card name for logging
                let card_name = self
                    .cards
                    .get(card_id)
                    .map(|c| c.name.to_string())
                    .unwrap_or_else(|_| "Unknown".to_string());

                // Move card to battlefield
                self.move_card(card_id, current_zone, Zone::Battlefield, return_controller)?;

                // Tap if required
                if tapped {
                    if let Ok(card) = self.cards.get_mut(card_id) {
                        card.tapped = true;
                    }
                }

                self.logger.normal(&format!(
                    "{} returns to the battlefield{}",
                    card_name,
                    if tapped { " tapped" } else { "" }
                ));
            }

            DelayedEffect::Sacrifice => {
                // Find where the card is and sacrifice it
                if let Some(zone) = self.find_card_zone(card_id) {
                    if zone == Zone::Battlefield {
                        let owner = self.cards.get(card_id).map(|c| c.owner).unwrap_or(controller);
                        self.move_card(card_id, Zone::Battlefield, Zone::Graveyard, owner)?;
                    }
                }
            }

            DelayedEffect::ExileCard => {
                // Exile the card from wherever it is
                if let Some(zone) = self.find_card_zone(card_id) {
                    let owner = self.cards.get(card_id).map(|c| c.owner).unwrap_or(controller);
                    self.move_card(card_id, zone, Zone::Exile, owner)?;
                }
            }

            DelayedEffect::CastWithoutPaying => {
                // TODO: Implement for Suspend mechanic
                // This requires putting the spell on the stack without paying costs
                log::warn!(target: "delayed_triggers", "CastWithoutPaying not yet implemented");
            }
        }

        Ok(())
    }

    /// Find which zone a card is currently in.
    fn find_card_zone(&self, card_id: CardId) -> Option<Zone> {
        // Check battlefield
        if self.battlefield.cards.contains(&card_id) {
            return Some(Zone::Battlefield);
        }

        // Check stack
        if self.stack.cards.contains(&card_id) {
            return Some(Zone::Stack);
        }

        // Check player zones
        for (_, zones) in &self.player_zones {
            if zones.hand.cards.contains(&card_id) {
                return Some(Zone::Hand);
            }
            if zones.library.cards.contains(&card_id) {
                return Some(Zone::Library);
            }
            if zones.graveyard.cards.contains(&card_id) {
                return Some(Zone::Graveyard);
            }
            if zones.exile.cards.contains(&card_id) {
                return Some(Zone::Exile);
            }
        }

        None
    }
}

impl Clone for GameState {
    fn clone(&self) -> Self {
        // Clone mana caches and mark them as needing rebuild
        // This is cheaper than rebuilding immediately (lazy rebuild on first query)
        let mana_caches_cloned: Vec<(PlayerId, ManaSourceCache)> = self
            .mana_caches
            .iter()
            .map(|(id, cache)| {
                let mut cloned = cache.clone();
                cloned.mark_dirty(); // Lazy rebuild on next query
                (*id, cloned)
            })
            .collect();

        GameState {
            cards: self.cards.clone(),
            players: self.players.clone(),
            player_zones: self.player_zones.clone(),
            mana_caches: mana_caches_cloned,
            battlefield: self.battlefield.clone(),
            stack: self.stack.clone(),
            turn: self.turn.clone(),
            combat: self.combat.clone(),
            rng: self.rng.clone(),
            next_entity_id: self.next_entity_id,
            undo_log: self.undo_log.clone(),
            logger: self.logger.clone(),
            token_definitions: self.token_definitions.clone(),
            // Each clone gets a fresh empty bump allocator
            bump: Bump::new(),
            mana_state_version: self.mana_state_version,
            persistent_effects: self.persistent_effects.clone(),
            delayed_triggers: self.delayed_triggers.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::Step;

    #[test]
    fn test_game_creation() {
        let game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);

        assert_eq!(game.players.len(), 2);
        assert_eq!(game.player_zones.len(), 2);
        assert_eq!(game.turn.turn_number, 1);
        assert_eq!(game.turn.current_step, Step::Untap);
    }

    #[test]
    fn test_bump_allocator_vec() {
        let game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);

        // Test that we can allocate a Vec from the game's bump allocator
        let mut v: Vec<i32, &Bump> = Vec::new_in(&game.bump);
        v.push(1);
        v.push(2);
        v.push(3);
        assert_eq!(v.len(), 3);
        assert_eq!(v[0], 1);
        assert_eq!(v[2], 3);

        // Test with_capacity_in
        let mut v2: Vec<u64, &Bump> = Vec::with_capacity_in(10, &game.bump);
        v2.push(42);
        assert_eq!(v2.capacity(), 10);
        assert_eq!(v2[0], 42);

        // Verify bump allocated bytes increased
        assert!(game.bump.allocated_bytes() > 0);
    }

    #[test]
    fn test_draw_card() {
        let mut game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);

        // Create a card and add it to library
        let p1_id = game.players.first().unwrap().id; // Copy the ID
        let card_id = game.next_entity_id();
        let card = Card::new(card_id, "Test Card".to_string(), p1_id);
        game.cards.insert(card_id, card);

        // Add to library
        if let Some(zones) = game.get_player_zones_mut(p1_id) {
            zones.library.add(card_id);
        }

        // Draw the card
        let drawn = game.draw_card(p1_id).unwrap();
        assert_eq!(drawn, Some(card_id));

        // Check it's in hand
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert!(zones.hand.contains(card_id));
            assert!(!zones.library.contains(card_id));
        }
    }

    #[test]
    fn test_game_over() {
        let mut game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);

        assert!(!game.is_game_over());
        assert_eq!(game.get_winner(), None);

        // Make player 1 lose
        let p1_id = game.players.first().unwrap().id; // Copy the ID
        if let Ok(player) = game.get_player_mut(p1_id) {
            player.lose_life(20);
        }

        assert!(game.is_game_over());
        let winner = game.get_winner().unwrap();
        assert_ne!(winner, p1_id);
    }

    #[test]
    fn test_undo_log_integration() {
        use crate::core::CardType;

        let mut game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
        let p1_id = game.players.first().unwrap().id;

        assert_eq!(game.undo_log.len(), 0);

        // Create and play a land
        let card_id = game.next_card_id();
        let mut card = Card::new(card_id, "Mountain", p1_id);
        card.add_type(CardType::Land);
        game.cards.insert(card_id, card);

        if let Some(zones) = game.get_player_zones_mut(p1_id) {
            zones.hand.add(card_id);
        }

        // Play the land - should log MoveCard, SetTurnEnteredBattlefield, SetLandsPlayedThisTurn
        game.play_land(p1_id, card_id).unwrap();
        assert_eq!(game.undo_log.len(), 3);
        matches!(
            game.undo_log.peek().unwrap(),
            crate::undo::GameAction::SetLandsPlayedThisTurn { .. }
        );

        // Tap for mana - should log TapCard and AddMana
        game.tap_for_mana(p1_id, card_id).unwrap();
        assert_eq!(game.undo_log.len(), 5); // MoveCard, SetTurnEnteredBattlefield, SetLandsPlayedThisTurn, TapCard, AddMana

        // Untap all - should log TapCard for untap
        game.untap_all(p1_id).unwrap();
        assert_eq!(game.undo_log.len(), 6); // + TapCard (untapped)

        // Verify all actions are logged
        let actions = game.undo_log.actions();
        assert!(matches!(actions[0], crate::undo::GameAction::MoveCard { .. }));
        assert!(matches!(
            actions[1],
            crate::undo::GameAction::SetTurnEnteredBattlefield { .. }
        ));
        assert!(matches!(
            actions[2],
            crate::undo::GameAction::SetLandsPlayedThisTurn { .. }
        ));
        assert!(matches!(
            actions[3],
            crate::undo::GameAction::TapCard { tapped: true, .. }
        ));
        assert!(matches!(actions[4], crate::undo::GameAction::AddMana { .. }));
        assert!(matches!(
            actions[5],
            crate::undo::GameAction::TapCard { tapped: false, .. }
        ));
    }
}
