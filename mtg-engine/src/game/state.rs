//! Main game state structure

use crate::core::{
    Card, CardId, CardName, Color, DelayedTriggerStore, EntityId, EntityStore, PersistentEffectStore, Player, PlayerId,
};
use crate::game::{CombatState, GameLogger, ManaSourceCache, TurnStructure};
use crate::undo::{GameAction, UndoLog};
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

    /// Card definitions for all cards in this game (server only)
    /// Maps card name to its definition for network transmission.
    /// Wrapped in Arc so that cloning GameState only bumps refcount.
    /// Not serialized - rebuilt during game initialization.
    #[serde(skip)]
    pub card_definitions: std::sync::Arc<std::collections::HashMap<CardName, crate::loader::CardDefinition>>,

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

    /// Skip reveal action generation for local (non-networked) games.
    ///
    /// When `true` (default), `maybe_reveal_to_player()` and `maybe_reveal_to_all()`
    /// are no-ops, avoiding the overhead of reveal tracking and undo logging.
    ///
    /// When `false`, reveal actions are generated for network synchronization.
    /// This also enables testing that local simulations match network behavior.
    ///
    /// Note: Action logs will differ between networked and local games when
    /// `skip_reveals=true`. Set to `false` for deterministic replay comparison.
    pub skip_reveals: bool,

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

    /// Remembered cards for ImmediateTrigger effects.
    ///
    /// Temporary storage used during ability resolution chains to pass card
    /// references between effects. For example:
    /// - DB$ Discard with RememberDiscarded$ True stores discarded cards here
    /// - DB$ ImmediateTrigger checks conditions against remembered cards
    /// - DB$ Cleanup clears this storage
    ///
    /// This mirrors Java Forge's "remembered" mechanism for tracking cards
    /// across linked effects in a resolution chain.
    #[serde(default)]
    pub remembered_cards: smallvec::SmallVec<[CardId; 4]>,

    /// Remembered players for conditional effects.
    ///
    /// Temporary storage used during ability resolution chains to pass player
    /// references between effects. For example:
    /// - DB$ Discard with RememberDiscardingPlayers$ True stores discarding player IDs here
    /// - DB$ Draw with Defined$ Remembered draws for each remembered player
    /// - DB$ Cleanup clears this storage
    #[serde(default)]
    pub remembered_players: smallvec::SmallVec<[PlayerId; 4]>,

    /// Queue of extra turns granted by effects (e.g., Time Walk).
    ///
    /// Each entry is the PlayerId who gets the extra turn.
    /// Extra turns are consumed FIFO: the first-granted extra turn happens first.
    /// At end of a turn, if extra_turns is non-empty, the front player gets
    /// the next turn instead of the normal alternation.
    #[serde(default)]
    pub extra_turns: std::collections::VecDeque<PlayerId>,

    /// Number of extra combat phases remaining this turn.
    ///
    /// When a card adds extra combat phases (e.g., Raphael Tag Team Tough),
    /// this counter is incremented. At end of the first combat's EndCombat step,
    /// if extra_combat_phases > 0, the turn goes back to BeginCombat instead of Main2.
    #[serde(default)]
    pub extra_combat_phases: u8,

    /// Indicates this is a network client shadow game state.
    ///
    /// When `true`, zone tracking for opponent's hidden zones (library, hand)
    /// may be incomplete. The game should be tolerant of zone operations
    /// failing due to missing cards - the server is authoritative.
    ///
    /// This allows move_card to succeed even when a card isn't found in the
    /// source zone, which happens when:
    /// - Opponent's library cards are milled/exiled (client doesn't know contents)
    /// - Cards are cast from exile (reveal timing issues)
    #[serde(default)]
    pub is_shadow_game: bool,

    /// Whether this is a Commander format game.
    ///
    /// When `true`, commander-specific rules apply:
    /// - Starting life is 40
    /// - Commanders start in the command zone
    /// - Commander tax applies when casting from command zone
    /// - Commander damage tracking is active (21+ = loss)
    /// - When a commander would go to graveyard/exile, owner may return to command zone
    #[serde(default)]
    pub is_commander_game: bool,

    /// Pending typecycling library search (WASM game loop resumption).
    ///
    /// When the WASM game loop is interrupted (NeedInput) during the library
    /// search phase of typecycling, this records the search in progress.
    /// On the next game loop invocation, `priority_round()` checks this flag
    /// and resumes the library search directly instead of asking the controller
    /// for a spell ability choice (which would misroute the queued LibrarySearchByName
    /// OpponentChoice to `choose_spell_ability_to_play`).
    ///
    /// Not serialized — this is transient game loop state that only persists
    /// across WASM step_harness() invocations within the same session.
    #[serde(skip)]
    pub pending_cycling_search: Option<(PlayerId, crate::core::Subtype)>,

    /// Pending spell cast (WASM game loop resumption).
    ///
    /// When the WASM game loop is interrupted (NeedInput) during mode selection
    /// or target selection of a spell cast, this records the card being cast.
    /// On the next game loop invocation, `priority_round()` checks this flag
    /// and resumes the cast from where it was interrupted, bypassing
    /// `choose_spell_ability_to_play` which would misroute the queued mode or
    /// target ChoiceRequest to the spell ability choice handler.
    ///
    /// Not serialized — transient game loop state for WASM resumption only.
    #[serde(skip)]
    pub pending_cast: Option<(PlayerId, CardId)>,

    /// Pending activated ability (WASM game loop resumption).
    ///
    /// When the WASM game loop is interrupted (NeedInput) during target selection
    /// of an activated ability, this records the ability being executed.
    /// On the next game loop invocation, `priority_round()` checks this flag
    /// and resumes from where it was interrupted, bypassing
    /// `choose_spell_ability_to_play` which would misroute the queued target
    /// ChoiceRequest to the spell ability choice handler.
    ///
    /// Stores (player_id, card_id, ability_index).
    ///
    /// Not serialized — transient game loop state for WASM resumption only.
    #[serde(skip)]
    pub pending_activation: Option<(PlayerId, CardId, usize)>,

    /// Effect resume index for interrupted activated ability execution (WASM).
    ///
    /// When an activated ability's effects loop is interrupted by NeedInput
    /// (e.g., DiscardCards routing needing a ChoiceRequest), this stores the
    /// index of the effect to resume from. On re-entry, costs are NOT re-paid
    /// and effects before this index are skipped.
    ///
    /// Also stores the chosen targets from the first entry, since target
    /// selection happens before cost payment and effects.
    ///
    /// Not serialized — transient game loop state for WASM resumption only.
    #[serde(skip)]
    pub pending_activation_effect_idx: Option<(usize, Vec<CardId>)>,

    /// Targets chosen for spells currently on the stack (spell_id -> chosen_targets).
    ///
    /// Targets are selected at CAST time but consumed at RESOLUTION time. In the WASM
    /// harness, `GameLoop` is recreated on every `step_harness()` call. Storing targets
    /// here (in `GameState`) ensures they survive across step_harness() invocations —
    /// e.g., when a priority_round returns NeedInput between the cast step_harness and
    /// the resolve step_harness.
    ///
    /// Without this persistence, spells would "resolve" with no targets and fizzle,
    /// causing state divergence (opponent's permanents not being destroyed/moved/etc).
    ///
    /// Entries are removed when the corresponding spell resolves or is countered.
    /// Uses SmallVec for targets since most spells have 0-2 targets.
    ///
    /// Not serialized — transient game loop state for WASM resumption only.
    #[serde(skip)]
    pub spell_targets: Vec<(CardId, smallvec::SmallVec<[CardId; 2]>)>,

    /// Player IDs whose library order changed via a hidden-info-dependent
    /// operation (scry/surveil) and therefore need to be resynchronised to
    /// network clients via a `LibraryReordered` message.
    ///
    /// On the SERVER (`is_shadow_game == false`), `scry_apply_decision` /
    /// `surveil_apply_decision` append the scrying player's id whenever the
    /// caller-supplied decision actually moves cards. The `NetworkController`
    /// drains this list when building the next `ChoiceRequest` and the
    /// coordinator broadcasts `LibraryReordered` to both clients before the
    /// request is forwarded.
    ///
    /// On the CLIENT (`is_shadow_game == true`), the controller pipeline
    /// applies the server-authoritative decision verbatim, so the resulting
    /// library order matches the server by construction; no client-side
    /// broadcast is needed and this list is left empty.
    ///
    /// Not serialized — purely a transient network-sync side channel.
    /// `RefCell` is required because `NetworkController` only sees an immutable
    /// `GameStateView`, but must drain the queue to assemble each `ChoiceRequest`.
    /// (Same pattern as `rng`.)
    #[serde(skip)]
    pub pending_library_reorders: std::cell::RefCell<Vec<PlayerId>>,
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
            card_definitions: std::sync::Arc::new(std::collections::HashMap::new()),
            bump: Bump::new(),
            mana_state_version: 0,
            skip_reveals: true, // Default: skip reveals for local games
            persistent_effects: PersistentEffectStore::new(),
            delayed_triggers: DelayedTriggerStore::new(),
            remembered_cards: smallvec::SmallVec::new(),
            remembered_players: smallvec::SmallVec::new(),
            extra_turns: std::collections::VecDeque::new(),
            extra_combat_phases: 0,
            is_shadow_game: false, // Default: not a shadow game
            is_commander_game: false,
            pending_cycling_search: None,
            pending_cast: None,
            pending_activation: None,
            pending_activation_effect_idx: None,
            spell_targets: Vec::new(),
            pending_library_reorders: std::cell::RefCell::new(Vec::new()),
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

    /// Enable or disable reveal action generation.
    ///
    /// - `true` (default): Skip reveal actions for local games (no overhead)
    /// - `false`: Generate reveal actions for networked games
    ///
    /// Call `set_skip_reveals(false)` for network games or when you want
    /// action logs to match network simulation for testing.
    #[inline]
    pub fn set_skip_reveals(&mut self, skip: bool) {
        self.skip_reveals = skip;
    }

    /// Mark this game state as a shadow game (network client).
    ///
    /// When `true`, the game is tolerant of zone operations failing due to
    /// incomplete zone tracking. This is necessary because:
    /// - Opponent's library contents are hidden
    /// - Reveal timing may cause cards to not be in expected zones
    ///
    /// The server is authoritative - shadow game state is approximate.
    #[inline]
    pub fn set_shadow_game(&mut self, is_shadow: bool) {
        self.is_shadow_game = is_shadow;
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
    /// # Errors
    ///
    /// Returns an error if the card doesn't exist.
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

    /// Apply a regeneration shield: consume one shield, tap, clear damage,
    /// remove from combat. CR 701.15a.
    ///
    /// # Errors
    ///
    /// Returns an error if the card doesn't exist.
    pub fn apply_regeneration_shield(&mut self, card_id: CardId) -> Result<()> {
        let card_name = {
            let card = self.cards.get_mut(card_id)?;
            // Consume one shield
            card.regeneration_shields = card.regeneration_shields.saturating_sub(1);
            // Remove all damage
            card.damage = 0;
            card.name.clone()
        };
        // Tap the creature (needs &mut self so can't hold card borrow)
        self.tap_permanent(card_id)?;

        // Remove from combat if attacking or blocking (CR 701.15a)
        self.combat.remove_from_combat(card_id);

        self.logger.gamelog(&format!(
            "{} ({}) regenerates (shield consumed, tapped, damage removed)",
            card_name, card_id
        ));

        Ok(())
    }

    /// Untap a permanent and log the action for undo
    ///
    /// This is the preferred way to untap permanents - it handles:
    /// - Setting the untapped state
    /// - Logging the undo action
    /// - Incrementing the mana state version for cache invalidation
    ///
    /// # Errors
    ///
    /// Returns an error if the card doesn't exist.
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
    /// This logs a ShuffleLibrary action to the undo log with the previous
    /// order, enabling proper undo for tutor effects and game tree search.
    ///
    /// ## Network Considerations
    ///
    /// After calling this, the server should send a LibraryReordered message
    /// to clients so their shadow states stay synchronized.
    pub fn shuffle_library(&mut self, player_id: PlayerId) {
        use rand::seq::SliceRandom;

        // Get the library and store previous order before shuffling
        if let Some(zones) = self
            .player_zones
            .iter_mut()
            .find(|(id, _)| *id == player_id)
            .map(|(_, z)| z)
        {
            // Capture the previous order for undo
            let previous_order = zones.library.cards.clone();

            // DEBUG: Log RNG state hash before shuffle (mtg-232 investigation)
            let rng_hash_before = {
                let rng = self.rng.borrow();
                let serialized = bincode::serialize(&*rng).unwrap_or_default();
                // Simple hash of serialized bytes
                serialized
                    .iter()
                    .fold(0u64, |acc, &b| acc.wrapping_mul(31).wrapping_add(u64::from(b)))
            };
            log::info!(
                "[SHUFFLE-DEBUG] Before shuffle: player={:?}, rng_hash={:016x}, lib_len={}, first_5_cards={:?}",
                player_id,
                rng_hash_before,
                zones.library.cards.len(),
                zones.library.cards.iter().rev().take(5).collect::<Vec<_>>()
            );

            // Perform the shuffle
            zones.library.cards.shuffle(&mut *self.rng.borrow_mut());

            // DEBUG: Log RNG state hash after shuffle (mtg-232 investigation)
            let rng_hash_after = {
                let rng = self.rng.borrow();
                let serialized = bincode::serialize(&*rng).unwrap_or_default();
                serialized
                    .iter()
                    .fold(0u64, |acc, &b| acc.wrapping_mul(31).wrapping_add(u64::from(b)))
            };
            log::info!(
                "[SHUFFLE-DEBUG] After shuffle: player={:?}, rng_hash={:016x}, first_5_cards={:?}",
                player_id,
                rng_hash_after,
                zones.library.cards.iter().rev().take(5).collect::<Vec<_>>()
            );

            // Log the action with the previous order and prior log size
            let prior_log_size = self.logger.log_count();
            self.undo_log.log(
                GameAction::ShuffleLibrary {
                    player: player_id,
                    previous_order,
                },
                prior_log_size,
            );
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

    /// Set the next entity ID counter
    ///
    /// Used by network clients in reserve-only mode to set the counter
    /// past the reserved CardID range so newly created entities (tokens)
    /// don't collide with reserved CardIDs.
    pub fn set_next_entity_id(&mut self, id: u32) {
        self.next_entity_id = id;
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

    /// Ensure that a per-player mana cache slot exists for the given player.
    ///
    /// This is needed after restoring a `GameState` from a snapshot, because
    /// `mana_caches` is `#[serde(skip)]` and therefore comes back as an empty
    /// `Vec` after deserialization. Without this initialization the next call
    /// to `ManaEngine::update`/`update_mut` would panic in
    /// `mana_engine.rs::update_mut` at `expect("Cache exists after rebuild")`.
    ///
    /// The cache is inserted in a "dirty" state so the next call lazily
    /// rebuilds it from the live battlefield.
    pub fn ensure_mana_cache(&mut self, player_id: PlayerId) {
        if self.mana_caches.iter().any(|(id, _)| *id == player_id) {
            return;
        }
        let mut cache = ManaSourceCache::new(player_id);
        cache.mark_dirty();
        self.mana_caches.push((player_id, cache));
    }

    /// Initialize missing per-player mana caches for ALL existing players.
    ///
    /// Convenience wrapper around `ensure_mana_cache` that walks the players
    /// list — useful immediately after deserializing a snapshot.
    pub fn ensure_mana_caches_for_all_players(&mut self) {
        let player_ids: Vec<PlayerId> = self.players.iter().map(|p| p.id).collect();
        for pid in player_ids {
            self.ensure_mana_cache(pid);
        }
    }

    /// Rebuild mana cache for a player if it needs rebuilding
    ///
    /// This is a helper for ManaEngine that handles borrow checker issues
    /// when calling rebuild_from_battlefield (which needs &mut cache and &GameState).
    pub fn rebuild_mana_cache_if_needed(&mut self, player_id: PlayerId) {
        // Make sure the cache slot exists. This guards against snapshot/restore
        // where `mana_caches` is `#[serde(skip)]` and therefore empty after
        // deserialization (see `ensure_mana_cache`).
        self.ensure_mana_cache(player_id);

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
    ///
    /// # Errors
    ///
    /// Returns an error if the player ID does not exist.
    pub fn get_player(&self, id: PlayerId) -> Result<&Player> {
        self.players
            .iter()
            .find(|p| p.id == id)
            .ok_or(crate::MtgError::EntityNotFound(id.as_u32()))
    }

    /// Human-readable display name for a player, falling back to
    /// `"Player N"` (1-based) when the player can't be looked up. Centralizes
    /// the name-or-fallback formatting used by target logging and the
    /// player-target sentinel display path.
    pub fn player_display_name(&self, id: PlayerId) -> String {
        self.get_player(id)
            .ok()
            .map(|p| p.name.to_string())
            .unwrap_or_else(|| format!("Player {}", id.as_u32() + 1))
    }

    /// Get a player by ID (returns Option, no error allocation)
    ///
    /// OPTIMIZATION: This returns `Option<&Player>` instead of `Result<&Player, MtgError>`.
    /// Use this on hot paths where you would otherwise call `.unwrap_or_default()` or `.ok()`
    /// since it avoids constructing MtgError on the failure path.
    #[inline]
    pub fn try_get_player(&self, id: PlayerId) -> Option<&Player> {
        self.players.iter().find(|p| p.id == id)
    }

    /// Get a mutable player by ID
    ///
    /// # Errors
    ///
    /// Returns an error if the player ID does not exist.
    pub fn get_player_mut(&mut self, id: PlayerId) -> Result<&mut Player> {
        self.players
            .iter_mut()
            .find(|p| p.id == id)
            .ok_or(crate::MtgError::EntityNotFound(id.as_u32()))
    }

    /// Get a mutable player by ID (returns Option, no error allocation)
    ///
    /// OPTIMIZATION: This returns `Option<&mut Player>` instead of `Result<&mut Player, MtgError>`.
    /// Use this on hot paths where you would otherwise call `.unwrap_or_default()` or `.ok()`
    /// since it avoids constructing MtgError on the failure path.
    #[inline]
    pub fn try_get_player_mut(&mut self, id: PlayerId) -> Option<&mut Player> {
        self.players.iter_mut().find(|p| p.id == id)
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

    // ========================================================================
    // Reveal helpers
    //
    // These functions handle the "should I reveal / emit reveal" logic for
    // card visibility in networked games.
    //
    // Two cases:
    // 1. Card exists in store (normal) - just update mask, log SetRevealedToMask
    // 2. Card doesn't exist (late-binding) - log RevealCard to create it
    // ========================================================================

    /// Maybe reveal a card to a specific player.
    ///
    /// Used when a card becomes visible to one player only (e.g., drawing a card).
    /// If the card already exists and isn't revealed to the player, logs SetRevealedToMask.
    /// If the card doesn't exist (late-binding), logs RevealCard.
    ///
    /// No-op if `skip_reveals` is true (default for local games).
    #[inline]
    pub fn maybe_reveal_to_player(&mut self, card_id: CardId, player_id: PlayerId, prior_log_size: usize) {
        if self.skip_reveals {
            return;
        }
        if let Some(card) = self.cards.try_get_mut(card_id) {
            // Card exists - check if needs reveal to this player
            if !card.is_revealed_to(player_id) {
                let old_mask = card.revealed_to_mask;
                let card_name = card.name.to_string();
                card.mark_revealed_to(player_id);
                // Log RevealCard with name (for network) and old_mask (for undo)
                self.undo_log.log(
                    crate::undo::GameAction::RevealCard {
                        card_id,
                        name: Some(card_name),
                        revealed_to: crate::undo::RevealTarget::Player(player_id),
                        old_mask,
                    },
                    prior_log_size,
                );
            }
        } else {
            // Card doesn't exist - late-binding reveal (client doesn't know name yet)
            self.undo_log.log(
                crate::undo::GameAction::RevealCard {
                    card_id,
                    name: None,
                    revealed_to: crate::undo::RevealTarget::Player(player_id),
                    old_mask: 0,
                },
                prior_log_size,
            );
        }
    }

    /// Maybe reveal a card to all players.
    ///
    /// Used when a card becomes publicly visible (e.g., entering battlefield, stack, graveyard).
    /// Logs RevealCard with name and old_mask for network and undo support.
    ///
    /// No-op if `skip_reveals` is true (default for local games).
    #[inline]
    pub fn maybe_reveal_to_all(&mut self, card_id: CardId, prior_log_size: usize) {
        if self.skip_reveals {
            return;
        }
        if let Some(card) = self.cards.try_get_mut(card_id) {
            // Card exists - check if needs reveal to all
            if !card.is_revealed_to_all() {
                let old_mask = card.revealed_to_mask;
                let card_name = card.name.to_string();
                card.mark_revealed_to_all();
                // Log RevealCard with name (for network) and old_mask (for undo)
                self.undo_log.log(
                    crate::undo::GameAction::RevealCard {
                        card_id,
                        name: Some(card_name),
                        revealed_to: crate::undo::RevealTarget::All,
                        old_mask,
                    },
                    prior_log_size,
                );
            }
        } else {
            // Card doesn't exist - late-binding reveal (client doesn't know name yet)
            self.undo_log.log(
                crate::undo::GameAction::RevealCard {
                    card_id,
                    name: None,
                    revealed_to: crate::undo::RevealTarget::All,
                    old_mask: 0,
                },
                prior_log_size,
            );
        }
    }

    /// Move a card from one zone to another
    ///
    /// # Errors
    ///
    /// Returns an error if zone operations fail.
    pub fn move_card(&mut self, card_id: CardId, from: Zone, mut to: Zone, owner: PlayerId) -> Result<()> {
        // Commander zone-change replacement (MTG CR 903.9a):
        // If a commander would be put into its owner's graveyard or exile from anywhere,
        // its owner may put it into the command zone instead.
        // For now, this is automatic (always returns to command zone).
        // TODO(mtg-274): Add player choice for commander zone replacement
        if self.is_commander_game && (to == Zone::Graveyard || to == Zone::Exile) {
            if let Some(card) = self.cards.try_get(card_id) {
                if card.is_commander {
                    self.logger.normal(&format!(
                        "{} returns to the command zone (commander replacement)",
                        card.name
                    ));
                    to = Zone::Command;
                }
            }
        }

        // NETWORK: Auto-reveal cards transitioning from hidden to public zones.
        // This ensures the server logs RevealCard actions for all zone moves,
        // preventing desync when clients don't know the card's identity.
        // Idempotent: callers that already revealed before calling move_card()
        // will hit the is_revealed check and skip (no duplicate log entries).
        if !self.skip_reveals {
            let prior_log_size = self.logger.log_count();
            match (from, to) {
                // Hidden → Public: reveal to all players
                (
                    Zone::Library | Zone::Hand,
                    Zone::Battlefield | Zone::Stack | Zone::Graveyard | Zone::Exile | Zone::Command,
                ) => {
                    self.maybe_reveal_to_all(card_id, prior_log_size);
                }
                // Library → Hand: reveal to owner only (hand is private)
                (Zone::Library, Zone::Hand) => {
                    self.maybe_reveal_to_player(card_id, owner, prior_log_size);
                }
                _ => {} // Public→Public, Public→Hidden: no reveal needed
            }
        }

        // Debug log card movement
        if let Some(card) = self.cards.try_get(card_id) {
            log::debug!(target: "zone", "Moving card {} (id={}) from {:?} to {:?} (owner: player {})",
                card.name, card_id.as_u32(), from, to, owner.as_u32());
        } else {
            log::debug!(target: "zone", "Moving unknown card (id={}) from {:?} to {:?} (owner: player {})",
                card_id.as_u32(), from, to, owner.as_u32());
        }

        // State-based action: If a creature is leaving the battlefield, detach all Equipment from it
        if from == Zone::Battlefield {
            if let Some(card) = self.cards.try_get(card_id) {
                if card.is_creature() {
                    // Collect Equipment to detach (to avoid borrow issues)
                    let equipment_to_detach: Vec<CardId> = self
                        .battlefield
                        .cards
                        .iter()
                        .filter_map(|&equip_id| {
                            let equip = self.cards.try_get(equip_id)?;
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

        // Capture the card's position in `from` BEFORE removing it, so
        // `GameAction::MoveCard` can record where to reinsert on undo.
        // Without this, undo re-appends to the end of the source zone,
        // silently permuting Hand/Library order across rewind/replay
        // cycles (root cause of the WASM verifier's hand-reorder drift).
        let from_position: Option<u32> = match from {
            Zone::Battlefield => self.battlefield.position_of(card_id).map(|p| p as u32),
            Zone::Stack => self.stack.position_of(card_id).map(|p| p as u32),
            Zone::Library | Zone::Hand | Zone::Graveyard | Zone::Exile | Zone::Command => self
                .get_player_zones(owner)
                .and_then(|z| z.get_zone(from))
                .and_then(|z| z.position_of(card_id))
                .map(|p| p as u32),
        };

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
            if self.is_shadow_game {
                // In shadow game mode, be tolerant of missing cards in source zones.
                // This happens for opponent's hidden zones (library) where we don't
                // have complete tracking. Return early like the server path — adding
                // the card to the destination without removing it from the source
                // would cause zone count divergence (e.g., extra graveyard entries).
                log::debug!(
                    target: "zone",
                    "Shadow game: Card {} not found in source zone {:?}, skipping move (server authoritative)",
                    card_id,
                    from
                );
                return Ok(());
            } else {
                // Card not in source zone - this can happen when:
                // 1. A trigger moved the card before SBA could process it
                // 2. Multiple effects target the same card (first move succeeds, second fizzles)
                // Log warning and return Ok to avoid crashing the game
                log::warn!(
                    target: "zone",
                    "Card {} not found in source zone {:?} - likely already moved by another effect",
                    card_id, from
                );
                return Ok(());
            }
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
            if let Some(card) = self.cards.try_get(card_id) {
                if card.definition.cache.enters_tapped {
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
            if let Some(card) = self.cards.try_get(card_id) {
                if card.definition.cache.etb_choose_color {
                    let exclude_colors = card.definition.cache.etb_exclude_colors.clone();
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
                from_position,
            },
            prior_log_size,
        );

        // Log significant zone transitions to the gamelog
        // This ensures visibility into all card movements for debugging and replay
        if let Some(card) = self.cards.try_get(card_id) {
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
    pub(crate) fn pick_prominent_color(&self, player_id: PlayerId, exclude: &[Color]) -> Color {
        use std::collections::BTreeMap;

        // BTreeMap provides deterministic iteration order by Color discriminant (WUBRG),
        // which is required for network determinism when breaking ties.
        let mut color_counts: BTreeMap<Color, u32> = BTreeMap::new();

        // Count colors from cards in hand, library, and graveyard
        let zones_to_check = if let Some(zones) = self.get_player_zones(player_id) {
            vec![&zones.hand, &zones.library, &zones.graveyard]
        } else {
            vec![]
        };

        for zone in zones_to_check {
            for &card_id in zone.cards.iter() {
                if let Some(card) = self.cards.try_get(card_id) {
                    // Count mana symbols in the mana cost
                    if card.mana_cost.white > 0 {
                        *color_counts.entry(Color::White).or_insert(0) += u32::from(card.mana_cost.white);
                    }
                    if card.mana_cost.blue > 0 {
                        *color_counts.entry(Color::Blue).or_insert(0) += u32::from(card.mana_cost.blue);
                    }
                    if card.mana_cost.black > 0 {
                        *color_counts.entry(Color::Black).or_insert(0) += u32::from(card.mana_cost.black);
                    }
                    if card.mana_cost.red > 0 {
                        *color_counts.entry(Color::Red).or_insert(0) += u32::from(card.mana_cost.red);
                    }
                    if card.mana_cost.green > 0 {
                        *color_counts.entry(Color::Green).or_insert(0) += u32::from(card.mana_cost.green);
                    }
                }
            }
        }

        // Remove excluded colors
        for color in exclude {
            color_counts.remove(color);
        }

        // Return the most prominent color, or a default if none found.
        // BTreeMap iteration is deterministic (WUBRG order); ties are broken
        // by Color discriminant via the then_with comparator below.
        color_counts
            .into_iter()
            .max_by(|(color_a, count_a), (color_b, count_b)| {
                count_a
                    .cmp(count_b)
                    .then_with(|| (*color_b as u8).cmp(&(*color_a as u8)))
            })
            .map(|(color, _)| color)
            .unwrap_or_else(|| {
                // Default: pick first non-excluded color
                Color::all_colors()
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
    ///
    /// In the late-binding architecture, all CardIDs are known upfront (library
    /// contains actual CardIds). Card identities are revealed via RevealCard
    /// BEFORE the move, per NETWORK_ARCHITECTURE.md.
    ///
    /// When drawing:
    /// 1. RevealCard logged to reveal the card's identity to the drawing player
    /// 2. MoveCard logged to move from Library to Hand
    ///
    /// # Errors
    ///
    /// Returns an error if drawing fails (should not happen for normal draw operations).
    /// Draw a card for a player
    ///
    /// Returns (Option<CardId>, draw_count) where draw_count is how many cards
    /// this player has drawn this turn (1 = first, 2 = second, etc.).
    /// This is used for "second card drawn" triggers like T:Mode$ Drawn.
    ///
    /// Emits a `"P draws CARD (id)"` gamelog entry on success. Use
    /// [`Self::draw_card_silent`] for setup paths (opening hands) where the
    /// per-card log noise is not desired.
    pub fn draw_card(&mut self, player_id: PlayerId) -> Result<(Option<CardId>, u8)> {
        self.draw_card_inner(player_id, /* log_gamelog = */ true)
    }

    /// Draw a card without emitting a gamelog entry.
    ///
    /// Used for opening hand setup (MTG Rules 103.4) where per-card draw
    /// messages would clutter the game log before the game has even started.
    /// All other code paths should call [`Self::draw_card`] so the draw is
    /// surfaced to the user.
    ///
    /// # Errors
    ///
    /// Returns the underlying [`Result`] from `draw_card_inner` — surfaces any
    /// engine error encountered while moving the top library card to the
    /// player's hand (e.g. an exhausted library is signalled separately via
    /// the returned `Option<CardId>` being `None`, not via `Err`).
    pub fn draw_card_silent(&mut self, player_id: PlayerId) -> Result<(Option<CardId>, u8)> {
        self.draw_card_inner(player_id, /* log_gamelog = */ false)
    }

    /// Internal helper backing both `draw_card` and `draw_card_silent`.
    ///
    /// Centralising the per-card "P draws CARD (id)" gamelog here ensures
    /// that abilities such as Bazaar of Baghdad
    /// ("Draw two cards, then discard three cards.") emit draw events in
    /// their natural order (draw before discard) instead of silently
    /// performing the draws — see regression test
    /// `test_draw_cards_effect_emits_per_card_gamelog` and bug-bazaar-no-draw.
    fn draw_card_inner(&mut self, player_id: PlayerId, log_gamelog: bool) -> Result<(Option<CardId>, u8)> {
        if let Some(zones) = self.get_player_zones_mut(player_id) {
            let lib_size = zones.library.len();
            log::debug!(
                "draw_card: player {} library cards_len={}",
                player_id.as_u32(),
                lib_size
            );

            // Try to draw from the library
            if let Some(card_id) = zones.library.draw_top() {
                // The card was at the top of the library; after `draw_top`
                // pops it, the new library length equals the slot index it
                // used to occupy. Capture this so undo can restore the card
                // to the same position on the library stack.
                let from_position = Some(zones.library.cards.len() as u32);
                zones.hand.add(card_id);

                let prior_log_size = self.logger.log_count();

                // Reveal to drawing player before logging movement
                // (drawing reveals to owner only since hand is hidden)
                self.maybe_reveal_to_player(card_id, player_id, prior_log_size);

                // Log the card movement for undo
                self.undo_log.log(
                    crate::undo::GameAction::MoveCard {
                        card_id,
                        from_zone: crate::zones::Zone::Library,
                        to_zone: crate::zones::Zone::Hand,
                        owner: player_id,
                        from_position,
                    },
                    prior_log_size,
                );

                // Record the draw for "second card drawn" triggers
                // This must be done after zones are released to avoid borrow conflicts
                let draw_count = if let Ok(player) = self.get_player_mut(player_id) {
                    let old_count = player.cards_drawn_this_turn;
                    let count = player.record_card_drawn();
                    log::debug!("Player {} drew card (draw #{} this turn)", player_id.as_u32(), count);

                    // Log for undo - use the prior_log_size from above (still in scope)
                    self.undo_log.log(
                        crate::undo::GameAction::SetCardsDrawnThisTurn {
                            player_id,
                            old_value: old_count,
                            new_value: count,
                        },
                        prior_log_size,
                    );

                    count
                } else {
                    1 // Fallback, shouldn't happen
                };

                if log_gamelog {
                    let player_name = self
                        .get_player(player_id)
                        .map(|p| p.name.to_string())
                        .unwrap_or_else(|_| format!("Player {}", player_id.as_u32() + 1));
                    let card_name = self
                        .cards
                        .try_get(card_id)
                        .map(|c| c.name.to_string())
                        .unwrap_or_else(|| "a card".to_string());
                    // Per-card draw lines reveal hidden information (the
                    // identity of a card the drawing player just put into
                    // their hand). Mark the entry as private to `player_id`
                    // so the WASM/web UIs can mask it as "P draws a card"
                    // when rendering from another player's perspective.
                    // Closes bug-draw-reveals-opponent-hand.
                    self.logger.gamelog_private(
                        &format!("{} draws {} ({})", player_name, card_name, card_id),
                        player_id,
                        &format!("{} draws a card", player_name),
                    );
                }

                return Ok((Some(card_id), draw_count));
            }
        }
        Ok((None, 0))
    }

    /// Mill cards from library to graveyard (used by mill effects)
    ///
    /// Returns SmallVec to avoid heap allocation for typical mill counts (up to 8 cards).
    /// Mill effects typically mill 1-7 cards (e.g., "mill 3 cards").
    ///
    /// Per NETWORK_ARCHITECTURE.md, cards are revealed to ALL players before moving
    /// to graveyard (which is a public zone).
    ///
    /// # Errors
    ///
    /// Returns an error if zone operations fail.
    pub fn mill_cards(&mut self, player_id: PlayerId, count: u8) -> Result<SmallVec<[CardId; 8]>> {
        let mut milled_cards: SmallVec<[CardId; 8]> = SmallVec::new();

        for _ in 0..count {
            // Try to get top card from library
            let card_id = if let Some(zones) = self.get_player_zones(player_id) {
                zones.library.cards.last().copied()
            } else {
                None
            };

            if let Some(card_id) = card_id {
                // Move the card from library to graveyard (move_card auto-reveals + logs MoveCard)
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

    /// Snapshot the top N cards of `player_id`'s library WITHOUT mutating
    /// anything. Returns the cards top-down (`result[0]` is the current top
    /// of the library). Returns an empty vec if the player has no zones or
    /// an empty library.
    ///
    /// This is the "look at the top N cards" half of scry/surveil; the
    /// caller then asks the controller for a decision and feeds the result
    /// to [`scry_apply_decision`] (or [`surveil_apply_decision`]).
    pub fn scry_snapshot_top_n(&self, player_id: PlayerId, count: u8) -> SmallVec<[CardId; 4]> {
        let Some(zones) = self.get_player_zones(player_id) else {
            return SmallVec::new();
        };
        zones
            .library
            .cards
            .iter()
            .rev() // Top of library is last in vec
            .take(count as usize)
            .copied()
            .collect()
    }

    /// Apply a scry decision to the library, after the controller has chosen
    /// how to partition the revealed cards.
    ///
    /// `revealed` MUST be the same slice produced by
    /// [`scry_snapshot_top_n`]; together they describe the cards the
    /// controller saw. `decision.top` and `decision.bottom` together must
    /// be a partition of `revealed`. Both decision vectors are stored
    /// **bottom-up** so they can be applied directly via `library.cards.push()`
    /// (see [`crate::game::ScryDecision`] for the convention).
    ///
    /// Emits the same logger messages the previous engine-baked
    /// implementation did so existing snapshots / e2e logs are unchanged.
    ///
    /// # Errors
    ///
    /// Returns `Ok(())` even when `revealed` is empty (nothing to do) or
    /// when `player_id`'s zones are missing (player has lost the game but
    /// triggered effects are still resolving). The result type is
    /// reserved for future failure modes (e.g. structured logging /
    /// undo-log allocation) so all callers thread `?` uniformly.
    pub fn scry_apply_decision(
        &mut self,
        player_id: PlayerId,
        revealed: &[CardId],
        decision: &crate::game::ScryDecision,
    ) -> Result<()> {
        if revealed.is_empty() {
            return Ok(());
        }

        // Log the scry action FIRST so message ordering matches the
        // pre-refactor implementation exactly.
        //
        // NETWORK SYNC NOTE (post-merge of fix-cycle-desync + fix-scry-choice-pipeline):
        // The previous shadow-game guard that bailed out when top cards
        // were unrevealed has been removed. Under the Phase A-E controller
        // pipeline, the SERVER's controller produces the decision and the
        // CLIENT's NetworkController receives it via the ChoiceRequest
        // payload (revealed CardIds inline + indices-into-revealed in the
        // response), so both sides apply identical, server-authoritative
        // decisions to the library — there's no client-side heuristic
        // left to diverge. The `pending_library_reorders` broadcast below
        // is kept as a defense-in-depth signal for cases where the
        // controller pipeline isn't engaged (e.g. legacy effect paths).
        let card_name = self.cards.try_get(revealed[0]).map(|c| c.name.clone());

        if revealed.len() == 1 {
            let name = card_name.as_ref().map(|n| n.as_str()).unwrap_or("Unknown");
            if decision.bottom.is_empty() {
                self.logger
                    .normal(&format!("P{} scries 1, keeps {} on top", player_id.as_u32() + 1, name));
            } else {
                self.logger.normal(&format!(
                    "P{} scries 1, puts {} on bottom",
                    player_id.as_u32() + 1,
                    name
                ));
            }
        } else {
            self.logger.normal(&format!(
                "P{} scries {}, keeps {} on top, puts {} on bottom",
                player_id.as_u32() + 1,
                revealed.len(),
                decision.top.len(),
                decision.bottom.len()
            ));
        }

        let library_actually_changed = !decision.bottom.is_empty();

        if let Some(zones) = self.get_player_zones_mut(player_id) {
            // Remove all revealed cards from library (they were on top).
            for card_id in revealed {
                zones.library.remove(*card_id);
            }

            // Put "bottom" cards at the absolute bottom. `decision.bottom`
            // is bottom-up, so iterating in reverse and inserting at index
            // 0 leaves the first element at the deepest position.
            for card_id in decision.bottom.iter().rev() {
                zones.library.cards.insert(0, *card_id);
            }

            // Put "top" cards back. `decision.top` is bottom-up, so the
            // last element of `decision.top` ends up on top of the library
            // (matching the meaning documented on ScryDecision).
            for &card_id in decision.top.iter() {
                zones.library.cards.push(card_id);
            }
        }

        // NETWORK SYNC: If we're the server (`!is_shadow_game`) running in
        // network mode (`!skip_reveals`) AND our heuristic actually moved at
        // least one card, signal that the scrying player's library order must
        // be broadcast to clients. The `NetworkController` drains this queue
        // when assembling the next `ChoiceRequest`. Local games and pure
        // client-side runs leave the queue alone.
        if library_actually_changed && !self.skip_reveals && !self.is_shadow_game {
            self.pending_library_reorders.borrow_mut().push(player_id);
        }

        Ok(())
    }

    /// Snapshot the top N cards of `player_id`'s library WITHOUT mutating
    /// anything. Returns the cards top-down (`result[0]` is the current
    /// top of the library). Returns an empty vec if the player has no
    /// zones or an empty library.
    pub fn surveil_snapshot_top_n(&self, player_id: PlayerId, count: u8) -> SmallVec<[CardId; 4]> {
        let Some(zones) = self.get_player_zones(player_id) else {
            return SmallVec::new();
        };
        zones.library.cards.iter().rev().take(count as usize).copied().collect()
    }

    /// Apply a surveil decision to the library and graveyard, after the
    /// controller has chosen how to partition the revealed cards.
    ///
    /// `revealed` MUST be the same slice produced by
    /// [`surveil_snapshot_top_n`]. `decision.top` and `decision.graveyard`
    /// together must be a partition of `revealed`. `decision.top` is
    /// stored bottom-up (last element ends up on top of library);
    /// `decision.graveyard` is stored in placement order (first element
    /// ends up deepest in the graveyard pile).
    ///
    /// # Errors
    ///
    /// Returns `Ok(())` even when `revealed` is empty (nothing to do) or
    /// when `player_id`'s zones are missing. The result type is reserved
    /// for future failure modes so all callers thread `?` uniformly.
    pub fn surveil_apply_decision(
        &mut self,
        player_id: PlayerId,
        revealed: &[CardId],
        decision: &crate::game::SurveilDecision,
    ) -> Result<()> {
        if revealed.is_empty() {
            return Ok(());
        }

        // Log the surveil action FIRST so message ordering matches the
        // pre-refactor implementation exactly. See scry_apply_decision for
        // the merged-cycle-desync-vs-scry-pipeline rationale (no shadow-game
        // guard needed under the controller pipeline).
        self.logger.gamelog(&format!(
            "P{} surveils {}, keeps {} on top, puts {} into graveyard",
            player_id.as_u32() + 1,
            revealed.len(),
            decision.top.len(),
            decision.graveyard.len()
        ));

        let library_actually_changed = !decision.graveyard.is_empty();

        // Move graveyard cards first (in the order chosen — first element
        // ends up deepest in the graveyard pile, matching push semantics).
        for card_id in &decision.graveyard {
            if let Some(zones) = self.get_player_zones_mut(player_id) {
                zones.library.remove(*card_id);
            }
            if let Some(zones) = self.get_player_zones_mut(player_id) {
                zones.graveyard.cards.push(*card_id);
            }
        }

        // Rearrange remaining cards: remove from current positions, put
        // back on top in the order chosen. `decision.top` is bottom-up
        // (last element ends up on top of library).
        if let Some(zones) = self.get_player_zones_mut(player_id) {
            for card_id in &decision.top {
                zones.library.remove(*card_id);
            }
            for &card_id in &decision.top {
                zones.library.cards.push(card_id);
            }
        }

        // NETWORK SYNC: see scry_apply_decision for rationale (mtg-420).
        if library_actually_changed && !self.skip_reveals && !self.is_shadow_game {
            self.pending_library_reorders.borrow_mut().push(player_id);
        }

        Ok(())
    }

    /// Counter a spell on the stack
    ///
    /// This removes the spell from the stack and moves it to its owner's graveyard.
    ///
    /// # Errors
    ///
    /// Returns an error if the spell is not on the stack or if the card cannot be found.
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

        // Capture stack position BEFORE removal so undo can restore the
        // countered spell to the same slot on the stack (matters for
        // top-of-stack semantics during rewind/replay).
        let from_position = self.stack.position_of(spell_id).map(|p| p as u32);

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
                from_position,
            },
            prior_log_size,
        );

        Ok(())
    }

    /// Untap all permanents controlled by a player
    ///
    /// # Errors
    ///
    /// Returns an error if card access fails (should not happen for normal operations).
    pub fn untap_all(&mut self, player_id: PlayerId) -> Result<()> {
        // Collect first so we can call untap_permanent (which borrows &mut self).
        let to_untap: SmallVec<[CardId; 16]> = self
            .battlefield
            .cards
            .iter()
            .copied()
            .filter(|&card_id| {
                self.cards
                    .try_get(card_id)
                    .map(|c| c.controller == player_id && c.tapped)
                    .unwrap_or(false)
            })
            .collect();
        for card_id in to_untap {
            // Route through untap_permanent so the undo log, ManaSourceCache
            // untapped counts, and mana_state_version all stay consistent. The
            // previous inline `card.untap()` left the mana cache stale.
            self.untap_permanent(card_id)?;
        }
        Ok(())
    }

    /// Add counters to a card and log for undo
    ///
    /// # Errors
    ///
    /// Returns an error if the card cannot be found.
    pub fn add_counters(&mut self, card_id: CardId, counter_type: crate::core::CounterType, amount: u8) -> Result<()> {
        if let Some(card) = self.cards.try_get_mut(card_id) {
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
    ///
    /// # Errors
    ///
    /// Returns an error if the card cannot be found.
    pub fn remove_counters(
        &mut self,
        card_id: CardId,
        counter_type: crate::core::CounterType,
        amount: u8,
    ) -> Result<u8> {
        if let Some(card) = self.cards.try_get_mut(card_id) {
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

    /// Determine the destination zone when a creature dies.
    ///
    /// If the creature has a finality counter, it goes to exile instead of graveyard
    /// (MTG CR 122.1c: "If a permanent with a finality counter on it would die, exile it instead.")
    pub fn death_destination_for_card(&self, card_id: CardId) -> Zone {
        if let Ok(card) = self.cards.get(card_id) {
            if card.get_counter(crate::core::CounterType::Finality) > 0 {
                return Zone::Exile;
            }
        }
        Zone::Graveyard
    }

    /// Check state-based actions for lethal damage (MTG CR 704.5g)
    ///
    /// If a creature has damage marked on it greater than or equal to its toughness,
    /// and it doesn't have indestructible, that creature's controller puts it into the graveyard.
    ///
    /// This should be called after damage is dealt or whenever state-based actions are checked.
    ///
    /// # Errors
    ///
    /// Returns an error if zone operations fail.
    pub fn check_lethal_damage(&mut self) -> Result<()> {
        // Collect creature IDs from battlefield first (to avoid borrow checker issues
        // with self.get_effective_toughness() needing &self while iterating battlefield)
        let creature_ids: smallvec::SmallVec<[CardId; 16]> = self
            .battlefield
            .cards
            .iter()
            .copied()
            .filter(|&card_id| self.cards.try_get(card_id).is_some_and(|card| card.is_creature()))
            .collect();

        // Now check each creature for lethal damage / zero toughness using effective P/T
        let creatures_to_destroy: smallvec::SmallVec<[(CardId, PlayerId); 4]> = creature_ids
            .iter()
            .filter_map(|&card_id| {
                let card = self.cards.try_get(card_id)?;

                // MTG CR 704.5f: Creature with toughness 0 or less dies (independent of damage)
                // MTG CR 704.5g: Creature has lethal damage if damage >= toughness
                // Use effective toughness (includes equipment, anthems, counters via layer system)
                let effective_toughness = self
                    .get_effective_toughness(card_id)
                    .unwrap_or_else(|_| i32::from(card.current_toughness()));
                let has_zero_toughness = effective_toughness <= 0;
                let has_lethal_damage = card.damage >= effective_toughness;

                // Debug: Log SBA check for creatures with damage or low toughness
                if log::log_enabled!(target: "sba", log::Level::Debug)
                    && (card.damage > 0 || effective_toughness <= 0)
                {
                    log::debug!(target: "sba", "SBA check: {} (id={}) damage={} effective_toughness={} zero_toughness={} lethal_damage={} indestructible={}",
                        card.name, card_id.as_u32(), card.damage, effective_toughness, has_zero_toughness, has_lethal_damage, card.has_indestructible());
                }

                // MTG CR 704.5f: Zero or less toughness → dies (even if indestructible!)
                // Note: Indestructible does NOT prevent death from 0 toughness (CR 702.12b only prevents destruction)
                if has_zero_toughness {
                    return Some((card_id, card.owner));
                }

                // MTG CR 702.12b: Indestructible permanents aren't destroyed by lethal damage
                if has_lethal_damage && !card.has_indestructible() {
                    Some((card_id, card.owner))
                } else {
                    None
                }
            })
            .collect();

        // Destroy all creatures with lethal damage
        for (card_id, owner) in creatures_to_destroy {
            let card_name = self.cards.try_get(card_id).map(|c| c.name.clone());
            let dest = self.death_destination_for_card(card_id);

            // Check death triggers BEFORE moving to graveyard (MTG Rules 603.6c)
            // The trigger sees the game state as it was just before the creature left
            let _ = self.check_death_triggers(card_id);

            self.move_card(card_id, Zone::Battlefield, dest, owner)?;
            if let Some(name) = card_name {
                if dest == Zone::Exile {
                    self.logger.gamelog(&format!(
                        "{} ({}) exiled from lethal damage (finality counter)",
                        name, card_id
                    ));
                } else {
                    self.logger
                        .gamelog(&format!("{} ({}) dies from lethal damage", name, card_id));
                }
            }
        }

        Ok(())
    }

    /// Check state-based actions for legendary rule (MTG CR 704.5j)
    ///
    /// If a player controls two or more legendary permanents with the same name,
    /// that player chooses one of them, and the rest are put into their owners' graveyards.
    /// This is a "legend rule" that prevents duplicate legendary permanents.
    ///
    /// Note: This is a simplified implementation that keeps the first one found.
    /// A proper implementation would let the player choose which to keep.
    ///
    /// # Errors
    ///
    /// Returns an error if a card cannot be found or moved to graveyard.
    pub fn check_legendary_rule(&mut self) -> Result<()> {
        use crate::core::CardName;
        use std::collections::BTreeMap;

        // Group legendary permanents by (controller, name)
        // BTreeMap provides deterministic iteration order by (PlayerId, CardName),
        // which is required for network determinism.
        let mut legendary_groups: BTreeMap<(PlayerId, CardName), Vec<CardId>> = BTreeMap::new();

        for &card_id in &self.battlefield.cards {
            if let Some(card) = self.cards.try_get(card_id) {
                if card.is_legendary {
                    let key = (card.controller, card.name.clone());
                    legendary_groups.entry(key).or_default().push(card_id);
                }
            }
        }

        // Find duplicates and sacrifice all but the first one
        // Store: (card_id_to_sacrifice, owner, name, kept_card_id)
        let mut cards_to_sacrifice: Vec<(CardId, PlayerId, CardName, CardId)> = Vec::new();

        for ((_controller, name), cards) in legendary_groups {
            if cards.len() > 1 {
                // Keep the first one (index 0), sacrifice the rest
                // TODO: Let player choose which one to keep
                let kept_card = cards[0];
                for &card_id in &cards[1..] {
                    if let Some(card) = self.cards.try_get(card_id) {
                        cards_to_sacrifice.push((card_id, card.owner, name.clone(), kept_card));
                    }
                }
            }
        }

        // Move duplicates to graveyard (legend rule)
        for (card_id, owner, name, kept_card) in cards_to_sacrifice {
            log::debug!(target: "sba", "Legendary rule: {} ({}) sacrificed as duplicate (keeping {})",
                name, card_id.as_u32(), kept_card.as_u32());
            let dest = self.death_destination_for_card(card_id);
            self.move_card(card_id, Zone::Battlefield, dest, owner)?;
            self.logger.gamelog(&format!(
                "{} ({}) sacrificed due to legendary rule (duplicate of {} ({}))",
                name, card_id, name, kept_card
            ));
        }

        Ok(())
    }

    /// Check state-based actions for aura attachment (MTG CR 704.5d)
    ///
    /// If an Aura is attached to an illegal permanent or not attached to anything,
    /// that Aura's controller puts it into the graveyard.
    ///
    /// # Errors
    ///
    /// Returns an error if zone operations fail.
    pub fn check_aura_attachment(&mut self) -> Result<()> {
        // OPTIMIZATION: Collect only CardId+PlayerId (avoid Arc<str> clone for card name).
        // Use SmallVec to avoid heap allocation when no auras are orphaned (common case).
        let auras_to_destroy: smallvec::SmallVec<[(CardId, PlayerId); 2]> = self
            .battlefield
            .cards
            .iter()
            .filter_map(|&card_id| {
                let card = self.cards.try_get(card_id)?;
                if !card.is_aura() {
                    return None;
                }

                // Check if aura is attached to something
                let attached_to = card.get_attached_to()?;

                // Check if the attached target still exists on battlefield
                if !self.battlefield.cards.contains(&attached_to) {
                    return Some((card_id, card.owner));
                }

                None
            })
            .collect();

        // Move orphaned auras to graveyard (or exile if finality counter)
        for (card_id, owner) in auras_to_destroy {
            let card_name = self
                .cards
                .try_get(card_id)
                .map(|c| c.name.to_string())
                .unwrap_or_else(|| "Unknown".to_string());
            let dest = self.death_destination_for_card(card_id);
            self.move_card(card_id, Zone::Battlefield, dest, owner)?;
            self.logger.gamelog(&format!(
                "{} ({}) goes to graveyard (aura not attached to valid permanent)",
                card_name, card_id
            ));
        }

        Ok(())
    }

    /// Re-derive control of permanents from control-changing Auras (CR 613.2, layer 2).
    ///
    /// Control-stealing Auras (Control Magic, Mind Control, Persuasion, Enslave, ...)
    /// carry a `S:Mode$ Continuous | Affected$ Card.EnchantedBy | GainControl$ You`
    /// static ([`StaticAbility::GainControl`]). Control is a *continuous* effect, so
    /// rather than mutating the host's controller at attach time and undoing it on
    /// detach, we recompute it from scratch every state-based-action pass:
    ///
    ///   desired_controller(permanent) =
    ///       controller of the most-recently-attached control Aura on it,
    ///       else the permanent's owner.
    ///
    /// This makes control self-correcting: when the Aura leaves the battlefield
    /// (destroyed by Disenchant, bounced, or the host dies and the Aura falls off),
    /// the Aura no longer contributes and control reverts to the owner on the very
    /// next SBA check — exactly the CR 613 behaviour, with no special-case cleanup.
    ///
    /// Note: an effect that *grants* control via a one-shot `AB$ GainControl`
    /// (Threaten/Act of Treason) is a separate mechanism — it mutates `controller`
    /// directly and is not re-derived here. Those effects do not enchant a permanent,
    /// so they never produce a control Aura and are unaffected by this pass.
    ///
    /// # Errors
    ///
    /// Returns an error if a card lookup fails.
    pub fn recompute_aura_control(&mut self) -> Result<()> {
        use crate::core::StaticAbility;

        // Step 1: collect (host_id -> (aura_id, new_controller)) for every permanent
        // currently enchanted by a control-granting Aura. Later auras in battlefield
        // order overwrite earlier ones, approximating CR 613.7 timestamp order (the
        // rare multi-control case).
        let mut control_overrides: smallvec::SmallVec<[(CardId, CardId, PlayerId); 2]> = smallvec::SmallVec::new();
        for &aura_id in &self.battlefield.cards {
            let Some(aura) = self.cards.try_get(aura_id) else {
                continue;
            };
            if !aura.is_aura() {
                continue;
            }
            let Some(host_id) = aura.get_attached_to() else {
                continue;
            };
            let grants_control = aura
                .static_abilities
                .iter()
                .any(|a| matches!(a, StaticAbility::GainControl { .. }));
            if !grants_control {
                continue;
            }
            // Only meaningful while the host is still on the battlefield.
            if !self.battlefield.cards.contains(&host_id) {
                continue;
            }
            let new_controller = aura.controller;
            if let Some(slot) = control_overrides.iter_mut().find(|(h, _, _)| *h == host_id) {
                slot.1 = aura_id;
                slot.2 = new_controller;
            } else {
                control_overrides.push((host_id, aura_id, new_controller));
            }
        }

        // Step 2: apply changes. We ONLY touch permanents whose control is, or should
        // be, governed by a control Aura — identified by `control_from_aura`. This
        // deliberately leaves control gained by other means untouched (Animate Dead's
        // one-shot `GainControl$ True`, Threaten's `AB$ GainControl`), which set the
        // controller directly and never populate `control_from_aura`.
        for &card_id in &self.battlefield.cards.clone() {
            let Some(card) = self.cards.try_get(card_id) else {
                continue;
            };
            let owner = card.owner;
            let current = card.controller;
            let prior_aura = card.control_from_aura;
            let override_entry = control_overrides.iter().find(|(h, _, _)| *h == card_id);

            // Determine the desired controller + the aura responsible (if any).
            let (desired, new_aura): (PlayerId, Option<CardId>) = match override_entry {
                Some((_, aura_id, controller)) => (*controller, Some(*aura_id)),
                None => {
                    // No control Aura on this permanent now. If a control Aura USED to
                    // govern it (prior_aura set), the Aura is gone — revert to owner.
                    // Otherwise leave the permanent entirely alone.
                    if prior_aura.is_some() {
                        (owner, None)
                    } else {
                        continue;
                    }
                }
            };

            if current == desired && prior_aura == new_aura {
                continue;
            }

            let card_name = card.name.to_string();
            let prior_log_size = self.logger.log_count();
            let card_mut = self.cards.get_mut(card_id)?;
            card_mut.control_from_aura = new_aura;
            if current != desired {
                card_mut.controller = desired;
                self.undo_log.log(
                    crate::undo::GameAction::ChangeController {
                        card_id,
                        old_controller: current,
                        new_controller: desired,
                    },
                    prior_log_size,
                );
                let target_name = self
                    .get_player(desired)
                    .map(|p| p.name.to_string())
                    .unwrap_or_else(|_| format!("Player {}", desired.as_u32() + 1));
                if new_aura.is_none() {
                    self.logger
                        .gamelog(&format!("{} returns to {}'s control", card_name, target_name));
                } else {
                    self.logger
                        .gamelog(&format!("{} comes under {}'s control", card_name, target_name));
                }
            }
        }

        Ok(())
    }

    /// Check state-based actions for equipment attachment (MTG CR 704.5n)
    ///
    /// If an Equipment or Fortification is attached to an illegal permanent or
    /// became a creature, it becomes unattached.
    ///
    /// # Errors
    ///
    /// Returns an error if card operations fail.
    pub fn check_equipment_attachment(&mut self) -> Result<()> {
        // Collect equipment that needs to become unattached
        let equipment_to_unattach: Vec<CardId> = self
            .battlefield
            .cards
            .iter()
            .filter_map(|&card_id| {
                let card = self.cards.try_get(card_id)?;
                if !card.is_equipment() || !card.is_attached() {
                    return None;
                }

                // CR 704.5n: Equipment that became a creature becomes unattached
                if card.is_creature() {
                    log::debug!(target: "sba", "Equipment {} ({}) is now a creature, becoming unattached",
                        card.name, card_id.as_u32());
                    return Some(card_id);
                }

                // Check if attached target still exists on battlefield
                let attached_to = card.get_attached_to()?;
                if !self.battlefield.cards.contains(&attached_to) {
                    log::debug!(target: "sba", "Equipment {} ({}) attached to {} which is no longer on battlefield",
                        card.name, card_id.as_u32(), attached_to.as_u32());
                    return Some(card_id);
                }

                // Check if attached target is still a creature
                let target = self.cards.try_get(attached_to)?;
                if !target.is_creature() {
                    log::debug!(target: "sba", "Equipment {} ({}) attached to {} which is no longer a creature",
                        card.name, card_id.as_u32(), attached_to.as_u32());
                    return Some(card_id);
                }

                None
            })
            .collect();

        // Unattach equipment
        for card_id in equipment_to_unattach {
            if let Some(card) = self.cards.try_get_mut(card_id) {
                let name = card.name.clone();
                card.attached_to = None;
                self.logger
                    .gamelog(&format!("{} ({}) becomes unattached", name, card_id));
            }
        }

        Ok(())
    }

    /// Clear temporary effects at end of turn (Cleanup step)
    /// This resets power/toughness bonuses from pump spells and clears damage
    /// MTG CR 514.2: Damage marked on permanents is removed (CR 704.5f)
    pub fn cleanup_temporary_effects(&mut self) {
        // Track whether any animated mana source reverted, so we can
        // invalidate the per-player ManaSourceCache afterwards (Mishra's
        // Factory: complex source while animated, simple colorless source
        // again after cleanup — same desync as the activate path).
        let mut any_mana_source_typeline_reverted = false;

        for card_id in self.battlefield.cards.iter() {
            if let Some(card) = self.cards.try_get_mut(*card_id) {
                // Reset temporary bonuses (pump effects last until end of turn)
                card.power_bonus = 0;
                card.toughness_bonus = 0;
                // Reset temporary base P/T overrides (from Animate effects)
                card.clear_temp_base_stats();
                // Clear damage marked on permanents (MTG CR 514.2, CR 704.5f)
                card.damage = 0;

                // Roll back animation type changes (Mishra's Factory and
                // friends become land-only again at end of turn). We have to
                // refresh the cache flags so combat / mana / target logic
                // stops treating the manland as a creature.
                let touched_types = !card.temp_animate_types.is_empty();
                let touched_subtypes = !card.temp_animate_subtypes.is_empty() || !card.temp_removed_subtypes.is_empty();
                if touched_types {
                    let removed: smallvec::SmallVec<[crate::core::CardType; 2]> =
                        card.temp_animate_types.drain(..).collect();
                    card.types.retain(|t| !removed.contains(t));
                }
                if touched_subtypes {
                    let added: smallvec::SmallVec<[crate::core::Subtype; 2]> =
                        card.temp_animate_subtypes.drain(..).collect();
                    card.subtypes.retain(|s| !added.contains(s));
                    // Restore subtypes that RemoveCreatureTypes$ True stripped.
                    let restored: smallvec::SmallVec<[crate::core::Subtype; 2]> =
                        card.temp_removed_subtypes.drain(..).collect();
                    card.subtypes.extend(restored);
                }
                if touched_types || touched_subtypes {
                    let types = card.types.clone();
                    let subtypes = card.subtypes.clone();
                    let name = card.name.clone();
                    card.definition.cache.update_from_types(&types);
                    card.definition.cache.update_from_subtypes(&subtypes, name.as_str());
                    if card.definition.cache.is_mana_source {
                        any_mana_source_typeline_reverted = true;
                    }
                }
            }
        }

        if any_mana_source_typeline_reverted {
            for (_, cache) in &mut self.mana_caches {
                cache.mark_dirty();
            }
            self.increment_mana_version();
        }
    }

    /// Advance the game to the next step
    ///
    /// # Errors
    ///
    /// Returns an error if player lookup fails during turn transitions.
    pub fn advance_step(&mut self) -> Result<()> {
        let from_step = self.turn.current_step;

        // If entering cleanup step, clean up temporary effects
        if from_step == crate::game::Step::End && self.turn.current_step.next() == Some(crate::game::Step::Cleanup) {
            self.cleanup_temporary_effects();
        }

        // Handle extra combat phases: when leaving EndCombat and we have extras,
        // loop back to BeginCombat instead of advancing to Main2
        if from_step == crate::game::Step::EndCombat && self.extra_combat_phases > 0 {
            self.extra_combat_phases -= 1;
            self.turn.current_step = crate::game::Step::BeginCombat;
            // Reset combat-specific turn guards so combat steps work again
            self.turn.attackers_declared_turn = None;
            self.turn.blockers_declared_turn = None;
            self.turn.combat_first_strike_damage_dealt_turn = None;
            self.turn.combat_first_strike_priority_done_turn = None;
            self.turn.combat_damage_dealt_turn = None;
            self.logger.gamelog("Additional combat phase begins!");
            return Ok(());
        }

        // Reset extra_combat_phases at end of turn
        if from_step == crate::game::Step::Cleanup {
            self.extra_combat_phases = 0;
        }

        if !self.turn.advance_step() {
            // End of turn - check for extra turns before normal alternation
            // (CR 500.7 - Time Walk, etc.)
            let from_player = self.turn.active_player;
            let next_player = if let Some(extra_turn_player) = self.extra_turns.pop_front() {
                // Extra turn granted (e.g., Time Walk)
                self.logger.gamelog(&format!(
                    "Extra turn for {}!",
                    self.get_player(extra_turn_player)
                        .map(|p| p.name.as_str())
                        .unwrap_or("Unknown")
                ));
                extra_turn_player
            } else {
                // Normal turn alternation
                self.get_next_player(self.turn.active_player)?
            };
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

            // Log turn transfer indicator with life totals BEFORE logging ChangeTurn action.
            // This ensures that when we rewind to turn start and truncate the log,
            // the turn separator is preserved (since prior_log_size is captured AFTER it).
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

            // Log the turn change with RNG state from before the turn change.
            // Capture prior_log_size AFTER logging turn separator so rewind preserves it.
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
    ///
    /// # Errors
    ///
    /// Returns an error if undoing any game action fails (e.g., card not found, invalid zone).
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
                            from_position,
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
                                // Reinsert at original position when known so
                                // hand/library/stack order is preserved across
                                // undo (see `add_at` doc on `CardZone`).
                                let pos = from_position.map(|p| p as usize);
                                match from_zone {
                                    Zone::Battlefield => match pos {
                                        Some(p) => self.battlefield.add_at(card_id, p),
                                        None => self.battlefield.add(card_id),
                                    },
                                    Zone::Stack => match pos {
                                        Some(p) => self.stack.add_at(card_id, p),
                                        None => self.stack.add(card_id),
                                    },
                                    _ => {
                                        if let Some(zones) = self.get_player_zones_mut(owner) {
                                            if let Some(zone) = zones.get_zone_mut(from_zone) {
                                                match pos {
                                                    Some(p) => zone.add_at(card_id, p),
                                                    None => zone.add(card_id),
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        crate::undo::GameAction::TapCard { card_id, tapped } => {
                            if let Some(card) = self.cards.try_get_mut(card_id) {
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
                            if let Some(card) = self.cards.try_get_mut(card_id) {
                                card.remove_counter(counter_type, amount);
                            }
                        }
                        crate::undo::GameAction::RemoveCounter {
                            card_id,
                            counter_type,
                            amount,
                        } => {
                            if let Some(card) = self.cards.try_get_mut(card_id) {
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
                            keywords_granted,
                        } => {
                            if let Some(card) = self.cards.try_get_mut(card_id) {
                                card.power_bonus -= power_delta;
                                card.toughness_bonus -= toughness_delta;
                                // Remove granted keywords
                                for keyword in keywords_granted {
                                    card.keywords.remove(keyword);
                                }
                            }
                        }
                        crate::undo::GameAction::SetTurnEnteredBattlefield {
                            card_id,
                            old_value,
                            new_value: _,
                        } => {
                            if let Some(card) = self.cards.try_get_mut(card_id) {
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
                        crate::undo::GameAction::SetSpellsCastThisTurn {
                            player_id,
                            old_value,
                            new_value: _,
                        } => {
                            if let Ok(player) = self.get_player_mut(player_id) {
                                player.spells_cast_this_turn = old_value;
                            }
                        }
                        crate::undo::GameAction::ChangeController {
                            card_id,
                            old_controller,
                            new_controller: _,
                        } => {
                            if let Ok(card) = self.cards.get_mut(card_id) {
                                card.controller = old_controller;
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
                        crate::undo::GameAction::RevealCard {
                            card_id,
                            name,
                            old_mask,
                            ..
                        } => {
                            // Undo reveal: restore mask (matches undo.rs behavior)
                            // CRITICAL: Do NOT unconditionally clear card from EntityStore.
                            // In WASM network mode, cards instantiated by process_card_reveal_wasm
                            // are outside the undo log and must be preserved.
                            if let Some(card) = self.cards.try_get_mut(card_id) {
                                card.revealed_to_mask = old_mask;
                            } else if old_mask == 0 && name.is_some() {
                                self.cards.clear(card_id);
                            }
                        }
                        crate::undo::GameAction::SetRevealedToMask {
                            card_id,
                            old_value,
                            new_value: _,
                        } => {
                            if let Some(card) = self.cards.try_get_mut(card_id) {
                                card.revealed_to_mask = old_value;
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
    ///
    /// # Errors
    ///
    /// Returns an error if the action cannot be undone (e.g., card not found, invalid state).
    pub fn undo(&mut self) -> Result<Option<usize>> {
        if let Some((action, prior_log_size)) = self.undo_log.pop() {
            match action {
                crate::undo::GameAction::MoveCard {
                    card_id,
                    from_zone,
                    to_zone,
                    owner,
                    from_position,
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
                        // Reinsert at original position when known so
                        // hand/library/stack order is preserved on undo.
                        let pos = from_position.map(|p| p as usize);
                        match from_zone {
                            Zone::Battlefield => match pos {
                                Some(p) => self.battlefield.add_at(card_id, p),
                                None => self.battlefield.add(card_id),
                            },
                            Zone::Stack => match pos {
                                Some(p) => self.stack.add_at(card_id, p),
                                None => self.stack.add(card_id),
                            },
                            Zone::Library | Zone::Hand | Zone::Graveyard | Zone::Exile | Zone::Command => {
                                if let Some(zones) = self.get_player_zones_mut(owner) {
                                    if let Some(zone) = zones.get_zone_mut(from_zone) {
                                        match pos {
                                            Some(p) => zone.add_at(card_id, p),
                                            None => zone.add(card_id),
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                crate::undo::GameAction::TapCard { card_id, tapped } => {
                    // Reverse the tap state
                    if let Some(card) = self.cards.try_get_mut(card_id) {
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
                    if let Some(card) = self.cards.try_get_mut(card_id) {
                        card.remove_counter(counter_type, amount);
                    }
                }
                crate::undo::GameAction::RemoveCounter {
                    card_id,
                    counter_type,
                    amount,
                } => {
                    // Add back the counters that were removed
                    if let Some(card) = self.cards.try_get_mut(card_id) {
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
                    keywords_granted,
                } => {
                    // Reverse the pump effect
                    if let Some(card) = self.cards.try_get_mut(card_id) {
                        card.power_bonus -= power_delta;
                        card.toughness_bonus -= toughness_delta;
                        // Remove granted keywords
                        for keyword in keywords_granted {
                            card.keywords.remove(keyword);
                        }
                    }
                }
                crate::undo::GameAction::DebuffCreature {
                    card_id,
                    keywords_removed,
                } => {
                    // Reverse the debuff by re-adding removed keywords
                    if let Some(card) = self.cards.try_get_mut(card_id) {
                        for keyword in keywords_removed {
                            card.keywords.insert(keyword);
                        }
                    }
                }
                crate::undo::GameAction::SetTurnEnteredBattlefield {
                    card_id,
                    old_value,
                    new_value: _,
                } => {
                    // Restore the previous turn_entered_battlefield value
                    if let Some(card) = self.cards.try_get_mut(card_id) {
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
                crate::undo::GameAction::SetCardsDrawnThisTurn {
                    player_id,
                    old_value,
                    new_value: _,
                } => {
                    // Restore the previous cards_drawn_this_turn count
                    if let Ok(player) = self.get_player_mut(player_id) {
                        player.cards_drawn_this_turn = old_value;
                    }
                }
                crate::undo::GameAction::SetSpellsCastThisTurn {
                    player_id,
                    old_value,
                    new_value: _,
                } => {
                    // Restore the previous spells_cast_this_turn count
                    if let Ok(player) = self.get_player_mut(player_id) {
                        player.spells_cast_this_turn = old_value;
                    }
                }
                crate::undo::GameAction::ChangeController {
                    card_id,
                    old_controller,
                    new_controller: _,
                } => {
                    if let Ok(card) = self.cards.get_mut(card_id) {
                        card.controller = old_controller;
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

                crate::undo::GameAction::RevealCard {
                    card_id,
                    name,
                    old_mask,
                    ..
                } => {
                    // Undo reveal: restore previous mask state (matches undo.rs behavior)
                    // Two cases:
                    // 1. Card exists (server or client after instantiation):
                    //    Restore the old_mask value (card instance is preserved)
                    // 2. Card doesn't exist but was created by this reveal
                    //    (late-binding, old_mask=0, name=Some): Clear the slot
                    //
                    // CRITICAL: Do NOT unconditionally clear the card from EntityStore!
                    // In WASM network mode, cards are instantiated by process_card_reveal_wasm
                    // which is outside the undo log. Clearing the card would destroy the instance
                    // and cause FATAL DESYNC when abilities are recomputed (the card's type info
                    // would be lost, so PlayLand abilities wouldn't be generated for it).
                    if let Some(card) = self.cards.try_get_mut(card_id) {
                        // Card exists - restore the mask
                        card.revealed_to_mask = old_mask;
                    } else if old_mask == 0 && name.is_some() {
                        // Card doesn't exist but was created by this reveal
                        // This shouldn't normally happen since the card should exist
                        // if it was instantiated, but handle it defensively
                        self.cards.clear(card_id);
                    }
                    // If card doesn't exist and old_mask != 0, this is a late-binding
                    // reveal that never instantiated (opponent's hidden card) - nothing to undo
                }

                crate::undo::GameAction::SetRevealedToMask {
                    card_id,
                    old_value,
                    new_value: _,
                } => {
                    // Restore the previous revealed_to_mask value
                    if let Some(card) = self.cards.try_get_mut(card_id) {
                        card.revealed_to_mask = old_value;
                    }
                }

                crate::undo::GameAction::ShuffleLibrary { player, previous_order } => {
                    // Restore the library to its previous order
                    if let Some(zones) = self.get_player_zones_mut(player) {
                        zones.library.cards = previous_order;
                    }
                }

                crate::undo::GameAction::SetLoyaltyActivated {
                    card_id,
                    old_value,
                    new_value: _,
                } => {
                    if let Ok(card) = self.cards.get_mut(card_id) {
                        card.loyalty_activated_this_turn = old_value;
                    }
                }

                crate::undo::GameAction::SetCommanderCastCount {
                    player_id,
                    old_value,
                    new_value: _,
                } => {
                    if let Some(player) = self.players.iter_mut().find(|p| p.id == player_id) {
                        player.commander_cast_count = old_value;
                    }
                }

                crate::undo::GameAction::SetCommanderDamage {
                    player_id,
                    from_player,
                    old_damage,
                    new_damage: _,
                } => {
                    if let Some(player) = self.players.iter_mut().find(|p| p.id == player_id) {
                        if old_damage == 0 {
                            // Entry was newly added - remove it
                            player.commander_damage_taken.retain(|(pid, _)| *pid != from_player);
                        } else if let Some(entry) = player
                            .commander_damage_taken
                            .iter_mut()
                            .find(|(pid, _)| *pid == from_player)
                        {
                            entry.1 = old_damage;
                        }
                    }
                }
            }

            // After undo, mark all mana caches as needing rebuild
            // (Lazy rebuild on next query - cheaper than incrementally reversing events)
            for (_, cache) in &mut self.mana_caches {
                cache.mark_dirty();
            }

            // When fully rewound to the initial state, reset transient guard fields.
            // These fields are #[serde(skip)] (not in undo log) so they persist their
            // end-of-game values after rewind. This matters for rewind benchmarks where
            // the game is fully rewound and replayed in the same session: without this
            // reset, guards like draw_step_executed_turn = Some(N) would fire on turn N
            // in the replay, corrupting game state (e.g. skipping mandatory draw steps).
            if self.undo_log.is_empty() {
                self.turn.reset_transient_guards();
                self.pending_cast = None;
                self.pending_activation = None;
                self.pending_activation_effect_idx = None;
                self.pending_cycling_search = None;
                self.spell_targets.clear();
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
    ///
    /// # Errors
    ///
    /// Returns an error if executing a trigger effect fails.
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
    ///
    /// # Errors
    ///
    /// Returns an error if the card cannot be found or zone movement fails.
    pub fn fire_delayed_trigger(&mut self, trigger: crate::core::DelayedTrigger) -> Result<()> {
        use crate::core::DelayedEffect;

        let card_id = trigger.tracked_card;
        let controller = trigger.controller;

        match trigger.effect {
            DelayedEffect::ReturnToBattlefield { tapped, to_owner } => {
                // Get the card's current zone and owner
                let (current_zone, card_owner) = {
                    if let Some(card) = self.cards.try_get(card_id) {
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
                    if let Some(card) = self.cards.try_get_mut(card_id) {
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
                        let owner = self.cards.try_get(card_id).map_or(controller, |c| c.owner);
                        let dest = self.death_destination_for_card(card_id);
                        self.move_card(card_id, Zone::Battlefield, dest, owner)?;
                    }
                }
            }

            DelayedEffect::ExileCard => {
                // Exile the card from wherever it is
                if let Some(zone) = self.find_card_zone(card_id) {
                    let owner = self.cards.try_get(card_id).map_or(controller, |c| c.owner);
                    self.move_card(card_id, zone, Zone::Exile, owner)?;
                }
            }

            DelayedEffect::CastWithoutPaying => {
                // TODO: Implement for Suspend mechanic
                // This requires putting the spell on the stack without paying costs
                log::warn!(target: "delayed_triggers", "CastWithoutPaying not yet implemented");
            }

            DelayedEffect::CopySpellAbility { may_choose_targets } => {
                // Copy the spell that triggered this delayed trigger
                // This is used by Jeong Jeong: "copy it and you may choose new targets"
                //
                // MTG Rules 707.10: To copy a spell means to put a copy of it onto the stack.
                // A copy of a spell is not cast.
                //
                // Note: For SpellCast triggers, the triggering spell is passed via
                // tracked_card (which is repurposed to hold the spell being copied).
                log::debug!(
                    target: "delayed_triggers",
                    "CopySpellAbility: copying spell {} (may_choose_targets={})",
                    card_id.as_u32(), may_choose_targets
                );

                // Get the spell to copy from the stack
                if self.stack.contains(card_id) {
                    // Clone the spell card to create a copy
                    let original_spell = self.cards.get(card_id)?;
                    let spell_name = original_spell.name.to_string();
                    let mut spell_copy = original_spell.clone();

                    // Give the copy a new ID
                    let copy_id = self.next_card_id();
                    spell_copy.id = copy_id;

                    // The copy is controlled by the trigger's controller
                    spell_copy.owner = controller;
                    spell_copy.controller = controller;

                    // Add the copy card to the entity store
                    self.cards.insert(copy_id, spell_copy);

                    // Put the copy on the stack (above the original)
                    self.stack.add(copy_id);

                    // Log the copy
                    let controller_name = self.get_player(controller)?.name.clone();
                    self.logger.gamelog(&format!(
                        "{} copies {} (copy id={})",
                        controller_name,
                        spell_name,
                        copy_id.as_u32()
                    ));

                    log::info!(
                        target: "delayed_triggers",
                        "CopySpellAbility: created copy of {} (original={}, copy={})",
                        spell_name, card_id.as_u32(), copy_id.as_u32()
                    );

                    // Note: The copy has the same effects and targets as the original.
                    // MTG Rules 707.10a: A copy has the same characteristics and targets
                    // unless the copying effect specifies otherwise.
                    //
                    // If may_choose_targets is true, the player may choose new targets,
                    // but this requires game loop interaction which is handled separately
                    // when the copy resolves.
                    if may_choose_targets {
                        log::debug!(
                            target: "delayed_triggers",
                            "CopySpellAbility: may_choose_targets=true, target selection deferred to resolution"
                        );
                    }
                } else {
                    log::debug!(
                        target: "delayed_triggers",
                        "CopySpellAbility: spell {} no longer on stack, trigger fizzles",
                        card_id.as_u32()
                    );
                }
            }

            DelayedEffect::ExecuteEffect { effect } => {
                // Execute the stored effect when the delayed trigger fires
                // Used by: SP$ DelayedTrigger (Fatal Fissure, etc.)
                //
                // The effect was parsed at card load time from the Execute$ SVar.
                // We need to resolve it now with the actual target (tracked_card).

                log::debug!(
                    target: "delayed_triggers",
                    "Executing delayed effect: {:?} for card {:?}",
                    effect, card_id
                );

                // For effects that need the tracked card as target, update the target
                match *effect {
                    crate::core::Effect::Earthbend { num_counters, .. } => {
                        // Earthbend needs to target a land we control
                        // Find an untapped land controlled by the trigger controller
                        let target_land = self
                            .battlefield
                            .cards
                            .iter()
                            .find(|&&cid| {
                                self.cards
                                    .get(cid)
                                    .is_ok_and(|c| c.is_land() && c.controller == controller && !c.tapped)
                            })
                            .copied();

                        if let Some(land_id) = target_land {
                            // Execute earthbend on the land (inline the logic)
                            use crate::core::{CardType, CounterType, Keyword};

                            // Get land name before mutable borrow
                            let land_name = self
                                .cards
                                .get(land_id)
                                .map(|c| c.name.to_string())
                                .unwrap_or_else(|_| "Land".to_string());

                            // Modify the land card
                            {
                                let card = self.cards.get_mut(land_id)?;

                                // Add Creature type (still remains a land)
                                if !card.is_creature() {
                                    card.add_type(CardType::Creature);
                                }

                                // Set base P/T to 0/0
                                card.set_temp_base_power(0);
                                card.set_temp_base_toughness(0);

                                // Add Haste keyword
                                card.keywords.insert(Keyword::Haste);
                            }

                            // Add +1/+1 counters
                            self.add_counters(land_id, CounterType::P1P1, num_counters)?;

                            // Register delayed trigger for return-to-battlefield on death/exile
                            use crate::core::{DelayedTrigger, DelayedTriggerCondition};
                            use smallvec::smallvec;

                            let return_trigger = DelayedTrigger::new(
                                crate::core::DelayedTriggerId::new(0),
                                land_id,
                                land_id,
                                controller,
                                DelayedTriggerCondition::ZoneChange {
                                    from_zones: smallvec![Zone::Battlefield],
                                    to_zones: smallvec![Zone::Graveyard, Zone::Exile],
                                },
                                DelayedEffect::ReturnToBattlefield {
                                    tapped: true,
                                    to_owner: true,
                                },
                            );
                            self.delayed_triggers.add(return_trigger);

                            self.logger
                                .normal(&format!("Delayed trigger: earthbend {} on {}", num_counters, land_name));
                        } else {
                            self.logger
                                .normal("Delayed trigger: no valid land target for earthbend");
                        }
                    }
                    crate::core::Effect::DealDamage { .. }
                    | crate::core::Effect::EachDamage { .. }
                    | crate::core::Effect::DrawCards { .. }
                    | crate::core::Effect::DiscardCards { .. }
                    | crate::core::Effect::Loot { .. }
                    | crate::core::Effect::GainLife { .. }
                    | crate::core::Effect::DestroyPermanent { .. }
                    | crate::core::Effect::TapPermanent { .. }
                    | crate::core::Effect::UntapPermanent { .. }
                    | crate::core::Effect::TapOrUntapPermanent { .. }
                    | crate::core::Effect::PumpCreature { .. }
                    | crate::core::Effect::DebuffCreature { .. }
                    | crate::core::Effect::PumpCreatureVariable { .. }
                    | crate::core::Effect::PumpAllCreatures { .. }
                    | crate::core::Effect::AnimateAll { .. }
                    | crate::core::Effect::Mill { .. }
                    | crate::core::Effect::Scry { .. }
                    | crate::core::Effect::Surveil { .. }
                    | crate::core::Effect::CounterSpell { .. }
                    | crate::core::Effect::AddMana { .. }
                    | crate::core::Effect::PutCounter { .. }
                    | crate::core::Effect::MultiplyCounter { .. }
                    | crate::core::Effect::PutCounterAll { .. }
                    | crate::core::Effect::ChangeZoneAll { .. }
                    | crate::core::Effect::RemoveCounter { .. }
                    | crate::core::Effect::ExilePermanent { .. }
                    | crate::core::Effect::SearchLibrary { .. }
                    | crate::core::Effect::AttachEquipment { .. }
                    | crate::core::Effect::CreateToken { .. }
                    | crate::core::Effect::CopyPermanent { .. }
                    | crate::core::Effect::Balance { .. }
                    | crate::core::Effect::SetBasePowerToughness { .. }
                    | crate::core::Effect::Airbend { .. }
                    | crate::core::Effect::Firebend { .. }
                    | crate::core::Effect::GrantCantBeBlocked { .. }
                    | crate::core::Effect::Regenerate { .. }
                    | crate::core::Effect::PreventDamage { .. }
                    | crate::core::Effect::LoseLife { .. }
                    | crate::core::Effect::DestroyAll { .. }
                    | crate::core::Effect::SacrificeAll { .. }
                    | crate::core::Effect::DamageAll { .. }
                    | crate::core::Effect::ForceSacrifice { .. }
                    | crate::core::Effect::TapAll { .. }
                    | crate::core::Effect::UntapAll { .. }
                    | crate::core::Effect::SetLife { .. }
                    | crate::core::Effect::ModalChoice { .. }
                    | crate::core::Effect::Dig { .. }
                    | crate::core::Effect::CreateDelayedTrigger { .. }
                    | crate::core::Effect::CopySpellAbility { .. }
                    | crate::core::Effect::ImmediateTrigger { .. }
                    | crate::core::Effect::ClearRemembered
                    | crate::core::Effect::AddTurn { .. }
                    | crate::core::Effect::AddPhase { .. }
                    | crate::core::Effect::ChooseColor { .. }
                    | crate::core::Effect::Clone { .. }
                    | crate::core::Effect::UnlessCostWrapper { .. }
                    | crate::core::Effect::GainControl { .. }
                    | crate::core::Effect::Fight { .. }
                    | crate::core::Effect::Proliferate
                    | crate::core::Effect::SelfExileFromStack { .. }
                    | crate::core::Effect::MoveSelfBetweenZones { .. }
                    | crate::core::Effect::ConditionalSelfCounter { .. }
                    | crate::core::Effect::Unimplemented { .. }
                    | crate::core::Effect::DealDamageXPaid { .. }
                    | crate::core::Effect::DrawCardsXPaid { .. }
                    | crate::core::Effect::GainLifeDynamic { .. }
                    | crate::core::Effect::DiscardCardsXPaid { .. } => {
                        // Other effect types not yet implemented for delayed triggers
                        // Note: CopySpellAbility inside ExecuteEffect is unusual;
                        // typically CopySpellAbility should be used with DelayedEffect::CopySpellAbility
                        log::warn!(
                            target: "delayed_triggers",
                            "Delayed ExecuteEffect for {:?} not yet implemented",
                            effect
                        );
                    }
                }
            }
        }

        Ok(())
    }

    /// Find which zone a card is currently in.
    /// Look up which zone a card is currently in, if any.
    ///
    /// Scans battlefield, stack, then every player's hidden zones (hand, library,
    /// graveyard, exile, command). Returns `None` if the card has been completely
    /// removed (e.g. token leaving play).
    pub fn find_card_zone(&self, card_id: CardId) -> Option<Zone> {
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
            if zones.command.cards.contains(&card_id) {
                return Some(Zone::Command);
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
            card_definitions: std::sync::Arc::clone(&self.card_definitions), // Arc clone = cheap refcount bump
            // Each clone gets a fresh empty bump allocator
            bump: Bump::new(),
            mana_state_version: self.mana_state_version,
            skip_reveals: self.skip_reveals,
            persistent_effects: self.persistent_effects.clone(),
            delayed_triggers: self.delayed_triggers.clone(),
            remembered_cards: self.remembered_cards.clone(),
            remembered_players: self.remembered_players.clone(),
            extra_turns: self.extra_turns.clone(),
            extra_combat_phases: self.extra_combat_phases,
            is_shadow_game: self.is_shadow_game,
            is_commander_game: self.is_commander_game,
            // pending_cycling_search, pending_cast, pending_activation, and spell_targets are transient game loop state — not cloned (reset to empty).
            pending_cycling_search: None,
            pending_cast: None,
            pending_activation: None,
            pending_activation_effect_idx: None,
            spell_targets: Vec::new(),
            // Network sync queue is intentionally NOT cloned — clones are used
            // for snapshots/replays which must not re-emit network messages.
            pending_library_reorders: std::cell::RefCell::new(Vec::new()),
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

    /// Regression for the snapshot/resume `Cache exists after rebuild` panic.
    ///
    /// `GameState::mana_caches` is `#[serde(skip)]`, so after deserialising
    /// a snapshot it comes back as an empty `Vec`. Before the fix,
    /// `rebuild_mana_cache_if_needed` would silently no-op when the cache
    /// slot was missing and `ManaEngine::update_mut` would panic on the
    /// follow-up `expect("Cache exists after rebuild")`.
    ///
    /// The fix adds `ensure_mana_cache` (and the bulk variant
    /// `ensure_mana_caches_for_all_players`) and calls it from
    /// `rebuild_mana_cache_if_needed`. This test simulates the post-restore
    /// state by manually clearing `mana_caches` and asserts that the
    /// rebuild path re-creates the slot instead of leaving it empty.
    #[test]
    fn test_ensure_mana_cache_after_simulated_resume() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // Simulate the post-deserialize state: `mana_caches` empty.
        game.mana_caches.clear();
        assert!(
            game.get_mana_cache(p1_id).is_none(),
            "precondition: cache should be missing"
        );

        // Calling rebuild_mana_cache_if_needed must not panic and must
        // populate the cache slot for the requested player.
        game.rebuild_mana_cache_if_needed(p1_id);
        assert!(
            game.get_mana_cache(p1_id).is_some(),
            "rebuild_mana_cache_if_needed must create missing slot"
        );

        // ensure_mana_caches_for_all_players should fill in any remaining
        // players (here: p2).
        game.ensure_mana_caches_for_all_players();
        assert!(
            game.get_mana_cache(p2_id).is_some(),
            "ensure_mana_caches_for_all_players must populate every player"
        );

        // Idempotency: calling again must not duplicate entries.
        let len_before = game.mana_caches.len();
        game.ensure_mana_caches_for_all_players();
        assert_eq!(
            game.mana_caches.len(),
            len_before,
            "ensure_mana_caches must be idempotent"
        );
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
        let (drawn, draw_count) = game.draw_card(p1_id).unwrap();
        assert_eq!(drawn, Some(card_id));
        assert_eq!(draw_count, 1); // First draw this turn

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
        // Enable reveal actions for this test (simulates network game)
        game.set_skip_reveals(false);
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

        // Play the land - should log RevealCard, MoveCard, SetTurnEnteredBattlefield, SetLandsPlayedThisTurn
        // RevealCard always used now (with old_mask for undo support)
        game.play_land(p1_id, card_id).unwrap();
        assert_eq!(game.undo_log.len(), 4);
        matches!(
            game.undo_log.peek().unwrap(),
            crate::undo::GameAction::SetLandsPlayedThisTurn { .. }
        );

        // Tap for mana - should log TapCard and AddMana
        game.tap_for_mana(p1_id, card_id).unwrap();
        assert_eq!(game.undo_log.len(), 6); // RevealCard, MoveCard, SetTurnEnteredBattlefield, SetLandsPlayedThisTurn, TapCard, AddMana

        // Untap all - should log TapCard for untap
        game.untap_all(p1_id).unwrap();
        assert_eq!(game.undo_log.len(), 7); // + TapCard (untapped)

        // Verify all actions are logged (RevealCard comes before MoveCard per NETWORK_ARCHITECTURE.md)
        // RevealCard is always logged now (with old_mask for undo support)
        let actions = game.undo_log.actions();
        assert!(matches!(actions[0], crate::undo::GameAction::RevealCard { .. }));
        assert!(matches!(actions[1], crate::undo::GameAction::MoveCard { .. }));
        assert!(matches!(
            actions[2],
            crate::undo::GameAction::SetTurnEnteredBattlefield { .. }
        ));
        assert!(matches!(
            actions[3],
            crate::undo::GameAction::SetLandsPlayedThisTurn { .. }
        ));
        assert!(matches!(
            actions[4],
            crate::undo::GameAction::TapCard { tapped: true, .. }
        ));
        assert!(matches!(actions[5], crate::undo::GameAction::AddMana { .. }));
        assert!(matches!(
            actions[6],
            crate::undo::GameAction::TapCard { tapped: false, .. }
        ));
    }

    /// Regression for mtg-420 (post-merge with fix-scry-choice-pipeline):
    /// scry on the SERVER side must enqueue the scrying player into
    /// `pending_library_reorders` whenever the decision actually moves cards
    /// to the bottom, so NetworkController can broadcast a `LibraryReordered`.
    ///
    /// Under the Phase A-E controller pipeline, the decision is now passed
    /// in by the caller (rather than computed by an engine-baked heuristic),
    /// so this test calls `scry_apply_decision` directly with an explicit
    /// "move the top card to the bottom" decision.
    #[test]
    fn test_scry_server_enqueues_library_reorder_when_changed() {
        let mut game = GameState::new_two_player("P1".into(), "P2".into(), 20);
        game.set_skip_reveals(false); // network mode
                                      // NOT a shadow game -> we are the server / authoritative
        let p1_id = game.players[0].id;

        // Library top card to be scried.
        let top_land = game.next_card_id();
        game.cards
            .insert(top_land, Card::new(top_land, "Mountain".to_string(), p1_id));
        let other = game.next_card_id();
        game.cards
            .insert(other, Card::new(other, "Lightning Bolt".to_string(), p1_id));
        if let Some(zones) = game.get_player_zones_mut(p1_id) {
            // bottom-to-top: `other` is bottom, `top_land` is top
            zones.library.cards = vec![other, top_land];
        }

        let revealed = game.scry_snapshot_top_n(p1_id, 1);
        assert_eq!(revealed.as_slice(), &[top_land]);

        // Decision: move the revealed card to the bottom.
        let decision = crate::game::ScryDecision {
            top: SmallVec::new(),
            bottom: smallvec::smallvec![top_land],
        };

        game.scry_apply_decision(p1_id, &revealed, &decision)
            .expect("scry_apply_decision must not error");

        // The land should have moved to the bottom of the library
        let lib = &game.get_player_zones(p1_id).unwrap().library.cards;
        assert_eq!(lib.len(), 2, "library size unchanged");
        assert_eq!(lib[0], top_land, "the Mountain should have moved to the bottom");

        // And the server must have queued a LibraryReordered for P1.
        let queued: Vec<PlayerId> = game.pending_library_reorders.borrow().clone();
        assert_eq!(
            queued,
            vec![p1_id],
            "server scry must enqueue exactly one pending LibraryReorder for the scrying player \
             (mtg-420)"
        );
    }

    /// A scry whose decision keeps the top card on top (no `bottom` cards) must
    /// NOT spam empty `LibraryReordered` messages — both client and server
    /// already agree on library order.
    #[test]
    fn test_scry_server_no_enqueue_when_unchanged() {
        let mut game = GameState::new_two_player("P1".into(), "P2".into(), 20);
        game.set_skip_reveals(false);
        let p1_id = game.players[0].id;

        let spell_id = game.next_card_id();
        game.cards
            .insert(spell_id, Card::new(spell_id, "Lightning Bolt".to_string(), p1_id));
        if let Some(zones) = game.get_player_zones_mut(p1_id) {
            zones.library.cards = vec![spell_id];
        }

        let revealed = game.scry_snapshot_top_n(p1_id, 1);
        // "Keep on top" decision: revealed → top, nothing to bottom.
        let decision = crate::game::ScryDecision {
            top: revealed.clone(),
            bottom: SmallVec::new(),
        };

        game.scry_apply_decision(p1_id, &revealed, &decision)
            .expect("scry_apply_decision must not error");

        assert!(
            game.pending_library_reorders.borrow().is_empty(),
            "no library reorder => no broadcast (mtg-420)"
        );
    }
}
