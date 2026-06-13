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

    /// Source of the damage currently being dealt by a resolving spell or
    /// ability, used by source-filtered damage prevention (Circle of
    /// Protection, CR 615.6). Set transiently around effect resolution in
    /// `resolve_spell_execute_effects` and read by `deal_damage`; `None`
    /// outside of resolution. Not part of persistent game state (it is always
    /// cleared back to `None` before control returns to the game loop), so it
    /// is skipped during serialization/snapshots — keeping save/restore and
    /// network shadow state identical.
    #[serde(skip)]
    pub current_damage_source: Option<CardId>,

    /// Total non-combat damage the [`current_damage_source`] has dealt during
    /// the in-progress effect resolution, used to fire the "whenever ~ deals
    /// damage" trigger (Spirit Link's `DamageDealtOnce`, CR 119.3) ONCE per
    /// resolution with the aggregated amount — never per individual target —
    /// matching the combat path which fires once per creature-damage event.
    ///
    /// `Some(n)` only between `resolve_spell_execute_effects` setting up the
    /// source and firing the deals-damage trigger at the end of that
    /// synchronous window; `None` everywhere else. Like `current_damage_source`
    /// it is purely transient resolution scratch (always cleared back to `None`
    /// before control returns to the game loop), so it is `#[serde(skip)]` —
    /// snapshots, save/restore, and network shadow state stay identical.
    #[serde(skip)]
    pub damage_dealt_by_source: Option<i32>,

    /// Controller of the spell/ability whose effects are currently being
    /// executed. Set at the start of `resolve_spell_execute_effects` and
    /// cleared at the end; also set by priority.rs when executing activated
    /// ability effects.  Like `current_damage_source` this is purely transient
    /// resolution scratch (`#[serde(skip)]`) — snapshots and network state are
    /// unaffected.
    ///
    /// Used by `execute_counter_spell` to set `had_creature_countered_this_turn`
    /// on the owner of the countered spell when that owner is NOT the same player
    /// as `current_spell_controller` (i.e. countered by an opponent).
    #[serde(skip)]
    pub current_spell_controller: Option<crate::core::PlayerId>,

    /// Toughness of the creature most recently sacrificed as an activation cost,
    /// captured BEFORE the card leaves the battlefield.
    ///
    /// Used by Diamond Valley's `AB$ GainLife | LifeAmount$ X` where
    /// `SVar:X:Sacrificed$CardToughness`. The toughness is set in
    /// `pay_ability_cost(Cost::SacrificePattern { .. })` and read in
    /// `resolve_dynamic_amount(DynamicAmount::SacrificedToughness)`.
    ///
    /// Transient resolution scratch — always cleared back to `None` before
    /// control returns to the game loop (or overwritten on the next sacrifice).
    /// `#[serde(skip)]` so snapshots / network shadow state are unaffected.
    #[serde(skip)]
    pub last_sacrificed_toughness: Option<i32>,

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

    /// Remembered numeric value for effect-resolution chains (`RememberNumber$`).
    ///
    /// Set when a `CounterSpell` with `RememberCounteredCMC$ True` resolves
    /// (Mana Drain records the countered spell's mana value here), then read by
    /// a chained `CreateDelayedTrigger` which captures it onto the delayed
    /// trigger. Cleared by `DB$ Cleanup | ClearRemembered$ True`. Part of
    /// serialized state so snapshot/resume and network-shadow stay identical.
    #[serde(default)]
    pub remembered_amount: Option<u32>,

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

    // ╔══════════════════════════════════════════════════════════════════════╗
    // ║  TRANSIENT SUB-ACTION SCRATCH STATE — NOT durable game state.          ║
    // ║                                                                        ║
    // ║  Everything reachable through `sub_action_scratch` is per-sub-action   ║
    // ║  continuation/scratch state that exists ONLY to bridge a WASM          ║
    // ║  step_harness()/JS yield: the GameLoop is recreated on every call, so  ║
    // ║  state that would normally live on the GameLoop's Rust call stack is   ║
    // ║  stashed here instead. It is reset between sub-actions and is NOT part ║
    // ║  of the durable, serialized game state (the whole sub-struct is        ║
    // ║  `#[serde(skip)]`, matching the previous per-field `#[serde(skip)]`).  ║
    // ║                                                                        ║
    // ║  *** CANDIDATE TO MOVE OUT of GameState entirely ***                   ║
    // ║  These fields are a known desync hazard (mtg-896 / mtg-677 /         ║
    // ║  mtg-610): a `#[serde(skip)]` field holding real game-loop state       ║
    // ║  across a choice point is dropped on snapshot/rewind/reconnect. The    ║
    // ║  netarch plan eliminates them by re-reaching the intra-turn frontier   ║
    // ║  via rewind+replay (so they never persist across a yield). They are    ║
    // ║  grouped here, clearly delimited, as the first step toward moving      ║
    // ║  them off GameState. Do NOT add new durable state to this sub-struct.  ║
    // ╚══════════════════════════════════════════════════════════════════════╝
    #[serde(skip)]
    pub sub_action_scratch: SubActionScratch,
}

/// Transient sub-action scratch / continuation state grouped off of
/// [`GameState`].
///
/// **This is NOT durable game state.** Every field here is per-sub-action
/// continuation/scratch state that exists only to bridge a WASM
/// `step_harness()` / JS yield, where `GameLoop` is recreated on every call
/// and therefore cannot carry continuation state on its Rust call stack.
/// It is reset between sub-actions and is intentionally NOT serialized.
///
/// **Candidate to move out of `GameState` entirely** (mtg-896 / mtg-677 /
/// mtg-610): a `#[serde(skip)]` field holding real game-loop state across a
/// choice point is a desync hazard (dropped on snapshot/rewind/reconnect).
/// The netarch plan eliminates these fields by re-reaching the intra-turn
/// frontier via rewind+replay instead of stashing them across a yield.
///
/// The whole struct is `#[serde(skip)]` on `GameState`, which preserves the
/// exact (per-field) `#[serde(skip)]` behaviour each field had before being
/// grouped — so rewind/replay hashing stays bit-identical. It deliberately
/// does NOT derive `Serialize`/`Deserialize`; it is never serialized.
#[derive(Debug, Default)]
pub struct SubActionScratch {
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
    pub pending_cycling_search: Option<(PlayerId, crate::core::Subtype)>,

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
    ///
    /// Each entry is `(player, action_count)` where `action_count` is the
    /// undo-log position of the reorder's own action (`ReorderLibrary` for
    /// scry/surveil, `ShuffleLibrary` for a shuffle) — the game position at
    /// which the post-reorder order is in effect (mtg-752). The
    /// `NetworkController` carries this ac onto the `ServerMessage::LibraryReordered`
    /// so the shadow can key the new order in its game-ac-indexed state-sync log.
    ///
    /// NOTE (mtg-896 / §6 of the netarch design): this field is a
    /// *partially separate* concern from the pure continuation fields above —
    /// it is a network scry/surveil side-channel, not suspended-stack residue.
    /// Its eventual elimination folds into mtg-752, not the long-lived-GameLoop
    /// change; it is grouped here only because it shares the
    /// `#[serde(skip)]`-on-GameState smell.
    pub pending_library_reorders: std::cell::RefCell<Vec<(PlayerId, u64)>>,

    /// Pending Dig-effect decisions to broadcast (server side, mtg-677/mtg-908).
    ///
    /// When the SERVER runs `execute_dig` for a self-dig it pushes
    /// `(digger, kept_card_ids, action_count)` here, where `kept_card_ids` is
    /// the authoritative list of cards that were moved to `destination`, and
    /// `action_count` is the undo-log position at the time of the decision.
    /// The `NetworkController` drains this list (alongside
    /// `pending_library_reorders`) and includes it in the `ChoiceRequest`,
    /// so the server coordinator can broadcast it to BOTH clients before
    /// forwarding the choice.
    ///
    /// The **shadow** does NOT push here; instead it pops from
    /// `pending_dig_authoritative_decision` (populated by `apply_state_sync_at`
    /// before the shadow runs `execute_dig`).
    ///
    /// Like `pending_library_reorders`, this is a network side-channel
    /// (not suspended-stack residue) that lives in `SubActionScratch` for the
    /// `#[serde(skip)]` behaviour.
    // clippy::type_complexity: this is a compact internal queue type; factoring
    // into a named type alias would only add indirection for a single use site.
    #[allow(clippy::type_complexity)]
    pub pending_dig_decisions:
        std::cell::RefCell<Vec<(crate::core::PlayerId, smallvec::SmallVec<[crate::core::CardId; 8]>, u64)>>,

    /// Authoritative Dig decision delivered to the SHADOW by the state-sync
    /// log (mtg-677/mtg-908).
    ///
    /// Before the shadow runs `execute_dig`, `apply_state_sync_at` /
    /// `apply_state_sync_up_to_frontier` sets this to the server's kept-list.
    /// `execute_dig` on the shadow checks this field FIRST: if `Some`, it
    /// uses those card IDs as the "kept" set and clears the field; if `None`
    /// it falls back to the heuristic (only safe for non-network / local AI
    /// games where hidden info is acceptable).
    pub pending_dig_authoritative_decision: Option<smallvec::SmallVec<[crate::core::CardId; 8]>>,

    /// The permanent most recently sacrificed to pay an activated-ability cost
    /// (`Cost$ Sac<N/Type>`), recorded during cost payment and consumed by the
    /// SAME ability's effects, then cleared.
    ///
    /// Used by `Sacrificed$CardToughness`-style dynamic amounts (Diamond Valley:
    /// "gain life equal to the sacrificed creature's toughness"). Activated
    /// abilities resolve IMMEDIATELY (cost payment is directly followed by the
    /// effect loop with no choice/priority boundary in between — see
    /// `priority.rs`, TODO mtg-70), so this scratch is provably `None` at every
    /// serialize / choice / game-loop boundary, exactly like
    /// `current_damage_source`. It carries NO rewind obligation: on rewind+replay
    /// the same sacrifice re-runs and re-populates it identically.
    ///
    /// Not serialized — transient per-sub-action resolution scratch.
    pub sacrificed_for_cost: Option<CardId>,
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
            current_damage_source: None,
            damage_dealt_by_source: None,
            current_spell_controller: None,
            last_sacrificed_toughness: None,
            delayed_triggers: DelayedTriggerStore::new(),
            remembered_cards: smallvec::SmallVec::new(),
            remembered_players: smallvec::SmallVec::new(),
            remembered_amount: None,
            extra_turns: std::collections::VecDeque::new(),
            extra_combat_phases: 0,
            is_shadow_game: false, // Default: not a shadow game
            is_commander_game: false,
            // Transient sub-action scratch/continuation state (see SubActionScratch).
            sub_action_scratch: SubActionScratch::default(),
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

    /// Set a card's temporary base power and/or toughness override, logging a
    /// reversible `GameAction::SetTempBaseStats` (mtg-614 hole (c)).
    ///
    /// `power`/`toughness` of `None` leave that stat's override unchanged; pass
    /// `Some(v)` to set it. Captures the PRIOR override pair before mutating so
    /// undo restores both exactly (including the prior `None`). Used by Animate /
    /// characteristic-defining / base-setting effects and manlands. No-op (and no
    /// log entry) if the card does not exist.
    pub fn set_temp_base_stats_logged(&mut self, card_id: CardId, power: Option<i8>, toughness: Option<i8>) {
        let (prev_power, prev_toughness) = {
            let card = match self.cards.get_mut(card_id) {
                Ok(c) => c,
                Err(_) => return,
            };
            let prev = (card.temp_base_power(), card.temp_base_toughness());
            if let Some(p) = power {
                card.set_temp_base_power(p);
            }
            if let Some(t) = toughness {
                card.set_temp_base_toughness(t);
            }
            prev
        };
        let prior_log_size = self.logger.log_count();
        self.undo_log.log(
            crate::undo::GameAction::SetTempBaseStats {
                card_id,
                prev_power,
                prev_toughness,
            },
            prior_log_size,
        );
    }

    /// Earthbend-style animate: add the `Creature` card type (keeping the land a
    /// land) and grant `Haste`, logging a reversible `GameAction::AnimateTypeline`
    /// so a rewind+replay removes exactly the type/keyword this granted (mtg-610).
    ///
    /// Soulstone Sanctuary's earthbend animates a land into a 0/0 creature with
    /// haste + counters. The Creature-type add and the `Haste` insert were
    /// previously mutated directly with NO undo entry, so a rewind left them on
    /// the card while replay re-granted them — making the turn-start `keywords`
    /// (Haste, ordinal 4) and type line history-dependent across rewinds (monored
    /// seed=13 turn-12 `cards[N].keywords` drift). Routing through the same
    /// `AnimateTypeline` action the Animate effect uses makes the grant reversible.
    /// Captures the prior typeline + the keywords this call actually adds (dedup
    /// against the existing set) so undo never strips a printed keyword. No-op (no
    /// log entry) if the card does not exist.
    pub fn earthbend_animate_creature_haste_logged(&mut self, card_id: CardId) {
        use crate::core::{CardType, Keyword};
        let (
            prev_types,
            prev_subtypes,
            prev_temp_animate_types,
            prev_temp_animate_subtypes,
            prev_temp_removed_subtypes,
            granted_keywords,
            types_changed,
        ) = {
            let card = match self.cards.get_mut(card_id) {
                Ok(c) => c,
                Err(_) => return,
            };
            let prev_types = card.types.clone();
            let prev_subtypes = card.subtypes.clone();
            let prev_temp_animate_types = card.temp_animate_types.clone();
            let prev_temp_animate_subtypes = card.temp_animate_subtypes.clone();
            let prev_temp_removed_subtypes = card.temp_removed_subtypes.clone();

            let mut types_changed = false;
            if !card.is_creature() {
                card.add_type(CardType::Creature);
                card.temp_animate_types.push(CardType::Creature);
                types_changed = true;
            }
            let mut granted: smallvec::SmallVec<[Keyword; 2]> = smallvec::SmallVec::new();
            if !card.keywords.contains(Keyword::Haste) {
                card.keywords.insert(Keyword::Haste);
                granted.push(Keyword::Haste);
            }
            if types_changed {
                let types = card.types.clone();
                let subtypes = card.subtypes.clone();
                let name = card.name.clone();
                card.definition.cache.update_from_types(&types);
                card.definition.cache.update_from_subtypes(&subtypes, name.as_str());
            }
            (
                prev_types,
                prev_subtypes,
                prev_temp_animate_types,
                prev_temp_animate_subtypes,
                prev_temp_removed_subtypes,
                granted,
                types_changed,
            )
        };

        if types_changed || !granted_keywords.is_empty() {
            let prior_log_size = self.logger.log_count();
            self.undo_log.log(
                crate::undo::GameAction::AnimateTypeline {
                    card_id,
                    prev_types,
                    prev_subtypes,
                    prev_temp_animate_types,
                    prev_temp_animate_subtypes,
                    prev_temp_removed_subtypes,
                    granted_keywords,
                    granted_keyword_args: Vec::new(),
                },
                prior_log_size,
            );
        }
    }

    /// Clear a card's temporary base P/T overrides (end-of-turn cleanup of
    /// Animate effects), logging a reversible `GameAction::SetTempBaseStats`
    /// (mtg-614 hole (c)). No-op (and no log entry) if the card does not exist or
    /// already has no override set.
    pub fn clear_temp_base_stats_logged(&mut self, card_id: CardId) {
        let (prev_power, prev_toughness) = {
            let card = match self.cards.get_mut(card_id) {
                Ok(c) => c,
                Err(_) => return,
            };
            let prev = (card.temp_base_power(), card.temp_base_toughness());
            if prev.0.is_none() && prev.1.is_none() {
                return; // nothing to clear → nothing to log
            }
            card.clear_temp_base_stats();
            prev
        };
        let prior_log_size = self.logger.log_count();
        self.undo_log.log(
            crate::undo::GameAction::SetTempBaseStats {
                card_id,
                prev_power,
                prev_toughness,
            },
            prior_log_size,
        );
    }

    /// Apply a regeneration shield: consume one shield, tap, clear damage,
    /// remove from combat. CR 701.15a.
    ///
    /// # Errors
    ///
    /// Returns an error if the card doesn't exist.
    pub fn apply_regeneration_shield(&mut self, card_id: CardId) -> Result<()> {
        // Capture pre-state for the per-action undo (mtg-ba6uq #5): the shield
        // decrement, damage clear, and combat removal below are otherwise
        // unlogged (only the tap is logged, via tap_permanent). Snapshot the
        // combat state BEFORE remove_from_combat.
        let prev_combat = Box::new(self.combat.clone());
        let (prev_shields, prev_damage, card_name) = {
            let card = self.cards.get_mut(card_id)?;
            let prev_shields = card.regeneration_shields;
            let prev_damage = card.damage;
            // Consume one shield
            card.regeneration_shields = card.regeneration_shields.saturating_sub(1);
            // Remove all damage
            card.damage = 0;
            (prev_shields, prev_damage, card.name.clone())
        };
        // Tap the creature (needs &mut self so can't hold card borrow). This
        // logs its own TapCard action, undone separately.
        self.tap_permanent(card_id)?;

        // Remove from combat if attacking or blocking (CR 701.15a)
        self.combat.remove_from_combat(card_id);

        // Log the covering action (after the tap, so LIFO undo restores
        // shields/damage/combat first, then untaps).
        let prior_log_size = self.logger.log_count();
        self.undo_log.log(
            crate::undo::GameAction::RegenerateReplaceDestroy {
                card_id,
                prev_shields,
                prev_damage,
                prev_combat,
            },
            prior_log_size,
        );

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

        // Capture the RNG state BEFORE the shuffle consumes randomness
        // (mtg-728 sig-2). Reversing the shuffle restores this, so a
        // partial-rewind replay re-shuffles from the SAME RNG and
        // byte-reproduces the forward order. Captured here (before the
        // `player_zones` mutable borrow) to avoid a borrow conflict.
        let rng_state = self.capture_rng_state();

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
                    rng_state,
                },
                prior_log_size,
            );

            // NETWORK SYNC (mtg-752 L2b, residual-#1 fix): emit a
            // `LibraryReordered` for this shuffle so the shadow can reproduce the
            // owner's post-shuffle library order. Previously ONLY scry/surveil
            // (`log_library_reorder` callers) enqueued reorders — a bare
            // `shuffle_library` (Timetwister, Wheel, tutor-then-shuffle) logged
            // `ShuffleLibrary` for undo but emitted NO `LibraryReordered`, so the
            // shadow's library order went stale after any shuffle and subsequent
            // draws popped the wrong CardIds (mtg-744 seed-2 turn-16: P1's hand
            // missing card 105 after a Timetwister). Stamp at the `ShuffleLibrary`
            // action's own ac (undo-log length right after the log above), so the
            // shadow adopts the new order at the SAME game position on the forward
            // pass and on every rewind/replay. Server network mode only
            // (`!skip_reveals && !is_shadow_game`); the game-start shuffle runs
            // under `skip_reveals` and is synced via the explicit ac-0
            // `LibraryReordered` messages instead. The reserved CardIds carry no
            // identity, so broadcasting the order to the opponent leaks nothing.
            if !self.skip_reveals && !self.is_shadow_game {
                let reorder_ac = self.undo_log.len() as u64;
                self.sub_action_scratch
                    .pending_library_reorders
                    .borrow_mut()
                    .push((player_id, reorder_ac));
            }
        }
    }

    /// Capture `player`'s current library order (and graveyard order when
    /// `include_graveyard` is set) and log a `ReorderLibrary` undo action.
    ///
    /// MUST be called BEFORE the raw zone reorder it covers — the captured
    /// snapshot is exactly what `GameAction::ReorderLibrary`'s undo restores.
    /// Shared by scry / surveil / Dig "rest to bottom" (mtg-ba6uq #2), all of
    /// which reorder the library (and surveil also mills to the graveyard) with
    /// raw `Vec` ops instead of logged `MoveCard`s, leaving the reorder
    /// previously un-undoable. `include_graveyard` is `true` only for surveil.
    pub fn log_library_reorder(&mut self, player: PlayerId, include_graveyard: bool) {
        let snapshot = self.get_player_zones(player).map(|z| {
            (
                z.library.cards.clone(),
                if include_graveyard {
                    Some(z.graveyard.cards.clone())
                } else {
                    None
                },
            )
        });
        if let Some((previous_order, previous_graveyard)) = snapshot {
            let prior_log_size = self.logger.log_count();
            self.undo_log.log(
                crate::undo::GameAction::ReorderLibrary {
                    player,
                    previous_order,
                    previous_graveyard,
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

    /// Hard deterministic cap on how many spell-copies (token objects created by
    /// [`copy_spell_onto_stack`]) may coexist on the stack. A self-replicating
    /// copy effect would otherwise allocate copies without bound and OOM the
    /// process. Set far above any legitimate copy chain so it only ever fires on
    /// a runaway loop. Purely a function of stack state → desync-safe.
    pub const MAX_SPELL_COPIES_ON_STACK: usize = 100;

    /// Put a copy of a spell already on the stack onto the stack (CR 707.10).
    ///
    /// This is the single shared implementation behind every "copy this/that
    /// spell" effect — both the delayed-trigger form (Jeong Jeong:
    /// `Defined$ TriggeredSpellAbility`) and the SubAbility form (Chain
    /// Lightning: `Defined$ Parent`, gated on the controller paying {R}{R}).
    /// Centralising it (rather than re-implementing the clone-new-id-push dance
    /// at each call site) keeps the two paths from silently diverging.
    ///
    /// MTG rules:
    /// - CR 707.10: "To copy a spell ... means to put a copy of it onto the
    ///   stack ... The copy is created using the copiable values of the
    ///   characteristics of the spell". The copy is **not** cast (CR 707.10a),
    ///   so no cast triggers fire — we push directly onto the stack rather than
    ///   routing through `cast_spell`.
    /// - CR 707.10a: the copy has the same targets unless the copying effect
    ///   lets the controller choose new ones. `new_targets = None` keeps the
    ///   original spell's targets (looked up from `spell_targets`); `Some(t)`
    ///   installs retargeted targets (used when the effect says "you may choose
    ///   a new target for that copy").
    /// - CR 707.10c / 111.7: the copy is a token-like game object that ceases to
    ///   exist as a state-based action once it leaves the stack, so it is
    ///   flagged `is_token` and `resolve_spell_finalize` removes it instead of
    ///   sending a phantom card to the graveyard.
    ///
    /// `original_id` must be a card on the stack. Returns the new copy's id, or
    /// `None` if the original is no longer on the stack (the copy "fizzles").
    ///
    /// # Errors
    ///
    /// Returns an error if the original card or the copy's new controller cannot
    /// be looked up in the entity store.
    pub fn copy_spell_onto_stack(
        &mut self,
        original_id: CardId,
        new_controller: PlayerId,
        new_targets: Option<SmallVec<[CardId; 2]>>,
    ) -> Result<Option<CardId>> {
        if !self.stack.contains(original_id) {
            log::debug!(
                target: "copy_spell",
                "copy_spell_onto_stack: spell {} no longer on stack, copy fizzles",
                original_id.as_u32()
            );
            return Ok(None);
        }

        // DETERMINISTIC SAFETY BOUND (anti-OOM): refuse to create a new copy once
        // the stack already holds [`MAX_SPELL_COPIES_ON_STACK`] spell-copies
        // (token objects created by this helper). A self-replicating copy
        // (e.g. a mis-scripted "copy this spell" with no terminating cost) would
        // otherwise allocate copies unbounded and OOM the whole box (the
        // commander-format Return-the-Favor incident: ~419k copies → 40 GB).
        // The bound is a pure function of the current stack contents — identical
        // on server and every client/WASM shadow — so it never causes a desync.
        // It is set far above any legitimate copy chain (Chain Lightning is
        // bounded by {R}{R} mana long before this; real multi-copy effects copy
        // a handful of times), so it only ever fires on a runaway loop.
        let live_copies = self
            .stack
            .cards
            .iter()
            .filter(|&&cid| self.cards.try_get(cid).is_some_and(|c| c.is_token))
            .count();
        if live_copies >= Self::MAX_SPELL_COPIES_ON_STACK {
            log::warn!(
                target: "copy_spell",
                "copy_spell_onto_stack: refusing to copy {} — {} spell-copies already on the stack \
                 (cap {}); breaking a probable infinite copy loop (anti-OOM safety bound).",
                original_id.as_u32(),
                live_copies,
                Self::MAX_SPELL_COPIES_ON_STACK
            );
            return Ok(None);
        }

        // Clone the spell card to create a copy with the copiable values.
        let mut spell_copy = self.cards.get(original_id)?.clone();
        let spell_name = spell_copy.name.to_string();

        let copy_id = self.next_card_id();
        spell_copy.id = copy_id;
        // The copy is owned/controlled by the player the effect specifies.
        spell_copy.owner = new_controller;
        spell_copy.controller = new_controller;
        // A spell copy is a game-created object that ceases to exist outside the
        // stack (CR 707.10c); reuse the token flag so it is cleaned up rather
        // than persisting as a phantom card in the graveyard.
        spell_copy.is_token = true;

        self.cards.insert(copy_id, spell_copy);
        // Put the copy on the stack ABOVE the original (LIFO): the priority loop
        // resolves `stack.cards.last()`, and the original leaves the stack in
        // `resolve_spell_finalize`, so the copy resolves next.
        self.stack.add(copy_id);

        // Register the copy's targets so `resolve_top_spell_from_stack` can find
        // them. Default: the original spell's chosen targets (CR 707.10a). The
        // existing per-call clone of `spell_targets` is unavoidable (we need an
        // owned target list for the copy keyed by its new id).
        let targets: SmallVec<[CardId; 2]> = match new_targets {
            Some(t) => t,
            None => self
                .sub_action_scratch
                .spell_targets
                .iter()
                .find(|(id, _)| *id == original_id)
                .map(|(_, t)| t.clone())
                .unwrap_or_default(),
        };
        self.sub_action_scratch.spell_targets.push((copy_id, targets));

        let controller_name = self.get_player(new_controller)?.name.clone();
        self.logger.gamelog(&format!(
            "{} copies {} (copy id={})",
            controller_name,
            spell_name,
            copy_id.as_u32()
        ));
        log::info!(
            target: "copy_spell",
            "copy_spell_onto_stack: created copy of {} (original={}, copy={}, controller={})",
            spell_name, original_id.as_u32(), copy_id.as_u32(), new_controller.as_u32()
        );

        Ok(Some(copy_id))
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

    /// Conceal a card that is entering a hidden library (mtg-728 sig-2b).
    ///
    /// A card put into the library becomes hidden again (CR 401: the library is a
    /// hidden zone; a card shuffled in is no longer revealed). If we leave a
    /// stale `revealed_to_mask` on it, a later draw of that same card sees
    /// `is_revealed_to(owner)` already true and SKIPS the `RevealCard` that
    /// `maybe_reveal_to_player` would otherwise log. Because the server and a
    /// viewer's shadow shuffle independently and draw DIFFERENT cards, one side
    /// can draw a previously-public (e.g. graveyard-origin) card while the other
    /// draws a fresh one — diverging the RevealCard COUNT and desyncing the view
    /// hash (observed in robots42 Timetwister/Wheel/Braingeyser mass-draws).
    /// Clearing the mask on entry makes every library card uniformly hidden, so
    /// every draw re-reveals regardless of which card lands on top.
    ///
    /// Mirrors `maybe_reveal_to_player`: gated on `skip_reveals`, logs an
    /// (undoable) `SetRevealedToMask`. Concealing on library ENTRY makes the
    /// later draw's reveal UNCONDITIONAL on both sides (mtg-728 sig-2d):
    /// without it a card that cycled library->hand->library keeps a stale
    /// `revealed_to_mask` on the SERVER (real instance) so the redraw SKIPS the
    /// RevealCard, while the viewer's shadow holds it as a reserved ID and logs
    /// the reveal unconditionally — diverging the RevealCard COUNT and the RNG.
    ///
    /// Two cases, kept SYMMETRIC between server (always real instances) and a
    /// viewer's shadow (opponent's hidden cards are reserved, instance-less):
    /// - Real instance with a non-empty mask: clear it + log the conceal.
    /// - Reserved (no instance) on a shadow game: an instance-less card can only
    ///   enter the library from a hidden zone (the owner's hand, where it had
    ///   been revealed to its owner), so the SERVER logs a conceal for the
    ///   corresponding real card — log a matching count-parity `SetRevealedToMask`
    ///   so the action count stays in lockstep.
    #[inline]
    pub fn maybe_conceal_in_library(&mut self, card_id: CardId, prior_log_size: usize) {
        if self.skip_reveals {
            return;
        }
        if let Some(card) = self.cards.try_get_mut(card_id) {
            // Conceal ANY card that is revealed to anyone (mtg-728 sig-2d):
            // a card in the hidden library must be revealed to nobody, so a
            // later draw uniformly re-reveals regardless of prior reveal history.
            if card.revealed_to_mask != 0 {
                let old_value = card.revealed_to_mask;
                card.clear_revealed_to_all();
                self.undo_log.log(
                    crate::undo::GameAction::SetRevealedToMask {
                        card_id,
                        old_value,
                        new_value: 0,
                    },
                    prior_log_size,
                );
            }
        } else if self.is_shadow_game {
            // Reserved (instance-less) opponent card on a shadow — count-parity
            // conceal mirroring the server's real-card conceal (mtg-728 sig-2d).
            self.undo_log.log(
                crate::undo::GameAction::SetRevealedToMask {
                    card_id,
                    old_value: 0,
                    new_value: 0,
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
                // Anything → Library: the card becomes hidden again, so clear any
                // stale revealed_to_mask (mtg-728 sig-2b). Without this, a
                // previously-public card drawn back out skips its RevealCard and
                // the server/shadow reveal counts diverge across independent
                // shuffles.
                (_, Zone::Library) => {
                    self.maybe_conceal_in_library(card_id, prior_log_size);
                }
                _ => {} // Public→Public, Public→Hidden(Hand): no reveal needed
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
                    let prior_log_size = self.logger.log_count();
                    if let Ok(card_mut) = self.cards.get_mut(card_id) {
                        card_mut.tapped = true;
                        self.logger
                            .verbose(&format!("{} ({}) enters the battlefield tapped", card_name, card_id));
                    }
                    // Cover the ETB-tapped write with an undo action so a mid-turn
                    // rewind that undoes this MoveCard also restores tapped=false
                    // (mtg-ba6uq #1). Reuses TapCard; its `mana_state_version`
                    // bump on undo is excluded from every state hash.
                    self.undo_log.log(
                        crate::undo::GameAction::TapCard { card_id, tapped: true },
                        prior_log_size,
                    );
                }
            }

            // Global ETB-tapped replacement: another permanent already on the
            // battlefield (Kismet, Loxodon Gatekeeper, Frozen Aether, Orb of
            // Dreams, …) imposes "permanents matching <predicate> enter tapped".
            // CR 614 replacement applied as the object enters. The predicate's
            // controller restriction (`OppCtrl`/`YouCtrl`) is resolved relative
            // to the SOURCE permanent's controller. Skipped if the card already
            // entered tapped via its own self-replacement above (idempotent).
            let tap_for_global = match self.cards.try_get(card_id) {
                Some(entering) if !entering.tapped => {
                    let entering_controller = entering.controller;
                    let matched = self.battlefield.cards.iter().any(|&src_id| {
                        if src_id == card_id {
                            return false;
                        }
                        let Some(src) = self.cards.try_get(src_id) else {
                            return false;
                        };
                        src.definition.etb_tapped_global.as_ref().is_some_and(|pred| {
                            pred.matches_with_controller(entering, src.controller, entering_controller)
                        })
                    });
                    matched.then(|| entering.name.clone())
                }
                _ => None,
            };
            if let Some(card_name) = tap_for_global {
                if let Ok(card_mut) = self.cards.get_mut(card_id) {
                    card_mut.tapped = true;
                    self.logger
                        .verbose(&format!("{} ({}) enters the battlefield tapped", card_name, card_id));
                }
            }

            // Handle ETB color choice (e.g., Thriving lands)
            if let Some(card) = self.cards.try_get(card_id) {
                if card.definition.cache.etb_choose_color {
                    let exclude_colors = card.definition.cache.etb_exclude_colors.clone();
                    let card_name = card.name.clone();
                    let prev_chosen_color = card.chosen_color;

                    // Pick the most prominent color in the player's deck (excluding excluded colors)
                    let chosen = self.pick_prominent_color(owner, &exclude_colors);

                    let prior_log_size = self.logger.log_count();
                    if let Ok(card_mut) = self.cards.get_mut(card_id) {
                        card_mut.chosen_color = Some(chosen);
                        self.logger
                            .normal(&format!("{} ({}) - chose {:?}", card_name, card_id, chosen));
                    }
                    // Cover the ETB color choice so a mid-turn rewind restores the
                    // previous value (mtg-ba6uq #1).
                    self.undo_log.log(
                        crate::undo::GameAction::SetChosenColor {
                            card_id,
                            prev: prev_chosen_color,
                        },
                        prior_log_size,
                    );
                }
            }

            // Handle ETB "as ~ enters, choose a player" replacement (Black Vise:
            // `K:ETBReplacement:Other:ChooseP` → `DB$ ChoosePlayer | Choices$
            // Player.Opponent`). Modeled like the color choice above: a
            // deterministic engine computation (NOT a hidden-information or
            // RNG-dependent decision), so it is byte-identical on native and
            // WASM and needs no choice-log round-trip. The chosen player is
            // stored in serialized state (`Card::chosen_player`) and read by the
            // `ValidPlayer$ Player.Chosen` trigger gate. CR 614 (replacement
            // effect applied as the object enters).
            if let Some(card) = self.cards.try_get(card_id) {
                if card.definition.cache.etb_choose_player {
                    let controller = card.controller;
                    let card_name = card.name.clone();
                    let prev_chosen_player = card.chosen_player;
                    let chosen = self.pick_chosen_opponent(controller);
                    let prior_log_size = self.logger.log_count();
                    if let Ok(card_mut) = self.cards.get_mut(card_id) {
                        card_mut.chosen_player = chosen;
                        if let Some(p) = chosen {
                            let player_name = self
                                .get_player(p)
                                .map(|pl| pl.name.as_str().to_string())
                                .unwrap_or_else(|_| format!("P{}", p.as_u32()));
                            self.logger
                                .normal(&format!("{} ({}) - chose {}", card_name, card_id, player_name));
                        }
                    }
                    // Cover the ETB player choice (Black Vise) so a mid-turn
                    // rewind restores the previous value (mtg-ba6uq #1).
                    self.undo_log.log(
                        crate::undo::GameAction::SetChosenPlayer {
                            card_id,
                            prev: prev_chosen_player,
                        },
                        prior_log_size,
                    );
                }
            }

            // Handle ETB "as ~ enters, choose a mode" replacement (Palace Siege:
            // `K:ETBReplacement:Other:SiegeChoice` → `DB$ GenericChoice |
            // Choices$ Khans,Dragons | AILogic$ Dragons`). The AI picks the
            // mode specified by `AILogic$` (falling back to the first choice);
            // the chosen mode is stored in `Card::chosen_mode` and gates which
            // upkeep trigger fires (Khans = return creature from graveyard,
            // Dragons = drain each opponent 2 life). CR 614 (ETB replacement).
            if let Some(card) = self.cards.try_get(card_id) {
                if card.definition.cache.etb_choose_mode {
                    let ai_logic = card.definition.cache.etb_mode_ai_logic.clone();
                    let choices = card.definition.cache.etb_mode_choices.clone();
                    let card_name = card.name.clone();
                    let prev_chosen_mode = card.chosen_mode.clone();

                    // Pick the AI-designated choice, or fall back to the first.
                    let chosen = ai_logic
                        .as_deref()
                        .filter(|m| choices.iter().any(|c| c.eq_ignore_ascii_case(m)))
                        .map(|m| m.to_string())
                        .or_else(|| choices.first().cloned())
                        .unwrap_or_else(|| "unknown".to_string());

                    let prior_log_size = self.logger.log_count();
                    if let Ok(card_mut) = self.cards.get_mut(card_id) {
                        card_mut.chosen_mode = Some(chosen.clone());
                        self.logger
                            .normal(&format!("{} ({}) — chose mode: {}", card_name, card_id, chosen));
                    }
                    self.undo_log.log(
                        crate::undo::GameAction::SetChosenMode {
                            card_id,
                            prev: prev_chosen_mode,
                        },
                        prior_log_size,
                    );
                }
            }

            // Handle ETB "pay any amount of life" replacement (Phyrexian Processor,
            // CR 614 replacement effect applied as the object enters).
            // The AI heuristic: pay min(current_life - 1, 7) to aim for a 7/7 token
            // while keeping at least 1 life.  The paid amount is stored in
            // `Card::stored_int` and read later by the `{4},{T}: create X/X token`
            // activated ability.
            if let Some(card) = self.cards.try_get(card_id) {
                if card.definition.cache.etb_pay_life {
                    let controller = card.controller;
                    let card_name = card.name.clone();
                    let prev_stored_int = card.stored_int;
                    let controller_life = self.get_player(controller).map(|p| p.life).unwrap_or(1);
                    // Pay at most 7, but never reduce life below 1.
                    let max_payable = (controller_life - 1).max(0);
                    let pay_amount = max_payable.min(7) as u32;

                    // Deduct life from the controller (drop the mutable borrow
                    // before logging, to satisfy the borrow checker).
                    let prior_log_size = self.logger.log_count();
                    let log_msg = if let Ok(player) = self.get_player_mut(controller) {
                        player.lose_life(pay_amount as i32);
                        let new_life = player.life;
                        Some(format!(
                            "{} pays {} life as {} enters the battlefield (life: {})",
                            player.name, pay_amount, card_name, new_life
                        ))
                    } else {
                        None
                    };
                    if let Some(msg) = log_msg {
                        self.logger.gamelog(&msg);
                    }
                    self.undo_log.log(
                        crate::undo::GameAction::ModifyLife {
                            player_id: controller,
                            delta: pay_amount as i32,
                        },
                        prior_log_size,
                    );

                    // Store the paid amount on the card for later use by the token ability.
                    let prior_log_size = self.logger.log_count();
                    if let Ok(card_mut) = self.cards.get_mut(card_id) {
                        card_mut.stored_int = Some(pay_amount);
                        self.logger.verbose(&format!(
                            "{} ({}) — stored {} life paid on ETB",
                            card_name, card_id, pay_amount
                        ));
                    }
                    self.undo_log.log(
                        crate::undo::GameAction::SetStoredInt {
                            card_id,
                            prev: prev_stored_int,
                        },
                        prior_log_size,
                    );
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
                    // Logged UNCONDITIONALLY below (outside this try_get-gated
                    // match) so the line is present on a network shadow's first
                    // forward pass even when the discarded opponent card's
                    // instance is not materialised yet (mtg-677). No-op here.
                }
                (Zone::Library, Zone::Graveyard) => {
                    // Mill - don't spam, this is logged by mill effect
                }
                _ => {
                    // Other moves are either logged elsewhere or not significant
                }
            }
        }

        // mtg-677: a discard puts the card into the graveyard, a PUBLIC zone
        // (CR 400.2, 404), so its identity is revealed to every player. Emit the
        // line UNCONDITIONALLY (outside the try_get gate above): on a network
        // shadow the discarded OPPONENT card may not be materialised yet — its
        // public `RevealCard` arrives one ChoiceRequest after the forced
        // resolution that discarded it (mtg-677 H2: the shadow's forward
        // GameLoop runs AHEAD of the reveal stream). A try_get-gated line would
        // therefore be DROPPED on the first forward pass but PRESENT on a rewind
        // replay (instance left behind) → a spurious line-count + name
        // LogMismatch. We log from `card_id` directly: DISPLAY shows the real
        // public name when known (server / replay) and a `card#<id>` fallback
        // before the reveal arrives; the rewind/replay verifier compares the
        // reveal-timing-INDEPENDENT id form (`verifier_stable`) so the
        // presentation asymmetry is not a fatal desync. The card is in the
        // graveyard identically either way (turn-start hash proves the STATE).
        if from == Zone::Hand && to == Zone::Graveyard {
            let display = self
                .cards
                .try_get(card_id)
                .map(|card| card.name.to_string())
                .unwrap_or_else(|| format!("card#{}", card_id.as_u32()));
            let stable = format!("card#{} is discarded", card_id.as_u32());
            self.logger
                .normal_reveal_stable(&format!("{} is discarded", display), &stable);
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

        // A card leaving the battlefield becomes a new object on return (CR 400.7);
        // reset its transient state (counters, temporary P/T modifications, damage,
        // cloned characteristics, etc.) and log the restore action for undo.
        if from == Zone::Battlefield {
            if let Ok(card) = self.cards.get_mut(card_id) {
                let original_def = self.card_definitions.get(&card.printed_name).or_else(|| {
                    self.token_definitions
                        .get(card.printed_name.as_str())
                        .map(|arc| arc.as_ref())
                });
                let snapshot = card.capture_state_snapshot();
                let prior_log_size = self.logger.log_count();
                self.undo_log.log(
                    crate::undo::GameAction::RestoreCardState {
                        card_id,
                        snapshot: Box::new(snapshot),
                    },
                    prior_log_size,
                );
                card.reset_transient_state(original_def);
            }
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
    /// Deterministically choose an opponent for an "as ~ enters, choose an
    /// opponent" replacement effect (Black Vise's `Choices$ Player.Opponent`
    /// with `AILogic$ MostCardsInHand`).
    ///
    /// The choice is a pure function of PUBLIC state — each opponent's hand
    /// SIZE (a public count, never the card identities) — so it produces the
    /// same result on the server, the native shadow, and the WASM shadow
    /// (information-independent; see NETWORK_ARCHITECTURE.md). In the common
    /// two-player game there is exactly one opponent, so the choice is forced.
    /// For 3+ players we pick the opponent with the most cards in hand, breaking
    /// ties by lowest `PlayerId` (a stable, deterministic order — never HashMap
    /// iteration). Returns `None` only if `chooser` has no opponents.
    pub(crate) fn pick_chosen_opponent(&self, chooser: PlayerId) -> Option<PlayerId> {
        // Players are stored in a Vec (stable order); iterate it directly so the
        // tie-break is deterministic across engines.
        self.players
            .iter()
            .map(|p| p.id)
            .filter(|&pid| pid != chooser)
            .max_by(|&a, &b| {
                let hand = |pid: PlayerId| self.get_player_zones(pid).map(|z| z.hand.cards.len()).unwrap_or(0);
                // Most cards in hand wins; on a tie, prefer the LOWER PlayerId
                // (so reverse the id comparison since max_by keeps the greatest).
                hand(a).cmp(&hand(b)).then_with(|| b.as_u32().cmp(&a.as_u32()))
            })
    }

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
    ///
    /// CR 702.52: Before drawing, checks whether the player has an eligible
    /// Dredge card in their graveyard. If so, the draw is replaced by the
    /// dredge action (mill N, return dredge card to hand). Dredge is never
    /// applied in [`Self::draw_card_silent`] (opening-hand setup).
    pub fn draw_card(&mut self, player_id: PlayerId) -> Result<(Option<CardId>, u8)> {
        // CR 702.52: Dredge replaces a draw. Check for eligible dredge cards
        // before the normal draw. draw_card_silent (opening hands) is exempt.
        if let Some(dredge_result) = self.try_dredge_instead_of_draw(player_id)? {
            return Ok(dredge_result);
        }
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
            log::trace!(
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
                    log::trace!("Player {} drew card (draw #{} this turn)", player_id.as_u32(), count);

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

    /// CR 702.52: Dredge draw-replacement effect.
    ///
    /// If the drawing player has at least one card with Dredge N in their
    /// graveyard AND their library contains at least N cards, the draw is
    /// replaced: the top N cards are milled and the dredge card returns to
    /// the player's hand. Returns `Some((None, 0))` when dredge fires (no
    /// card is "drawn" in the MTG sense, so `draw_count` stays 0 and no
    /// card-drawn triggers fire). Returns `None` when no eligible dredge
    /// card exists (caller falls through to the normal draw).
    ///
    /// **Heuristic selection** (AI / no human controller): among all eligible
    /// cards, prefer the one with the lowest Dredge N (mills the fewest
    /// cards, preserving more of the library). If multiple candidates have
    /// the same N, pick the one that appears earliest in the graveyard
    /// (oldest in the yard). This heuristic is information-independent — it
    /// uses only zone contents visible to any controller — satisfying the
    /// network-determinism invariant in `NETWORK_ARCHITECTURE.md`.
    fn try_dredge_instead_of_draw(&mut self, player_id: PlayerId) -> Result<Option<(Option<CardId>, u8)>> {
        use crate::core::{Keyword, KeywordArgs};

        // Snapshot graveyard card IDs and library length (immutable borrow).
        let (graveyard_ids, lib_len) = {
            let Some(zones) = self.get_player_zones(player_id) else {
                return Ok(None);
            };
            let ids: SmallVec<[CardId; 8]> = zones.graveyard.cards.iter().copied().collect();
            (ids, zones.library.cards.len())
        };

        // Find the best eligible dredge candidate:
        // eligible = has Dredge N keyword AND library.len() >= N.
        // Prefer lowest N (fewest mills), then earliest in graveyard.
        let mut best: Option<(CardId, u8)> = None; // (card_id, dredge_amount)
        for &cid in &graveyard_ids {
            let Some(card) = self.cards.try_get(cid) else {
                continue;
            };
            if let Some(KeywordArgs::Dredge { amount }) = card.keywords.get_args(Keyword::Dredge) {
                let amount = *amount;
                if lib_len >= amount as usize {
                    let is_better = match best {
                        None => true,
                        Some((_, best_n)) => amount < best_n,
                    };
                    if is_better {
                        best = Some((cid, amount));
                    }
                }
            }
        }

        let Some((dredge_card_id, dredge_n)) = best else {
            return Ok(None); // No eligible dredge card → normal draw proceeds.
        };

        // Dredge fires. Perform the replacement:
        //   1. Mill dredge_n cards from the top of the library.
        //   2. Move dredge_card_id from graveyard → hand.
        //   3. Log the action.

        // Step 1: mill dredge_n cards (reuses existing mill infrastructure).
        self.mill_cards(player_id, dredge_n)?;

        // Step 2: move the dredge card from graveyard to hand.
        self.move_card(dredge_card_id, Zone::Graveyard, Zone::Hand, player_id)?;

        // Step 3: emit a gamelog line visible to all (dredge is a public action).
        let player_name = self
            .get_player(player_id)
            .map(|p| p.name.to_string())
            .unwrap_or_else(|_| format!("Player {}", player_id.as_u32() + 1));
        let card_name = self
            .cards
            .try_get(dredge_card_id)
            .map(|c| c.name.to_string())
            .unwrap_or_else(|| "a card".to_string());
        self.logger.gamelog(&format!(
            "{} dredges {} (mills {}, returns {} from graveyard to hand)",
            player_name, card_name, dredge_n, card_name
        ));

        log::debug!(
            "Dredge: player {} returned {} ({}) from graveyard to hand, milled {}",
            player_id.as_u32(),
            card_name,
            dredge_card_id,
            dredge_n,
        );

        // No card is "drawn" — return draw_count = 0 so card-drawn triggers
        // do not fire (CR 702.52: dredge replaces the draw entirely).
        Ok(Some((None, 0)))
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

    /// AI-heuristic draw replacement for the Dredge keyword (CR 702.52).
    ///
    /// "If you would draw a card, you may mill N cards instead. If you do,
    /// return this card from your graveyard to your hand."
    ///
    /// This method implements the **automatic AI version**: the first Dredge card
    /// found in `player_id`'s graveyard is used whenever the library has at least
    /// N cards (so milling is possible). Returns `true` if the draw was replaced by
    /// a dredge, `false` if the normal draw should proceed.
    ///
    /// Called from `draw_step` (and `execute_draw_cards`) before the actual
    /// `draw_card` call. Because it resolves without controller interaction it is
    /// safe to call from `GameState` context.
    ///
    /// MTG CR 702.52a: "Dredge N means 'As long as you have at least N cards in
    /// your library, if you would draw a card, you may instead put N cards from
    /// the top of your library into your graveyard and return this card from your
    /// graveyard to your hand.'"
    ///
    /// # Errors
    ///
    /// Returns an error if the mill or zone-move operation fails (e.g. invalid
    /// card ID or zone state).
    pub fn try_apply_dredge(&mut self, player_id: PlayerId) -> Result<bool> {
        use crate::core::{Keyword, KeywordArgs};

        // Collect (dredge_card_id, dredge_amount) pairs from the player's graveyard.
        let dredge_candidates: smallvec::SmallVec<[(crate::core::CardId, u8); 4]> = {
            let Some(zones) = self.get_player_zones(player_id) else {
                return Ok(false);
            };
            zones
                .graveyard
                .cards
                .iter()
                .filter_map(|&cid| {
                    self.cards.try_get(cid).and_then(|c| {
                        c.keywords.get_args(Keyword::Dredge).and_then(|args| {
                            if let KeywordArgs::Dredge { amount } = args {
                                Some((cid, *amount))
                            } else {
                                None
                            }
                        })
                    })
                })
                .collect()
        };

        if dredge_candidates.is_empty() {
            return Ok(false);
        }

        // Check library size and pick the first usable dredge card.
        let library_size = self
            .get_player_zones(player_id)
            .map(|z| z.library.cards.len())
            .unwrap_or(0);

        let Some((dredge_card_id, amount)) = dredge_candidates
            .into_iter()
            .find(|(_, amount)| library_size >= *amount as usize)
        else {
            return Ok(false);
        };

        // Apply dredge replacement: mill `amount` cards, return dredge card to hand.
        let dredge_name = self
            .cards
            .try_get(dredge_card_id)
            .map(|c| c.name.to_string())
            .unwrap_or_else(|| "Dredge card".to_string());
        let player_name = self
            .get_player(player_id)
            .map(|p| p.name.to_string())
            .unwrap_or_else(|_| format!("Player {}", player_id.as_u32() + 1));

        self.logger.gamelog(&format!(
            "{player_name} dredges {dredge_name} (mills {amount}, returns to hand)"
        ));

        // Mill N from library.
        self.mill_cards(player_id, amount)?;

        // Return the dredge card from graveyard to hand.
        self.move_card(
            dredge_card_id,
            crate::zones::Zone::Graveyard,
            crate::zones::Zone::Hand,
            player_id,
        )?;

        Ok(true)
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
            // The scried card's identity is hidden information visible only to
            // the scrying player (scry looks at the top of their OWN library;
            // the card does not change zones to a public one). Mark the entry
            // private so opponent-perspective WASM/web UIs mask the card name.
            // The fact that a scry-1 occurred (and the top/bottom decision) is
            // public; only the card name is not. See mtg-412.
            let p = player_id.as_u32() + 1;
            if decision.bottom.is_empty() {
                self.logger.normal_private(
                    &format!("P{} scries 1, keeps {} on top", p, name),
                    player_id,
                    &format!("P{} scries 1, keeps the card on top", p),
                );
            } else {
                self.logger.normal_private(
                    &format!("P{} scries 1, puts {} on bottom", p, name),
                    player_id,
                    &format!("P{} scries 1, puts the card on the bottom", p),
                );
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

        // Capture the pre-reorder library order so a rewind can restore it
        // (mtg-ba6uq #2): the raw remove/insert/push below is not otherwise
        // undo-logged. Logged unconditionally (even a top-only reorder with no
        // cards-to-bottom changes the order).
        self.log_library_reorder(player_id, false);

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
            // ac = undo-log length right after `log_library_reorder` logged the
            // `ReorderLibrary` action (the raw reorder ops below log nothing), i.e.
            // the position at which the post-reorder order is in effect (mtg-752).
            let reorder_ac = self.undo_log.len() as u64;
            self.sub_action_scratch
                .pending_library_reorders
                .borrow_mut()
                .push((player_id, reorder_ac));
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

        // Capture pre-reorder library AND graveyard order so a rewind can
        // restore both (mtg-ba6uq #2): surveil mills to the graveyard and
        // reorders the top with raw Vec ops, none of it otherwise undo-logged.
        self.log_library_reorder(player_id, true);

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
            // ac = undo-log length after `log_library_reorder` (mtg-752).
            let reorder_ac = self.undo_log.len() as u64;
            self.sub_action_scratch
                .pending_library_reorders
                .borrow_mut()
                .push((player_id, reorder_ac));
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
            // Snapshot the FULL counter set BEFORE the add. `add_counter` may
            // trigger +1/+1 ⟷ -1/-1 annihilation (CR 122.3), which a per-type
            // AddCounter reversal cannot undo (the annihilated opposing counters
            // are gone). Logging the pre-state and restoring it on undo reverses
            // the net change exactly (mtg-ba6uq #4). The snapshot is a tiny
            // inline SmallVec.
            let prev_counters = card.counters.clone();
            card.add_counter(counter_type, amount);

            // Log the action with prior log size
            let prior_log_size = self.logger.log_count();
            self.undo_log.log(
                crate::undo::GameAction::SetCardCounters { card_id, prev_counters },
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
            // CR 122.1c: finality counter exiles instead of dying.
            // CR 614: Disintegrate-style "if it would die this turn, exile it
            // instead" zone-change replacement (exile_if_would_die_this_turn).
            if card.get_counter(crate::core::CounterType::Finality) > 0 || card.exile_if_would_die_this_turn {
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
                    self.logger
                        .gamelog(&format!("{} ({}) exiled instead of dying", name, card_id));
                } else {
                    self.logger
                        .gamelog(&format!("{} ({}) dies from lethal damage", name, card_id));
                }
            }
        }

        // Put planeswalkers with 0 loyalty in graveyard (MTG CR 704.5i)
        let planeswalkers_to_destroy: smallvec::SmallVec<[(CardId, PlayerId); 4]> = self
            .battlefield
            .cards
            .iter()
            .copied()
            .filter_map(|card_id| {
                let card = self.cards.try_get(card_id)?;
                if card.is_planeswalker() && card.get_counter(crate::core::CounterType::Loyalty) == 0 {
                    Some((card_id, card.owner))
                } else {
                    None
                }
            })
            .collect();

        for (card_id, owner) in planeswalkers_to_destroy {
            let card_name = self.cards.try_get(card_id).map(|c| c.name.clone());
            let dest = self.death_destination_for_card(card_id);
            self.move_card(card_id, Zone::Battlefield, dest, owner)?;
            if let Some(name) = card_name {
                self.logger.gamelog(&format!(
                    "{} ({}) has 0 loyalty and is put into the graveyard",
                    name, card_id
                ));
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

    /// Apply continuous "destroy any matching permanent that's on the
    /// battlefield" state-trigger sweeps (the `T:Mode$ Always` pattern),
    /// modeled as a state-based-action-like check (CR 603.8 state triggers,
    /// applied deterministically alongside the other SBAs).
    ///
    /// For every permanent that has a
    /// [`StaticAbility::SacrificeMatchingPresent`] (e.g. City in a Bottle),
    /// every OTHER battlefield permanent matching that ability's filter is put
    /// into its owner's graveyard ("sacrificed", per Oracle text). This single
    /// rule covers the one-time on-enter sweep AND "destroy any such permanent
    /// that enters afterward", because it re-runs at every SBA pass.
    ///
    /// Determinism: iterates `battlefield.cards` (insertion-ordered, network /
    /// snapshot stable). Adds NO new game state — derived entirely from the
    /// already-serialized static abilities and `origin_set` on each card.
    ///
    /// # Errors
    ///
    /// Returns an error if zone operations fail.
    pub fn check_set_origin_sacrifice(&mut self) -> Result<()> {
        use crate::core::StaticAbility;

        // Collect (source_id, restriction) pairs from sweepers currently on the
        // battlefield. Cheap clone of the (small) restriction so we don't hold a
        // borrow of self.cards across the mutation below.
        let sweepers: smallvec::SmallVec<[(CardId, crate::core::TargetRestriction); 2]> = self
            .battlefield
            .cards
            .iter()
            .filter_map(|&id| {
                let card = self.cards.try_get(id)?;
                card.static_abilities.iter().find_map(|sa| {
                    if let StaticAbility::SacrificeMatchingPresent { restriction, .. } = sa {
                        Some((id, restriction.clone()))
                    } else {
                        None
                    }
                })
            })
            .collect();

        if sweepers.is_empty() {
            return Ok(());
        }

        // Determine which permanents are swept. A permanent matched by ANY
        // active sweeper is sacrificed. Iterate the battlefield in its stable
        // order so the (rarely >1) results are deterministic.
        let victims: smallvec::SmallVec<[(CardId, PlayerId); 4]> = self
            .battlefield
            .cards
            .iter()
            .filter_map(|&victim_id| {
                let victim = self.cards.try_get(victim_id)?;
                let swept = sweepers
                    .iter()
                    .any(|(source_id, restriction)| restriction.matches_excluding(victim, *source_id));
                if swept {
                    Some((victim_id, victim.owner))
                } else {
                    None
                }
            })
            .collect();

        for (card_id, owner) in victims {
            let card_name = self.cards.try_get(card_id).map(|c| c.name.clone());
            // Death triggers fire before the permanent leaves (CR 603.6c).
            let _ = self.check_death_triggers(card_id);
            let dest = self.death_destination_for_card(card_id);
            self.move_card(card_id, Zone::Battlefield, dest, owner)?;
            if let Some(name) = card_name {
                self.logger.gamelog(&format!(
                    "{} ({}) is sacrificed (originally-printed-set hoser)",
                    name, card_id
                ));
            }
        }

        Ok(())
    }

    /// True if some permanent on the battlefield has a `CantBeCast` static
    /// whose filter matches `card` (CR 605-style cast prohibition). Used to
    /// gate the available-plays enumeration so a prohibited spell is never
    /// offered (City in a Bottle: ARN-origin cards can't be cast).
    ///
    /// General hoser machinery — works for any `S:Mode$ CantBeCast | ValidCard$ <filter>`.
    /// True if some permanent on the battlefield has a `CantBeCast` static
    /// whose `valid_card` matches `card` AND whose `caster_restriction` applies
    /// to `caster_id` given the current turn's active player.
    ///
    /// - `CasterRestriction::Any`         — applies to all players.
    /// - `CasterRestriction::You`         — applies only to the source's controller.
    /// - `CasterRestriction::YouNonActive`— applies only to the source's controller
    ///   while they are the non-active player.
    /// - `CasterRestriction::Opponent`    — applies to all players who are NOT the
    ///   source's controller.
    pub fn is_cast_prohibited(&self, caster_id: crate::core::PlayerId, card: &crate::core::Card) -> bool {
        use crate::core::{CasterRestriction, StaticAbility};
        let active = self.turn.active_player;
        // Pre-compute the caster's sorcery window: active player, main phase, empty stack.
        // Used by OnlySorcerySpeed statics (Teferi, Time Raveler) which prohibit casting
        // only when the player is OUTSIDE a sorcery window.
        let caster_in_sorcery_window =
            caster_id == active && self.turn.current_step.is_sorcery_speed() && self.stack.is_empty();
        self.battlefield.cards.iter().any(|&id| {
            self.cards.try_get(id).is_some_and(|src| {
                src.static_abilities.iter().any(|sa| {
                    if let StaticAbility::CantBeCast {
                        valid_card,
                        caster_restriction,
                        origin_restriction,
                        only_sorcery_speed,
                        ..
                    } = sa
                    {
                        // Origin-scoped prohibitions (e.g. Experimental Frenzy's
                        // `CantBeCast | Origin$ Hand`) are handled by the
                        // zone-specific callers (`has_hand_cast_prohibition`).
                        // Skip them here so library-cast offers are not blocked.
                        if origin_restriction.is_some() {
                            return false;
                        }
                        // OnlySorcerySpeed statics (Teferi, Time Raveler): the
                        // prohibition is lifted when the caster IS in a sorcery
                        // window (active player, main phase, empty stack).
                        if *only_sorcery_speed && caster_in_sorcery_window {
                            return false;
                        }
                        // First check if the card matches the filter
                        if !valid_card.matches(card) {
                            return false;
                        }
                        // Then check if the caster restriction applies to this caster
                        match caster_restriction {
                            CasterRestriction::Any => true,
                            CasterRestriction::You => caster_id == src.controller,
                            CasterRestriction::YouNonActive => caster_id == src.controller && caster_id != active,
                            CasterRestriction::Opponent => caster_id != src.controller,
                        }
                    } else {
                        false
                    }
                })
            })
        })
    }

    /// True if some permanent on the battlefield has a `CantBeCast` static with
    /// `origin_restriction == Some(Zone::Hand)` that prohibits `caster_id` from
    /// casting `card` from their hand.
    ///
    /// Used by `push_castable_spells` and `push_castable_with_fires` (which both
    /// enumerate hand cards) to enforce Experimental Frenzy's
    /// `CantBeCast | ValidCard$ Card | Caster$ You | Origin$ Hand` line.
    pub fn has_hand_cast_prohibition(&self, caster_id: crate::core::PlayerId, card: &crate::core::Card) -> bool {
        use crate::core::{CasterRestriction, StaticAbility};
        use crate::zones::Zone;
        let active = self.turn.active_player;
        self.battlefield.cards.iter().any(|&id| {
            self.cards.try_get(id).is_some_and(|src| {
                src.static_abilities.iter().any(|sa| {
                    if let StaticAbility::CantBeCast {
                        valid_card,
                        caster_restriction,
                        origin_restriction: Some(Zone::Hand),
                        ..
                    } = sa
                    {
                        if !valid_card.matches(card) {
                            return false;
                        }
                        match caster_restriction {
                            CasterRestriction::Any => true,
                            CasterRestriction::You => caster_id == src.controller,
                            CasterRestriction::YouNonActive => caster_id == src.controller && caster_id != active,
                            CasterRestriction::Opponent => caster_id != src.controller,
                        }
                    } else {
                        false
                    }
                })
            })
        })
    }

    /// True if some permanent on the battlefield has a `CantPlayLand` static with
    /// `origin_restriction == Some(Zone::Hand)` that prohibits `player_id` from
    /// playing a land from their hand.
    ///
    /// Used by the land-play enumeration loop in `populate_actions` to enforce
    /// Experimental Frenzy's `CantPlayLand | Player$ You | Origin$ Hand` line.
    pub fn has_hand_land_prohibition(&self, player_id: crate::core::PlayerId) -> bool {
        use crate::core::{CasterRestriction, StaticAbility};
        use crate::zones::Zone;
        let active = self.turn.active_player;
        self.battlefield.cards.iter().any(|&id| {
            self.cards.try_get(id).is_some_and(|src| {
                src.static_abilities.iter().any(|sa| {
                    if let StaticAbility::CantPlayLand {
                        player_restriction,
                        origin_restriction: Some(Zone::Hand),
                        ..
                    } = sa
                    {
                        match player_restriction {
                            CasterRestriction::Any => true,
                            CasterRestriction::You => player_id == src.controller,
                            CasterRestriction::YouNonActive => player_id == src.controller && player_id != active,
                            CasterRestriction::Opponent => player_id != src.controller,
                        }
                    } else {
                        false
                    }
                })
            })
        })
    }

    /// True if `player_id` currently has flash-casting permission for `card`.
    ///
    /// Two sources are checked:
    /// 1. A battlefield permanent controlled by `player_id` with a
    ///    `StaticAbility::CastWithFlash` whose filter matches `card`
    ///    (e.g. Valley Floodcaller's permanent static).
    /// 2. A temporary `PersistentEffectKind::GrantCastWithFlash` effect whose
    ///    filter matches `card` (e.g. Teferi, Time Raveler's +1 ability grant).
    pub fn player_has_cast_with_flash(&self, player_id: PlayerId, card: &crate::core::Card) -> bool {
        use crate::core::StaticAbility;
        // Check permanent battlefield statics
        let has_static = self.battlefield.cards.iter().any(|&id| {
            self.cards.try_get(id).is_some_and(|src| {
                src.controller == player_id
                    && src.static_abilities.iter().any(|sa| {
                        if let StaticAbility::CastWithFlash { valid_card, .. } = sa {
                            valid_card.matches(card)
                        } else {
                            false
                        }
                    })
            })
        });
        if has_static {
            return true;
        }
        // Check temporary GrantCastWithFlash persistent effects
        self.persistent_effects
            .player_has_grant_cast_with_flash(player_id, card)
    }

    /// True if some permanent on the battlefield has a `CantPlayLand` static
    /// whose filter matches `card`. Gates land plays (and, per Forge's
    /// CantPlayLand semantics, spell casts) for prohibited cards. City in a
    /// Bottle: "Players can't cast spells or play lands ... printed in ARN."
    ///
    /// Note: `CantPlayLand` statics with an `origin_restriction` (e.g.
    /// Experimental Frenzy's `Origin$ Hand`) are SKIPPED here and handled by
    /// zone-specific callers (`has_hand_land_prohibition`).
    pub fn is_land_play_prohibited(&self, card: &crate::core::Card) -> bool {
        use crate::core::StaticAbility;
        self.battlefield.cards.iter().any(|&id| {
            self.cards.try_get(id).is_some_and(|src| {
                src.static_abilities.iter().any(|sa| {
                    if let StaticAbility::CantPlayLand {
                        valid_card,
                        origin_restriction,
                        ..
                    } = sa
                    {
                        // Origin-scoped prohibitions are handled by the
                        // zone-specific callers. Skip them here so non-hand
                        // land plays (e.g. from library via Experimental
                        // Frenzy's MayPlay grant) are not blocked.
                        if origin_restriction.is_some() {
                            return false;
                        }
                        valid_card.matches(card)
                    } else {
                        false
                    }
                })
            })
        })
    }

    /// Combined gate: a card may not be put on the stack / played as a land if
    /// either a `CantBeCast` or a `CantPlayLand` static matches it. (Forge's
    /// `CantPlayLand` on City in a Bottle carries the "can't cast spells"
    /// clause too, so it applies to both lands and spells.)
    ///
    /// `caster_id` is the player attempting to cast or play the card; it is
    /// forwarded to `is_cast_prohibited` to evaluate `CasterRestriction`.
    pub fn is_play_prohibited(&self, caster_id: crate::core::PlayerId, card: &crate::core::Card) -> bool {
        self.is_cast_prohibited(caster_id, card) || self.is_land_play_prohibited(card)
    }

    /// True if some permanent on the battlefield has a `CantAttackOrBlockMatching`
    /// static with `cant_attack = true` whose filter matches `card` (CR 508.1c).
    /// Used to exclude matching creatures from the legal-attackers list.
    pub fn is_attack_prohibited(&self, card: &crate::core::Card) -> bool {
        use crate::core::StaticAbility;
        self.battlefield.cards.iter().any(|&id| {
            self.cards.try_get(id).is_some_and(|src| {
                src.static_abilities.iter().any(|sa| {
                    if let StaticAbility::CantAttackOrBlockMatching {
                        cant_attack, filter, ..
                    } = sa
                    {
                        *cant_attack && filter.matches(card)
                    } else {
                        false
                    }
                })
            })
        })
    }

    /// True if some permanent on the battlefield has a `CantAttackOrBlockMatching`
    /// static with `cant_block = true` whose filter matches `card` (CR 509.1b).
    /// Used to exclude matching creatures from the legal-blockers list.
    pub fn is_block_prohibited(&self, card: &crate::core::Card) -> bool {
        use crate::core::StaticAbility;
        self.battlefield.cards.iter().any(|&id| {
            self.cards.try_get(id).is_some_and(|src| {
                src.static_abilities.iter().any(|sa| {
                    if let StaticAbility::CantAttackOrBlockMatching { cant_block, filter, .. } = sa {
                        *cant_block && filter.matches(card)
                    } else {
                        false
                    }
                })
            })
        })
    }

    /// True if some permanent on the battlefield has a `CantBeActivated` static
    /// whose `creature_filter` matches `card`. Used to suppress activated
    /// abilities on matching creatures (Cursed Totem: "activated abilities of
    /// creatures can't be activated", CR 602.1).
    pub fn is_activated_ability_prohibited(&self, card: &crate::core::Card) -> bool {
        use crate::core::StaticAbility;
        self.battlefield.cards.iter().any(|&id| {
            self.cards.try_get(id).is_some_and(|src| {
                src.static_abilities.iter().any(|sa| {
                    if let StaticAbility::CantBeActivated { creature_filter, .. } = sa {
                        creature_filter.matches(card)
                    } else {
                        false
                    }
                })
            })
        })
    }

    /// True if some permanent on the battlefield carries a
    /// `DisableCreatureEtbTriggers` static (i.e. Torpor Orb is in play).
    ///
    /// Used in `check_triggers_inner` to suppress `TriggerEvent::EntersBattlefield`
    /// triggers when the entering permanent is a creature (CR 603.6b / Torpor Orb).
    ///
    /// `entering_card` is the card that just entered the battlefield; the check
    /// only suppresses the trigger if that card is a creature, matching Torpor
    /// Orb's `ValidCause$ Creature` clause.
    pub fn is_creature_etb_trigger_suppressed(&self, entering_card: &crate::core::Card) -> bool {
        if !entering_card.is_creature() {
            return false;
        }
        use crate::core::StaticAbility;
        self.battlefield.cards.iter().any(|&id| {
            self.cards.try_get(id).is_some_and(|src| {
                src.static_abilities
                    .iter()
                    .any(|sa| matches!(sa, StaticAbility::DisableCreatureEtbTriggers { .. }))
            })
        })
    }

    /// True if some permanent on the battlefield has an `OpalescenceStyle` static
    /// and `card` is a non-Aura enchantment OTHER than that source permanent.
    ///
    /// Implements the Layer-4 part of the Opalescence continuous effect (CR 613.1a):
    /// while Opalescence is on the battlefield, each other non-Aura enchantment is
    /// a creature in addition to its other types.
    ///
    /// Used to include enchantments-as-creatures in the legal-attackers and
    /// legal-blockers lists.  Does NOT check whether the card is already a creature
    /// (callers handle the `card.is_creature()` fast path themselves).
    pub fn is_opalescence_creature(&self, card: &crate::core::Card) -> bool {
        use crate::core::StaticAbility;
        // A card is only an "Opalescence creature" if it is an enchantment (but not
        // an Aura — Auras are attached to permanents and can't be declared as
        // attackers or blockers per CR 303.4g / 508.1a).
        if !card.definition.cache.is_enchantment {
            return false;
        }
        // Auras (enchantments with the subtype Aura) are excluded by Opalescence's
        // Affected$ filter (`Enchantment.nonAura+Other`).
        if card.subtypes.contains(&crate::core::Subtype::new("Aura")) {
            return false;
        }
        // Check if any OpalescenceStyle permanent is on the battlefield (other than
        // the card itself — Opalescence says "other non-Aura enchantments").
        self.battlefield.cards.iter().any(|&src_id| {
            if src_id == card.id {
                return false;
            }
            self.cards.try_get(src_id).is_some_and(|src| {
                src.static_abilities
                    .iter()
                    .any(|sa| matches!(sa, StaticAbility::OpalescenceStyle { .. }))
            })
        })
    }

    /// Returns the mana value (CMC) of `card` as an `i32` for use as Opalescence
    /// P/T (Layer 7b, CR 613.4b).  Returns `None` when no OpalescenceStyle static
    /// applies to `card` (i.e. when `is_opalescence_creature` would return `false`).
    pub fn opalescence_pt(&self, card: &crate::core::Card) -> Option<i32> {
        if self.is_opalescence_creature(card) {
            Some(i32::from(card.mana_cost.cmc()))
        } else {
            None
        }
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

    /// Re-derive control of permanents stolen by a one-shot `AB$ GainControl`
    /// with a source-dependent duration (`LoseControl$ LeavesPlay,LoseControl` —
    /// Aladdin: "Gain control of target artifact for as long as you control
    /// Aladdin", CR 800.4a). Sibling of [`Self::recompute_aura_control`]: control
    /// is treated as a continuous, self-correcting effect rather than a one-way
    /// mutation. Each SBA pass, for every permanent carrying a
    /// [`crate::core::Card::control_grant`] `(source, grantee)`:
    ///
    /// - if `grantee` still controls `source` (it is on the battlefield and its
    ///   controller is `grantee`), the grant holds — leave control with `grantee`;
    /// - otherwise the source left play or changed hands, so control reverts to
    ///   the permanent's OWNER and the grant is cleared.
    ///
    /// This makes the duration robust with no special-case cleanup: Aladdin dying,
    /// being bounced, or being stolen back all return the artifact on the next SBA.
    ///
    /// # Errors
    ///
    /// Returns an error if a card lookup fails.
    pub fn recompute_source_control(&mut self) -> Result<()> {
        // Snapshot which sources are on the battlefield + their controllers, so
        // the per-target check below is a pure lookup (no nested borrow of the
        // battlefield while mutating a target).
        for &card_id in &self.battlefield.cards.clone() {
            let Some(card) = self.cards.try_get(card_id) else {
                continue;
            };
            let Some((source, grantee)) = card.control_grant else {
                continue;
            };
            let owner = card.owner;
            let current = card.controller;

            // The grant holds iff the grantee still controls the source permanent.
            let grant_holds = self
                .cards
                .try_get(source)
                .is_some_and(|src| self.battlefield.cards.contains(&source) && src.controller == grantee);

            if grant_holds {
                // Keep control with the grantee (self-correct if some other effect
                // nudged it). Normally already equal — only log/undo on a change.
                if current != grantee {
                    let prior_log_size = self.logger.log_count();
                    let card_mut = self.cards.get_mut(card_id)?;
                    card_mut.controller = grantee;
                    self.undo_log.log(
                        crate::undo::GameAction::ChangeController {
                            card_id,
                            old_controller: current,
                            new_controller: grantee,
                        },
                        prior_log_size,
                    );
                }
                continue;
            }

            // Grant lapsed: revert to owner and clear the record.
            let card_name = card.name.to_string();
            let prior_log_size = self.logger.log_count();
            let card_mut = self.cards.get_mut(card_id)?;
            card_mut.control_grant = None;
            if current != owner {
                card_mut.controller = owner;
                self.undo_log.log(
                    crate::undo::GameAction::ChangeController {
                        card_id,
                        old_controller: current,
                        new_controller: owner,
                    },
                    prior_log_size,
                );
                let owner_name = self
                    .get_player(owner)
                    .map(|p| p.name.to_string())
                    .unwrap_or_else(|_| format!("Player {}", owner.as_u32() + 1));
                self.logger
                    .gamelog(&format!("{} returns to {}'s control", card_name, owner_name));
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
                // Clear the "dealt damage to an opponent this turn" intervening-if
                // flag (Whirling Dervish) — a per-turn transient, cleared at the
                // end-of-turn cleanup step alongside marked damage (CR 514.2).
                card.dealt_damage_to_opponent_this_turn = false;
                // Clear the "attacked this turn" flag (Berserk's end-step destroy
                // intervening-if) on the same per-turn cleanup boundary (CR 514.2).
                card.attacked_this_turn = false;

                // Roll back animation type changes (Mishra's Factory and
                // friends become land-only again at end of turn). We have to
                // refresh the cache flags so combat / mana / target logic
                // stops treating the manland as a creature. Shared with
                // `UndoLog::rewind_to_turn_start` via `Card::revert_temp_animation`.
                let (_touched, reverted_mana_source) = card.revert_temp_animation();
                if reverted_mana_source {
                    any_mana_source_typeline_reverted = true;
                }
            }
        }

        // Remove until-end-of-turn granted keywords (Rockface Village "gains
        // haste until EOT", AnimateAll, ...) for ALL cards, NOT just battlefield
        // permanents (CR 514.2 / mtg-610). This must be zone-independent to
        // match `UndoLog::rewind_to_turn_start`'s all-cards sweep: a creature
        // can be granted an until-EOT keyword and then leave the battlefield the
        // same turn (bounced/killed/sacrificed), so a battlefield-only clear
        // would leave the bit on the now-off-battlefield card while the rewind
        // sweep cleared it everywhere — making the turn-start `keywords`
        // history-dependent across rewinds (mtg-610: monored Nova Hellkite
        // turn-20 Haste drift). Same per-turn-transient, zone-independent class
        // as the `regeneration_shields` reset.
        for card in self.cards.values_mut() {
            card.clear_temp_keywords_until_eot();
            // CR 614: "exile instead of going to graveyard this turn" flag set by
            // PlayFromGraveyard (Chandra −2). Clear at end-of-turn across all zones.
            card.exile_if_would_go_to_graveyard_this_turn = false;
        }

        if any_mana_source_typeline_reverted {
            for (_, cache) in &mut self.mana_caches {
                cache.mark_dirty();
            }
            self.increment_mana_version();
        }
    }

    /// Serialize the current RNG state into a compact `SmallVec` for storage in
    /// a `ChangeTurn` undo-log marker.
    ///
    /// Uses bincode for compact serialization (56 bytes vs 152 bytes for JSON).
    /// `SmallVec<[u8; 64]>` fits ChaCha12Rng serialization (56 bytes, no heap
    /// allocation). Returns `None` if serialization fails.
    ///
    /// OPTIMIZATION: Uses `serialize_into` with a fixed buffer to avoid a `Vec`
    /// allocation — ChaCha12Rng bincode serialization is exactly 56 bytes.
    pub fn capture_rng_state(&self) -> Option<smallvec::SmallVec<[u8; 64]>> {
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
    }

    /// Emit a clean turn-1 start boundary marker into the undo log (mtg-610).
    ///
    /// Turn 2+ each begin with a `ChangeTurn` marker logged by `advance_step`'s
    /// turn-transition path, which `undo::rewind_to_turn_start` uses as the
    /// "rewind here and stop" boundary. Turn 1 has no preceding turn transition,
    /// so without this it has NO boundary marker — a turn-1 rewind then pops the
    /// ENTIRE undo log, unwinding pre-game setup (shuffle + opening-hand draws)
    /// and re-running it non-deterministically on replay. The most visible
    /// symptom is a turn-1 land play lost on rewind+replay (the shadow re-offers
    /// `PlayLand` after the server already used the one-land-per-turn rule).
    ///
    /// We emit a `ChangeTurn` marker for turn 1 (reusing the existing variant so
    /// `rewind_to_turn_start` / `current_turn` need no special-casing) right
    /// after setup completes and before any turn-1 play. It is gated on "turn 1
    /// AND no `ChangeTurn` marker yet", so it fires exactly once across every
    /// entry path (`run_game` / `run_turns` / `run_one_turn`) and is idempotent
    /// under WASM step-harness / network re-entry (which recreates the GameLoop
    /// each call). `ChangeTurn::undo` special-cases `turn_number == 1` so a full
    /// undo-to-empty restores the initial turn 1 (there is no turn 0).
    pub fn ensure_turn_one_boundary(&mut self) {
        if self.turn.turn_number != 1 || self.undo_log.current_turn().is_some() {
            return;
        }
        let active_player = self.turn.active_player;
        let rng_state = self.capture_rng_state();
        let prior_log_size = self.logger.log_count();
        self.undo_log.log(
            crate::undo::GameAction::ChangeTurn {
                from_player: active_player,
                to_player: active_player,
                turn_number: 1,
                rng_state,
            },
            prior_log_size,
        );
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
            // No combat-guard reset needed (mtg-610): the per-turn combat guards
            // (blockers_declared / first-strike / damage-dealt) were deleted now
            // that WASM re-entry resumes via rewind+replay, so an additional
            // combat phase declares blockers and deals damage afresh with no guard
            // to clear.
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

            // Serialize RNG state BEFORE changing turns. This captures the RNG
            // state at the END of the current turn, which will be the START of
            // the next turn after next_turn() is called.
            let rng_state = self.capture_rng_state();

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
                    // Delegate to the canonical reversal so this rewind path
                    // shares the single source of truth (mtg-732). The prior
                    // inline match was a SUBSET with a silent `_ => {}` that
                    // skipped ShuffleLibrary / DeclareAttacker / ClearCombat /
                    // CloneCard / AnimateTypeline / SetCommanderDamage / ... —
                    // a latent hole on the human/MCTS undo-to-choice-point path.
                    if let Err(e) = action.undo(self) {
                        eprintln!("WARNING: Failed to undo action {action:?}: {e}");
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
            // Delegate the per-variant reversal to the single canonical
            // implementation `GameAction::undo` (undo.rs) so there is exactly
            // ONE source of truth for how each action is reversed (mtg-732).
            // Previously this match RE-IMPLEMENTED every arm inline, a DRY
            // footgun where a new variant (or a fix to an existing one) had to
            // be mirrored in both impls or they silently diverged. On the rare
            // failure (e.g. a card that legitimately left the entity store) we
            // log a warning and continue, matching `rewind_to_turn_start`'s
            // tolerance — the historical inline match here also tolerated
            // missing entities via `if let Ok/Some`.
            if let Err(e) = action.undo(self) {
                eprintln!("WARNING: Failed to undo action {action:?}: {e}");
            }

            // After undo, mark all mana caches as needing rebuild
            // (Lazy rebuild on next query - cheaper than incrementally reversing events)
            for (_, cache) in &mut self.mana_caches {
                cache.mark_dirty();
            }

            // When fully rewound to the initial state, clear the transient
            // pending-resolution fields. These are #[serde(skip)] (not in the undo
            // log) so they persist their end-of-game values after rewind; clearing
            // them lets a fully-rewound game replay cleanly in the same session.
            // (The per-turn re-entry guard family was deleted in mtg-610 once WASM
            // re-entry started resuming via rewind+replay, so there is no longer a
            // guard family to reset here.)
            if self.undo_log.is_empty() {
                self.sub_action_scratch.pending_activation = None;
                self.sub_action_scratch.pending_activation_effect_idx = None;
                self.sub_action_scratch.pending_cycling_search = None;
                self.sub_action_scratch.spell_targets.clear();
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
                // Undo-log the removal so rewind-to-turn-start (snapshot/resume,
                // WASM rewind/replay, undo search) restores the trigger AND rolls
                // `next_id` back — mirroring the `Mode$ Phase` firing path below
                // (mtg-519). Without this, a delayed trigger that FIRES during a
                // turn (e.g. Animate Dead's leave-the-battlefield SacrificeOther
                // when the Aura is destroyed) is removed via the plain `remove`,
                // so on rewind its `RegisterDelayedTrigger` undo finds nothing to
                // roll back and `delayed_triggers.next_id` drifts — diverging the
                // turn-start state hash across rewinds (mtg-400 rogerbrand seed-3
                // turn-6 desync).
                let prior_log_size = self.logger.log_count();
                self.undo_log.log(
                    crate::undo::GameAction::FireDelayedTrigger {
                        trigger: Box::new(trigger.clone()),
                    },
                    prior_log_size,
                );
                self.fire_delayed_trigger(trigger)?;
                fired_count += 1;
            }
        }

        Ok(fired_count)
    }

    /// Check and fire `Mode$ Phase` delayed triggers at the beginning of a phase.
    ///
    /// Called by the turn machinery when a new phase begins (e.g. the
    /// pre-combat main phase) to fire one-shot "at the beginning of your next
    /// [main] phase" delayed triggers such as Mana Drain. Matching triggers are
    /// removed (one-shot) and their effects executed.
    ///
    /// `active_player` is the player whose turn it currently is; the trigger's
    /// `ValidPlayer$` / `whose_turn` is matched against it.
    ///
    /// Returns the number of triggers that fired.
    ///
    /// # Errors
    ///
    /// Returns an error if executing a trigger effect fails.
    pub fn check_delayed_triggers_on_phase(
        &mut self,
        phase: crate::core::TriggerPhase,
        active_player: PlayerId,
    ) -> Result<usize> {
        let trigger_ids = self.delayed_triggers.find_phase_triggers(phase, active_player);
        if trigger_ids.is_empty() {
            return Ok(0);
        }

        let mut fired_count = 0;
        for trigger_id in trigger_ids {
            if let Some(trigger) = self.delayed_triggers.remove(trigger_id) {
                // Undo-log the removal so rewind-to-turn-start restores the
                // trigger; the effect's own mutations (AddMana) are logged
                // separately. Without this, a rewind past the firing would lose
                // the trigger and the replay would not re-fire it (mtg-519).
                let prior_log_size = self.logger.log_count();
                self.undo_log.log(
                    crate::undo::GameAction::FireDelayedTrigger {
                        trigger: Box::new(trigger.clone()),
                    },
                    prior_log_size,
                );
                self.fire_delayed_trigger(trigger)?;
                fired_count += 1;
            }
        }
        Ok(fired_count)
    }

    /// Scan each player's opening hand for cards with `K:MayEffectFromOpeningHand` and
    /// process any that declare an upkeep-scry effect (e.g. Sphinx of Foresight).
    ///
    /// Called from `GameLoop::setup_game` immediately after opening hands are dealt,
    /// before Turn 1 begins. For each card with the keyword, the AI unconditionally
    /// reveals it (it is always beneficial — free scry) and registers a one-shot
    /// `Phase=Upkeep` delayed trigger for the owner so that `upkeep_step`'s
    /// `check_delayed_triggers_on_phase(TriggerPhase::Upkeep, ...)` fires it on the
    /// player's first upkeep.
    ///
    /// The SVar chain is parsed with `AbilityParams` (no substring hacks) to confirm
    /// the pattern really is "scry N on first upkeep" before registering the trigger.
    ///
    /// # Errors
    ///
    /// Propagates any `scry_apply_decision` errors (zone manipulation).
    pub fn process_opening_hand_reveals(&mut self, player_ids: &[crate::core::PlayerId]) -> Result<()> {
        use crate::core::{DelayedEffect, DelayedTrigger, DelayedTriggerCondition, Keyword, TriggerPhase};
        use crate::loader::ability_parser::AbilityParams;
        use smallvec::smallvec;

        // Collect (card_id, player_id, scry_count) triples to avoid borrow issues.
        let mut reveals: Vec<(crate::core::CardId, crate::core::PlayerId, u8)> = Vec::new();

        for &player_id in player_ids {
            let hand_card_ids: Vec<crate::core::CardId> = self
                .get_player_zones(player_id)
                .map(|z| z.hand.cards.to_vec())
                .unwrap_or_default();

            for card_id in hand_card_ids {
                let Some(card) = self.cards.try_get(card_id) else {
                    continue;
                };
                if !card.keywords.contains(Keyword::MayEffectFromOpeningHand) {
                    continue;
                }

                // Retrieve the effect-name from the complex keyword args.
                let effect_svar_name = match card.keywords.get_args(Keyword::MayEffectFromOpeningHand) {
                    Some(crate::core::KeywordArgs::MayEffectFromOpeningHand { effect }) => effect.clone(),
                    _ => continue,
                };

                // Parse the top-level SVar (e.g. "RevealCard") to find the SubAbility.
                let sub_ability_name = {
                    let top_body = match card.definition.svars.get(&effect_svar_name) {
                        Some(b) => b.clone(),
                        None => {
                            log::debug!(
                                target: "opening_hand",
                                "MayEffectFromOpeningHand: SVar '{}' not found on {}",
                                effect_svar_name,
                                card.name,
                            );
                            continue;
                        }
                    };
                    // The top-level SVar: "DB$ Reveal | RevealDefined$ Self | SubAbility$ ScryOnUpkeep"
                    // We must find SubAbility$.
                    let params = match AbilityParams::parse_svar_body(&top_body) {
                        Some(p) => p,
                        None => continue,
                    };
                    match params.get("SubAbility") {
                        Some(s) => s.to_string(),
                        None => continue,
                    }
                };

                // Parse the SubAbility SVar (e.g. "ScryOnUpkeep") to find the Triggers$ SVar name.
                let trigger_svar_name = {
                    let sub_body = match card.definition.svars.get(&sub_ability_name) {
                        Some(b) => b.clone(),
                        None => continue,
                    };
                    // "DB$ Effect | Triggers$ TrigOpenScry | Duration$ Permanent"
                    let params = match AbilityParams::parse_svar_body(&sub_body) {
                        Some(p) => p,
                        None => continue,
                    };
                    match params.get("Triggers") {
                        Some(s) => s.to_string(),
                        None => continue,
                    }
                };

                // Parse the Trigger SVar (e.g. "TrigOpenScry") to find the Execute$ SVar name.
                // Note: this SVar is a trigger definition, NOT a DB$/AB$ ability, so it has no
                // record-type prefix. We parse the key=value pairs manually with | and $ splits.
                let execute_svar_name = {
                    let trig_body = match card.definition.svars.get(&trigger_svar_name) {
                        Some(b) => b.clone(),
                        None => continue,
                    };
                    // "Mode$ Phase | Phase$ Upkeep | ValidPlayer$ You | OneOff$ True | Execute$ DBScry"
                    // Parse by splitting on '|' then each fragment on '$'.
                    let mut trig_params: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
                    for fragment in trig_body.split('|') {
                        let fragment = fragment.trim();
                        if let Some((k, v)) = fragment.split_once('$') {
                            trig_params.insert(k.trim(), v.trim());
                        }
                    }
                    // Only handle Phase=Upkeep, ValidPlayer=You patterns.
                    if trig_params.get("Mode").copied() != Some("Phase") {
                        continue;
                    }
                    if trig_params.get("Phase").copied() != Some("Upkeep") {
                        continue;
                    }
                    match trig_params.get("Execute").copied() {
                        Some(s) => s.to_string(),
                        None => continue,
                    }
                };

                // Parse the Execute SVar (e.g. "DBScry") to find the scry count.
                let scry_count: u8 = {
                    let exec_body = match card.definition.svars.get(&execute_svar_name) {
                        Some(b) => b.clone(),
                        None => continue,
                    };
                    // "DB$ Scry | ScryNum$ 3"
                    let params = match AbilityParams::parse_svar_body(&exec_body) {
                        Some(p) => p,
                        None => continue,
                    };
                    if params.api_type != crate::loader::ability_parser::ApiType::Scry {
                        continue;
                    }
                    match params.get_u8("ScryNum") {
                        Ok(n) => n,
                        Err(_) => continue,
                    }
                };

                reveals.push((card_id, player_id, scry_count));
            }
        }

        // Register one delayed trigger per reveal.
        for (card_id, player_id, scry_count) in reveals {
            let card_name = self
                .cards
                .try_get(card_id)
                .map(|c| c.name.to_string())
                .unwrap_or_else(|| "Unknown".to_string());

            self.logger.gamelog(&format!(
                "{} reveals {} from opening hand (will scry {} on first upkeep)",
                self.get_player(player_id).map(|p| p.name.as_str()).unwrap_or("Player"),
                card_name,
                scry_count,
            ));

            let trigger = DelayedTrigger::new(
                crate::core::DelayedTriggerId::new(0), // id assigned by store.add()
                card_id,
                card_id,
                player_id,
                DelayedTriggerCondition::Phase {
                    phases: smallvec![TriggerPhase::Upkeep],
                    whose_turn: crate::core::delayed_trigger::TurnOwner::You,
                },
                DelayedEffect::ExecuteEffect {
                    effect: Box::new(crate::core::Effect::Scry {
                        player: player_id,
                        count: scry_count,
                        only_if_bargained: false,
                    }),
                },
            );

            let prior_log_size = self.logger.log_count();
            let trigger_id = self.delayed_triggers.add(trigger);
            self.undo_log.log(
                crate::undo::GameAction::RegisterDelayedTrigger { id: trigger_id },
                prior_log_size,
            );

            log::debug!(
                target: "opening_hand",
                "Registered opening-hand upkeep-scry delayed trigger #{} for {} (card: {}, scry {})",
                trigger_id.as_u32(),
                player_id,
                card_name,
                scry_count,
            );
        }

        Ok(())
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
        let remembered_amount = trigger.remembered_amount;

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

            DelayedEffect::SacrificeOther { card: sac_card } => {
                // Sacrifice a card OTHER than the tracked one (Animate Dead: the
                // tracked Aura just left the battlefield; sacrifice the
                // reanimated creature). CR 701.21 sacrifice: move from the
                // battlefield to its owner's graveyard (or the death
                // destination). No-op if the creature already left the
                // battlefield (e.g. it died first, which removed the Aura via
                // SBA and fired THIS trigger) — find_card_zone gates on that, so
                // the creature is never double-handled.
                if self.find_card_zone(sac_card) == Some(Zone::Battlefield) {
                    let owner = self.cards.try_get(sac_card).map_or(controller, |c| c.owner);
                    let card_name = self
                        .cards
                        .try_get(sac_card)
                        .map(|c| c.name.to_string())
                        .unwrap_or_else(|| format!("card#{}", sac_card.as_u32()));
                    let dest = self.death_destination_for_card(sac_card);
                    self.move_card(sac_card, Zone::Battlefield, dest, owner)?;
                    self.logger.normal(&format!("{} is sacrificed", card_name));
                }
            }

            DelayedEffect::ExileCard => {
                // Exile the card from wherever it is
                if let Some(zone) = self.find_card_zone(card_id) {
                    let owner = self.cards.try_get(card_id).map_or(controller, |c| c.owner);
                    self.move_card(card_id, zone, Zone::Exile, owner)?;
                }
            }

            DelayedEffect::DestroyTracked {
                require_attacked_this_turn,
            } => {
                // Berserk: destroy the tracked creature at the next end step,
                // gated on whether it attacked this turn (CR 603.4
                // intervening-if). The gate reads the per-turn `attacked_this_turn`
                // flag — public, rewind-reconstructed state, so the firing is
                // information-independent and replay-faithful. If the creature
                // already left the battlefield the destroy is a no-op (LKI is not
                // needed; "destroy" simply finds nothing to destroy).
                let on_battlefield = self.find_card_zone(card_id) == Some(Zone::Battlefield);
                let gate_ok =
                    !require_attacked_this_turn || self.cards.try_get(card_id).is_some_and(|c| c.attacked_this_turn);
                if on_battlefield && gate_ok {
                    // Mirror the Effect::DestroyPermanent resolution path
                    // (indestructible / regeneration / death-trigger aware).
                    // Berserk allows regeneration (no NoRegen$ on the script).
                    let (owner, name, has_indestructible, has_regen_shield) = {
                        let card = self.cards.get(card_id)?;
                        (
                            card.owner,
                            card.name.to_string(),
                            card.has_indestructible(),
                            card.regeneration_shields > 0,
                        )
                    };
                    if has_indestructible {
                        // CR 702.12b: can't be destroyed — no-op.
                    } else if has_regen_shield {
                        // CR 701.15a: regeneration replaces destruction.
                        self.apply_regeneration_shield(card_id)?;
                    } else {
                        let dest = self.death_destination_for_card(card_id);
                        let _ = self.check_death_triggers(card_id);
                        self.move_card(card_id, Zone::Battlefield, dest, owner)?;
                        self.logger.gamelog(&format!("{} is destroyed (Berserk)", name));
                    }
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
                // For SpellCast triggers, the triggering spell is passed via
                // tracked_card (`card_id` here), and the copy is controlled by
                // the trigger's controller. Target retargeting (may_choose_targets)
                // is deferred to resolution; the copy keeps the original targets
                // (CR 707.10a) via `copy_spell_onto_stack(.., None)`.
                log::debug!(
                    target: "delayed_triggers",
                    "CopySpellAbility: copying spell {} (may_choose_targets={})",
                    card_id.as_u32(), may_choose_targets
                );
                self.copy_spell_onto_stack(card_id, controller, None)?;
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
                            use crate::core::CounterType;

                            // Get land name before mutable borrow
                            let land_name = self
                                .cards
                                .get(land_id)
                                .map(|c| c.name.to_string())
                                .unwrap_or_else(|_| "Land".to_string());

                            // Add Creature type (still remains a land) + Haste via
                            // the logged helper so the grant is reversible by the
                            // undo log (mtg-610: the inline insert leaked Haste
                            // across rewind+replay).
                            self.earthbend_animate_creature_haste_logged(land_id);

                            // Set base P/T to 0/0 via the logged helper so the
                            // override is reversible by the undo log (mtg-614
                            // hole (c)).
                            self.set_temp_base_stats_logged(land_id, Some(0), Some(0));

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
                    crate::core::Effect::AddMana {
                        ref mana,
                        ref amount_var,
                        ..
                    } => {
                        // Mana Drain: "add an amount of {C} equal to that spell's
                        // mana value" at the controller's next main phase.
                        //
                        // When the script uses a variable amount (`Amount$ X` /
                        // `Count$TriggerRememberAmount`), the count is the value
                        // remembered at registration time (the countered spell's
                        // mana value, captured onto this trigger). Otherwise the
                        // mana cost in the effect is already concrete.
                        let player = controller;
                        let count = if amount_var.is_some() {
                            remembered_amount.unwrap_or(0)
                        } else {
                            u32::from(mana.colorless)
                        };
                        // Mana Drain only ever produces {C}; produce `count`
                        // colorless and log it. (If a future Phase-delayed mana
                        // effect needs colored mana, extend here using `mana`.)
                        if count > 0 {
                            let player_name = self.get_player(player).map(|p| p.name.to_string()).ok();
                            let prior_log_size = self.logger.log_count();
                            {
                                let p = self.get_player_mut(player)?;
                                for _ in 0..count {
                                    p.mana_pool.add_color(crate::core::Color::Colorless);
                                }
                            }
                            // Undo-log the mana addition (mirrors the AddMana
                            // effect path) so rewind/replay reverses it.
                            let mut added = crate::core::ManaCost::new();
                            added.colorless = u8::try_from(count).unwrap_or(u8::MAX);
                            self.undo_log.log(
                                crate::undo::GameAction::AddMana {
                                    player_id: player,
                                    mana: added,
                                },
                                prior_log_size,
                            );
                            if let Some(name) = player_name {
                                self.logger
                                    .gamelog(&format!("{} adds {{C}}×{} to mana pool (delayed trigger)", name, count));
                            }
                        }
                    }
                    crate::core::Effect::Scry { player, count, .. } => {
                        // Sphinx of Foresight: "scry 3 at the beginning of your first upkeep"
                        // from opening-hand reveal. The delayed trigger was registered at game
                        // start with a concrete player ID and count. Use the GameState-level
                        // scry path (heuristic: keep all on top) since no controller reference
                        // is available at delayed-trigger-fire time.
                        let player_name = self.get_player(player).map(|p| p.name.to_string()).ok();
                        self.logger.gamelog(&format!(
                            "{} scrys {} (opening-hand reveal trigger)",
                            player_name.as_deref().unwrap_or("Player"),
                            count,
                        ));
                        // Inline the scry: snapshot top N, keep-all-on-top heuristic, apply.
                        let revealed = self.scry_snapshot_top_n(player, count);
                        if !revealed.is_empty() {
                            let decision = crate::game::ScryDecision::keep_all_on_top(&revealed);
                            self.scry_apply_decision(player, &revealed, &decision)?;
                        }
                    }

                    // DestroyPermanent with placeholder target: the tracked card
                    // IS the target (Berserk: "destroy that creature if it attacked
                    // this turn"). card_id here is the `tracked_card` from the
                    // delayed trigger registration — the Berserked creature.
                    // We check the `attacked_this_turn` flag set when the
                    // creature was declared as an attacker.
                    crate::core::Effect::DestroyPermanent {
                        target, no_regenerate, ..
                    } if target.is_placeholder() => {
                        // Only destroy if the creature attacked this turn
                        let did_attack = self
                            .cards
                            .try_get(card_id)
                            .map(|c| c.attacked_this_turn)
                            .unwrap_or(false);

                        if !did_attack {
                            log::debug!(
                                target: "delayed_triggers",
                                "DestroyPermanent delayed trigger: card {:?} did not attack this turn — skipping",
                                card_id
                            );
                        } else if self.battlefield.contains(card_id) {
                            let no_regen = no_regenerate;
                            let has_indestructible =
                                self.cards.try_get(card_id).is_some_and(|c| c.has_indestructible());
                            let has_regen_shield =
                                self.cards.try_get(card_id).is_some_and(|c| c.regeneration_shields > 0);

                            if has_indestructible {
                                log::debug!(
                                    target: "delayed_triggers",
                                    "DestroyPermanent delayed trigger: card {:?} is indestructible — skipping",
                                    card_id
                                );
                            } else if has_regen_shield && !no_regen {
                                self.apply_regeneration_shield(card_id)?;
                            } else {
                                let owner = self.cards.get(card_id)?.owner;
                                let dest = self.death_destination_for_card(card_id);
                                let name = self
                                    .cards
                                    .try_get(card_id)
                                    .map(|c| c.name.to_string())
                                    .unwrap_or_default();
                                log::debug!(
                                    target: "delayed_triggers",
                                    "DestroyPermanent delayed trigger: destroying {} ({:?})",
                                    name, card_id
                                );
                                let _ = self.check_death_triggers(card_id);
                                self.move_card(card_id, Zone::Battlefield, dest, owner)?;
                            }
                        } else {
                            log::debug!(
                                target: "delayed_triggers",
                                "DestroyPermanent delayed trigger: card {:?} no longer on battlefield — skipping",
                                card_id
                            );
                        }
                    }

                    crate::core::Effect::DestroyAll { restriction, .. } => {
                        // DestroyAll in a delayed ExecuteEffect (e.g. Siren's Call:
                        // "Destroy all attacking non-Wall creatures that aren't blocked").
                        // Delegate to the standard execute_effect path; it already
                        // handles TargetRestriction, indestructible, etc.
                        let effect_clone = crate::core::Effect::DestroyAll {
                            restriction,
                            no_regenerate: false,
                            cmc_eq_source: None,
                        };
                        self.execute_effect(&effect_clone)?;
                    }

                    crate::core::Effect::GainLifeDynamic {
                        player,
                        amount,
                        reference,
                    } => {
                        // Delegate to the standard execute_effect path so the
                        // dynamic amount is resolved consistently.
                        let effect_clone = crate::core::Effect::GainLifeDynamic {
                            player,
                            amount,
                            reference,
                        };
                        self.execute_effect(&effect_clone)?;
                    }

                    crate::core::Effect::DealDamage { .. }
                    | crate::core::Effect::DealDamageDivided { .. }
                    | crate::core::Effect::DealDamageDynamic { .. }
                    | crate::core::Effect::DealDamageToTriggeredPlayer { .. }
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
                    | crate::core::Effect::DrainMana { .. }
                    | crate::core::Effect::Surveil { .. }
                    | crate::core::Effect::CounterSpell { .. }
                    | crate::core::Effect::PutCounter { .. }
                    | crate::core::Effect::MultiplyCounter { .. }
                    | crate::core::Effect::PutCounterAll { .. }
                    | crate::core::Effect::ChangeZoneAll { .. }
                    | crate::core::Effect::RemoveCounter { .. }
                    | crate::core::Effect::ExilePermanent { .. }
                    | crate::core::Effect::ExileIfWouldDieThisTurn { .. }
                    | crate::core::Effect::SearchLibrary { .. }
                    | crate::core::Effect::AttachEquipment { .. }
                    | crate::core::Effect::CreateToken { .. }
                    | crate::core::Effect::CreateTokenWithStoredPt { .. }
                    | crate::core::Effect::CopyPermanent { .. }
                    | crate::core::Effect::Balance { .. }
                    | crate::core::Effect::SetBasePowerToughness { .. }
                    | crate::core::Effect::Airbend { .. }
                    | crate::core::Effect::Firebend { .. }
                    | crate::core::Effect::GrantCantBeBlocked { .. }
                    | crate::core::Effect::Regenerate { .. }
                    | crate::core::Effect::PreventDamage { .. }
                    | crate::core::Effect::PreventDamageFromSource { .. }
                    | crate::core::Effect::LoseLife { .. }
                    | crate::core::Effect::SacrificeAll { .. }
                    | crate::core::Effect::DamageAll { .. }
                    | crate::core::Effect::ForceSacrifice { .. }
                    | crate::core::Effect::SacrificeSelf { .. }
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
                    | crate::core::Effect::ReturnCardsFromGraveyardToHand { .. }
                    | crate::core::Effect::PutCardsFromHandOnTopOfLibrary { .. }
                    | crate::core::Effect::RevealCardsFromHand { .. }
                    | crate::core::Effect::ReturnGraveyardCardToHand { .. }
                    | crate::core::Effect::ReturnGraveyardCardToZone { .. }
                    | crate::core::Effect::PutCreatureFromHandOnBattlefield { .. }
                    | crate::core::Effect::ReturnSelfAsEnchantment { .. }
                    | crate::core::Effect::PreventAllCombatDamageThisTurn { .. }
                    | crate::core::Effect::ConditionalSelfCounter { .. }
                    | crate::core::Effect::RearrangeTopOfLibrary { .. }
                    | crate::core::Effect::SkipUntapStep { .. }
                    | crate::core::Effect::Unimplemented { .. }
                    | crate::core::Effect::NoOp { .. }
                    | crate::core::Effect::ClassLevelUp { .. }
                    | crate::core::Effect::DealDamageXPaid { .. }
                    | crate::core::Effect::DrawCardsXPaid { .. }
                    | crate::core::Effect::DiscardCardsXPaid { .. }
                    | crate::core::Effect::CreateTokenDynamic { .. }
                    | crate::core::Effect::CreateEmblem { .. }
                    | crate::core::Effect::PlayFromGraveyard { .. }
                    | crate::core::Effect::GrantCastWithFlash { .. }
                    | crate::core::Effect::ReturnPermanentToHand { .. }
                    | crate::core::Effect::RepeatEach { .. }
                    | crate::core::Effect::ExtraLandPlay { .. }
                    | crate::core::Effect::TapPermanentsMatchingFilter { .. }
                    | crate::core::Effect::ChooseAndRememberOneOfEach { .. } => {
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
            current_damage_source: self.current_damage_source,
            damage_dealt_by_source: self.damage_dealt_by_source,
            current_spell_controller: self.current_spell_controller,
            last_sacrificed_toughness: self.last_sacrificed_toughness,
            delayed_triggers: self.delayed_triggers.clone(),
            remembered_cards: self.remembered_cards.clone(),
            remembered_players: self.remembered_players.clone(),
            remembered_amount: self.remembered_amount,
            extra_turns: self.extra_turns.clone(),
            extra_combat_phases: self.extra_combat_phases,
            is_shadow_game: self.is_shadow_game,
            is_commander_game: self.is_commander_game,
            // Transient sub-action scratch/continuation state (pending_*, spell_targets,
            // pending_library_reorders) is NOT cloned — clones are used for
            // snapshots/replays and must reset to empty (and must not re-emit network
            // messages). A fresh default() preserves the previous per-field reset.
            sub_action_scratch: SubActionScratch::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::Step;

    /// Anti-OOM safety bound: `copy_spell_onto_stack` must refuse to keep
    /// creating copies once the stack already holds MAX_SPELL_COPIES_ON_STACK
    /// spell-copies. This is the deterministic backstop that prevents a
    /// self-replicating copy effect from allocating unbounded copies and OOMing
    /// the process (the commander Return-the-Favor incident: ~419k copies →
    /// 40 GB). The original fix (bare CopySpellAbility → no-op TargetedSpell)
    /// stops THAT card from looping; this bound stops ANY future copy loop.
    #[test]
    fn test_copy_spell_onto_stack_is_bounded_against_runaway_loop() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1 = game.players.first().unwrap().id;

        // Put an original spell on the stack.
        let orig = game.next_entity_id();
        game.cards.insert(orig, Card::new(orig, "Loopy Spell".to_string(), p1));
        game.stack.add(orig);

        // Try to create FAR more copies than the cap allows. Every call keeps
        // the original on the stack, so without the bound this would create one
        // copy per call forever.
        let attempts = GameState::MAX_SPELL_COPIES_ON_STACK + 50;
        let mut created = 0usize;
        for _ in 0..attempts {
            if game.copy_spell_onto_stack(orig, p1, None).unwrap().is_some() {
                created += 1;
            }
        }

        // The helper stops creating copies at the cap (the original is not a
        // token, so only copies count toward the limit).
        assert_eq!(
            created,
            GameState::MAX_SPELL_COPIES_ON_STACK,
            "copy_spell_onto_stack must stop at the MAX_SPELL_COPIES_ON_STACK cap, not loop unbounded"
        );
        let copies_on_stack = game
            .stack
            .cards
            .iter()
            .filter(|&&c| game.cards.try_get(c).is_some_and(|card| card.is_token))
            .count();
        assert_eq!(copies_on_stack, GameState::MAX_SPELL_COPIES_ON_STACK);
    }

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
        let queued: Vec<(PlayerId, u64)> = game.sub_action_scratch.pending_library_reorders.borrow().clone();
        assert_eq!(queued.len(), 1, "exactly one pending LibraryReorder (mtg-420)");
        assert_eq!(
            queued[0].0, p1_id,
            "server scry must enqueue a pending LibraryReorder for the scrying player (mtg-420)"
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
            game.sub_action_scratch.pending_library_reorders.borrow().is_empty(),
            "no library reorder => no broadcast (mtg-420)"
        );
    }
}
