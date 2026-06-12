//! Undo log for efficient game tree search
//!
//! This module provides a transaction log of game actions that can be
//! rewound to efficiently explore the game tree without expensive deep copies.

use crate::core::{CardId, CardStateSnapshot, CounterType, Keyword, PlayerId};
use crate::zones::Zone;
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use std::fmt;

use crate::game::GameState;

/// Target audience for a card reveal
///
/// Specifies WHO should see a card's identity when it's revealed.
/// Per NETWORK_ARCHITECTURE.md, reveals are first-class game actions logged
/// BEFORE any move that depends on the card's identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RevealTarget {
    /// Reveal to a single player only (e.g., drawing a card - only the owner sees it)
    Player(PlayerId),
    /// Reveal to all players (e.g., card entering battlefield - everyone sees it)
    All,
}

/// Atomic game actions that can be logged and undone
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GameAction {
    /// Move a card between zones
    MoveCard {
        card_id: CardId,
        from_zone: Zone,
        to_zone: Zone,
        owner: PlayerId,
        /// Position of the card in `from_zone` *before* the move, so undo
        /// can restore it to the same slot.
        ///
        /// Card order matters in many zones (Library is obviously ordered;
        /// Hand iteration order affects controller decisions; Battlefield
        /// iteration order is stable for determinism — see
        /// `CardZone::remove`'s comment). Without this field, undo
        /// re-appends the card at the end of `from_zone`, which permutes
        /// hand order across rewind/replay cycles and trips the WASM
        /// rewind/replay verifier (turn-start hash drifts on repeated
        /// rewinds to the same turn). `None` is accepted for
        /// backward-compatibility with snapshots that predate this field.
        #[serde(default)]
        from_position: Option<u32>,
    },

    /// Tap/untap a permanent
    TapCard { card_id: CardId, tapped: bool },

    /// Set a card's ETB-chosen color (e.g. Thriving lands' `etb_choose_color`),
    /// storing the PREVIOUS value so a mid-turn rewind that undoes the ETB
    /// `MoveCard` also restores `Card::chosen_color`. Without this the choice
    /// field stays stale on the off-battlefield card and the (hashed) turn-start
    /// state diverges across rewinds (mtg-ba6uq #1).
    SetChosenColor {
        card_id: CardId,
        prev: Option<crate::core::Color>,
    },

    /// Set a card's ETB-chosen player (e.g. Black Vise's "as ~ enters, choose a
    /// player"), storing the PREVIOUS value so a mid-turn rewind restores
    /// `Card::chosen_player` (mtg-ba6uq #1). Black Vise is in the 1994/old-school
    /// target decks, so this is the most current-relevant ETB choice-field hole.
    SetChosenPlayer { card_id: CardId, prev: Option<PlayerId> },

    /// Set a card's ETB-chosen mode (e.g. Palace Siege's `DB$ GenericChoice |
    /// Choices$ Khans,Dragons`), storing the PREVIOUS value so a mid-turn rewind
    /// restores `Card::chosen_mode`. Same rationale as `SetChosenColor`.
    SetChosenMode { card_id: CardId, prev: Option<String> },

    /// Modify life total (delta is the change, not absolute value)
    ModifyLife { player_id: PlayerId, delta: i32 },

    /// Add mana to pool
    AddMana {
        player_id: PlayerId,
        mana: crate::core::ManaCost,
    },

    /// Empty mana pool (stores previous state for undo)
    EmptyManaPool {
        player_id: PlayerId,
        prev_white: u8,
        prev_blue: u8,
        prev_black: u8,
        prev_red: u8,
        prev_green: u8,
        prev_colorless: u8,
    },

    /// Add counter to card
    AddCounter {
        card_id: CardId,
        counter_type: CounterType,
        amount: u8,
    },

    /// Remove counter from card
    RemoveCounter {
        card_id: CardId,
        counter_type: CounterType,
        amount: u8,
    },

    /// Advance game step
    AdvanceStep {
        from_step: crate::game::Step,
        to_step: crate::game::Step,
    },

    /// Change turn (stores RNG state for proper rewind)
    ChangeTurn {
        from_player: PlayerId,
        to_player: PlayerId,
        turn_number: u32,
        /// RNG state at the START of this turn (for snapshot rewind)
        /// SmallVec<[u8; 64]> fits ChaCha12Rng bincode serialization (56 bytes, no heap allocation)
        /// Size 64 chosen as smallest power-of-2 supported by smallvec that fits 56 bytes
        /// INVARIANT: serialization code asserts exactly 56 bytes to catch future changes
        rng_state: Option<SmallVec<[u8; 64]>>,
    },

    /// Pump creature (temporary stat modification and/or keyword grant)
    PumpCreature {
        card_id: CardId,
        power_delta: i32,
        toughness_delta: i32,
        /// Keywords granted by this pump effect (for undo)
        keywords_granted: smallvec::SmallVec<[Keyword; 2]>,
    },

    /// Debuff creature (keyword removal)
    DebuffCreature {
        card_id: CardId,
        /// Keywords removed by this debuff effect (restored on undo)
        keywords_removed: smallvec::SmallVec<[Keyword; 2]>,
    },

    /// Set turn_entered_battlefield field (for summoning sickness tracking)
    SetTurnEnteredBattlefield {
        card_id: CardId,
        /// Previous value (None if wasn't on battlefield)
        old_value: Option<u32>,
        /// New value (Some(turn) when entering battlefield, None when leaving)
        new_value: Option<u32>,
    },

    /// Set lands_played_this_turn counter (for land play limit tracking)
    SetLandsPlayedThisTurn {
        player_id: PlayerId,
        /// Previous count
        old_value: u8,
        /// New count
        new_value: u8,
    },

    /// Set spells_cast_this_turn counter (for prowess / cast triggers).
    ///
    /// Without this, the counter increments forward but never decrements
    /// on undo, so the WASM rewind/replay verifier sees a drift on the
    /// `players[].spells_cast_this_turn` field across rewinds.
    SetSpellsCastThisTurn {
        player_id: PlayerId,
        /// Previous count
        old_value: u8,
        /// New count
        new_value: u8,
    },

    /// Set cards_drawn_this_turn counter (for "second card drawn" triggers)
    SetCardsDrawnThisTurn {
        player_id: PlayerId,
        /// Previous count
        old_value: u8,
        /// New count
        new_value: u8,
    },

    /// Change controller of a permanent (for GainControl effects)
    ChangeController {
        card_id: CardId,
        /// Previous controller
        old_controller: PlayerId,
        /// New controller
        new_controller: PlayerId,
    },

    /// Set attached_to field (for Equipment/Aura attachment tracking)
    SetAttachedTo {
        equipment_id: CardId,
        /// Previous attachment target (None if not attached)
        old_target: Option<CardId>,
        /// New attachment target (None when detaching, Some(card) when attaching)
        new_target: Option<CardId>,
    },

    /// Mark a choice point (for tree search and replay)
    ///
    /// Stores both the fact that a choice occurred and what that choice was,
    /// enabling deterministic replay from snapshots.
    ChoicePoint {
        player_id: PlayerId,
        choice_id: u32,
        /// The actual choice made (for replay). None if choice hasn't been recorded yet.
        choice: Option<crate::game::replay_controller::ReplayChoice>,
    },

    /// Reveal a card's identity (CardID ⟺ CardName binding)
    ///
    /// Part of the late-binding CardID architecture (mtg-218). This action binds
    /// a pre-allocated CardID to its actual card identity (name).
    ///
    /// ## Target Audience
    ///
    /// The `revealed_to` field specifies WHO should see this reveal:
    /// - `RevealTarget::Player(id)`: Only that player sees it (e.g., drawing a card)
    /// - `RevealTarget::All`: Everyone sees it (e.g., card entering battlefield)
    ///
    /// ## Viewer-Specific Content
    ///
    /// This action is logged by ALL players for EVERY reveal, but with different content:
    /// - Players in the target audience: `name = Some("Lightning Bolt")`
    /// - Players NOT in the audience: `name = None` (keeps action_count in sync)
    ///
    /// This keeps action_count synchronized across all clients while maintaining
    /// information asymmetry.
    ///
    /// ## Write-Once Semantics
    ///
    /// Reveals are monotonic: a CardID can only transition from unrevealed (None)
    /// to revealed (Some). The EntityStore enforces this with a panic if attempting
    /// to insert into an already-occupied slot. This prevents revealing CardID 33
    /// as "Lightning Bolt" then later revealing it as "Mountain".
    ///
    /// For game tree exploration, undo clears the slot back to None, allowing
    /// a subsequent re-reveal (which is fine since each timeline only sees
    /// a single None→Some transition).
    ///
    /// ## Forward Logic
    ///
    /// If `name` is Some, the Card should be instantiated and inserted into
    /// the EntityStore at `card_id` by the caller.
    /// If `name` is None, this is a "dummy" reveal that doesn't modify state
    /// (for opponents who don't learn the card identity).
    ///
    /// ## Undo Logic
    ///
    /// Restores the previous revealed_to_mask value. If old_mask is 0 and
    /// name is Some (card was created by this reveal), clears the card slot.
    RevealCard {
        /// The CardID being revealed
        card_id: CardId,
        /// The revealed card name, or None for late-binding (client doesn't know yet)
        name: Option<String>,
        /// Who should see this reveal
        revealed_to: RevealTarget,
        /// Previous mask value (for undo). If 0, this was the first reveal.
        old_mask: u8,
    },

    /// Set revealed_to_mask field (for tracking which players have seen a card)
    ///
    /// DEPRECATED: Use RevealCard with old_mask instead. This is kept for
    /// backwards compatibility with existing undo logs but should not be
    /// logged in new code.
    SetRevealedToMask {
        card_id: CardId,
        /// Previous mask value (for undo)
        old_value: u8,
        /// New mask value
        new_value: u8,
    },

    /// Set loyalty_activated_this_turn flag on a card (for planeswalker once-per-turn rule)
    SetLoyaltyActivated {
        card_id: CardId,
        /// Previous value (for undo)
        old_value: bool,
        /// New value
        new_value: bool,
    },

    /// Set commander_cast_count on a player (for commander tax tracking)
    SetCommanderCastCount {
        player_id: PlayerId,
        /// Previous count (for undo)
        old_value: u8,
        /// New count
        new_value: u8,
    },

    /// Record commander damage taken (for 21-damage loss condition tracking)
    SetCommanderDamage {
        player_id: PlayerId,
        /// The opponent whose commander dealt damage
        from_player: PlayerId,
        /// Previous cumulative damage (for undo)
        old_damage: u16,
        /// New cumulative damage
        new_damage: u16,
    },

    /// Shuffle a player's library
    ///
    /// Stores the previous order of CardIds so it can be restored on undo.
    /// This is essential for deterministic replay and game tree search when
    /// tutor effects (search library, then shuffle) are involved.
    ///
    /// ## Network Considerations
    ///
    /// In network mode, after shuffling the server sends a LibraryReordered
    /// message to clients with the new order. The previous_order stored here
    /// is the order BEFORE shuffling, which is only known to the server.
    ShuffleLibrary {
        /// Which player's library was shuffled
        player: PlayerId,
        /// Previous order of CardIds (for undo)
        /// Stored as Vec since library size varies and SmallVec wouldn't help
        previous_order: Vec<CardId>,
        /// RNG state captured BEFORE the shuffle consumed randomness (mtg-mb668
        /// sig-2). A shuffle advances the ChaCha12Rng, so a partial (mid-turn)
        /// rewind that reverses the shuffle MUST also restore the pre-shuffle
        /// RNG — otherwise replaying the shuffle draws from an advanced RNG and
        /// produces a DIFFERENT library order, diverging every downstream
        /// mass-draw (Timetwister / Wheel of Fortune / Braingeyser) and the
        /// shadow's hand. Mirrors `ChangeTurn`'s `rng_state`. `None` only on
        /// legacy/deserialized logs that predate this field.
        rng_state: Option<SmallVec<[u8; 64]>>,
    },

    /// Register a delayed trigger in the delayed-trigger store.
    ///
    /// Undo removes the trigger with this id from the store. Required so
    /// rewind-to-turn-start (snapshot/resume, undo search) reverses a delayed
    /// trigger created during the turn — otherwise the rewound "turn start"
    /// state retains it and the replay double-registers it (mtg-519).
    RegisterDelayedTrigger { id: crate::core::DelayedTriggerId },

    /// Fire (remove + execute) a delayed trigger.
    ///
    /// Stores the full trigger so undo can restore it via the store's
    /// `restore`. The mutating effects of firing (e.g. AddMana) are logged as
    /// their own actions, so undo of those is handled separately.
    FireDelayedTrigger { trigger: Box<crate::core::DelayedTrigger> },

    /// Set `GameState::remembered_amount` (RememberCounteredCMC$ / RememberNumber$).
    ///
    /// Stores the previous value so undo restores it.
    SetRememberedAmount {
        #[serde(default)]
        previous: Option<u32>,
    },

    /// Declare a creature as an attacker (mtg-614 hole (b)).
    ///
    /// `CombatState::declare_attacker` mutates `combat.attackers` and sets
    /// `combat_active`; previously this was NOT undoable, so per-action undo
    /// could not reverse it and `rewind_to_turn_start` had to hand-clear the
    /// whole `CombatState`. Logging this action makes the declaration reversible
    /// (`undo()` calls `CombatState::undo_declare_attacker`), restoring the prior
    /// `combat_active` flag.
    DeclareAttacker {
        card_id: CardId,
        /// Value of `combat.combat_active` BEFORE this declaration (for undo).
        prev_combat_active: bool,
    },

    /// Declare a creature as a blocker (mtg-614 hole (b)).
    ///
    /// `CombatState::declare_blocker` inserts into `combat.blockers` and pushes
    /// into each attacker's `attacker_blockers` reverse map. Logging this action
    /// makes it reversible (`undo()` calls `CombatState::undo_declare_blocker`).
    /// The attacker list is stored so undo can prune the exact reverse-map
    /// entries it added.
    DeclareBlocker {
        blocker_id: CardId,
        attackers: SmallVec<[CardId; 2]>,
    },

    /// Clear the whole `CombatState` at end of combat (mtg-614 hole (b)).
    ///
    /// `end_combat_step` calls `combat.clear()`, discarding the attacker/blocker
    /// declarations. Without logging, a rewind across the end-of-combat boundary
    /// could not restore those declarations. Storing the previous `CombatState`
    /// lets `undo()` put it back exactly. Boxed to keep the `GameAction` enum
    /// small (avoids bloating every variant with the combat maps).
    ClearCombat {
        prev: Box<crate::game::combat::CombatState>,
    },

    /// Set a card's temporary base power/toughness override (mtg-614 hole (c)).
    ///
    /// `Card::set_temp_base_power` / `set_temp_base_toughness` (Animate Dead,
    /// characteristic-defining / base-setting effects, manlands) and
    /// `clear_temp_base_stats` previously had NO undo support — `undo.rs`'s own
    /// comment admitted this. Storing the previous `Option<i8>` values lets
    /// `undo()` restore them, so an Animate/base-set/clear round-trips exactly.
    SetTempBaseStats {
        card_id: CardId,
        prev_power: Option<i8>,
        prev_toughness: Option<i8>,
    },
    /// Records an until-end-of-turn `AB$ Animate` typeline mutation (Mishra's
    /// Factory et al. becoming a creature, robots-deck animated artifacts) so
    /// `undo()` restores the card's `types` / `subtypes` and the three
    /// animate-tracking vectors EXACTLY, then refreshes the definition cache.
    ///
    /// Animate mutates `types`/`subtypes` directly plus the
    /// `temp_animate_types` / `temp_animate_subtypes` / `temp_removed_subtypes`
    /// bookkeeping, none of which were undo-logged. The end-of-turn cleanup
    /// reverts them, but a rewind+replay (network shadow / MCTS) needs an exact
    /// inverse keyed by the undo log — relying on the bookkeeping vectors alone
    /// is fragile because the animate re-application guard
    /// (`if !card.subtypes.contains(st)`) can leave them out of sync with
    /// `subtypes` after a rewind. Capturing the full prior snapshot makes the
    /// round-trip exact (mtg-610).
    AnimateTypeline {
        card_id: CardId,
        prev_types: SmallVec<[crate::core::CardType; 2]>,
        prev_subtypes: SmallVec<[crate::core::Subtype; 3]>,
        prev_temp_animate_types: SmallVec<[crate::core::CardType; 2]>,
        prev_temp_animate_subtypes: SmallVec<[crate::core::Subtype; 2]>,
        prev_temp_removed_subtypes: SmallVec<[crate::core::Subtype; 2]>,
        /// Keywords the animate ACTUALLY inserted (those not already present on
        /// the card), so `undo()` removes exactly them. An animate like
        /// Soulstone Sanctuary ("becomes a 3/3 creature with vigilance") grants
        /// Vigilance until end of turn; without recording it here the keyword
        /// rode along through a rewind+replay (the keyword bit was re-granted on
        /// replay but never removed by the rewind), making the turn-start
        /// `keywords` history-dependent and breaking the round-trip (mtg-610).
        /// Only the keywords this animate newly added are listed (the insert
        /// site dedups against the existing set), so undo never strips a keyword
        /// the card printed or gained from another source.
        granted_keywords: SmallVec<[crate::core::Keyword; 2]>,
    },
    /// Records that `source` was appended to `target`'s `damaged_by_this_turn`
    /// list when combat (or other) damage was dealt. `undo()` pops that exact
    /// source so the until-end-of-turn damage-source tracking (read by death
    /// triggers like Sengir Vampire, CR 514.2) round-trips through a rewind.
    /// Only logged for the entry that was ACTUALLY added (the dedup guard in
    /// the marking site means a no-op push is never logged), so undo simply
    /// removes the last occurrence of `source` (mtg-610).
    MarkDamagedBy { target: CardId, source: CardId },

    /// Records that `card`'s `dealt_damage_to_opponent_this_turn` flag was set
    /// to `true` when it dealt combat damage to an opponent (Whirling Dervish's
    /// end-step intervening-if, CR 603.4 / CR 514.2). `prev` is the flag value
    /// before the set, so `undo()` restores it exactly. Logged UNCONDITIONALLY
    /// for every attacker that dealt damage to an opponent (not guarded on the
    /// live value), so the undo-log length is identical on a forward server pass
    /// and a WASM/native rewind+replay pass — a guard would desync the action
    /// count (mirrors the `SetCommanderDamage` old/new pattern).
    MarkDealtDamageToOpponent { card: CardId, prev: bool },

    /// Records that `card`'s `attacked_this_turn` flag was set to `true` when it
    /// was declared as an attacker (Berserk's end-step destroy intervening-if,
    /// CR 603.4 / CR 514.2). `prev` is the flag value before the set, so
    /// `undo()` restores it exactly. Logged UNCONDITIONALLY for every declared
    /// attacker (not guarded on the live value), so the undo-log length is
    /// identical on a forward server pass and a WASM/native rewind+replay pass —
    /// a guard would desync the action count (same contract as
    /// `MarkDealtDamageToOpponent`).
    MarkAttackedThisTurn { card: CardId, prev: bool },

    /// Records a Clone copy-transformation (`GameState::apply_clone`, CR 707.2:
    /// Copy Artifact, Clone, Vesuvan Doppelganger, ...) so a rewind+replay can
    /// reverse it exactly. `apply_clone` overwrites ~15 copiable characteristics
    /// of the cloning permanent IN PLACE with no undo support, so a rewind left
    /// the card stuck as the copied permanent (mtg-559/mtg-610: robots42's Copy
    /// Artifact stayed Mishra's Factory across rewinds, making the turn-start
    /// hash history-dependent). `undo()` restores the captured prior state.
    /// Boxed to keep the `GameAction` enum small.
    CloneCard {
        card_id: CardId,
        prev: Box<crate::core::CardCopiableState>,
    },

    /// Records that `player` was appended to the `extra_turns` queue by an
    /// `AddTurn` effect (Time Walk, Temporal Manipulation, ...; CR 500.7).
    /// `undo()` pops it back off the BACK of the queue. The push happens during
    /// a turn (when the extra-turn spell resolves), so a rewind+replay that
    /// unwinds past the resolution must remove the queued entry — otherwise it
    /// rode through the rewind and replay re-pushed it, making the turn-start
    /// `extra_turns` history-dependent (mtg-559/mtg-610: robots42 extra_turns[0]
    /// drift). The pop_front that CONSUMES the queue happens AT the ChangeTurn
    /// boundary, which `rewind_to_turn_start` stops at without undoing, so only
    /// the push needs logging for the rewind path; logging it also keeps the
    /// per-action undo oracle exact.
    PushExtraTurn { player: PlayerId },

    /// Set or clear the `Player::skip_untap_next_turn` flag.
    ///
    /// Set by `Effect::SkipUntapStep` (Yosei die trigger) and cleared at the
    /// start of `untap_step` when the flag is consumed.
    SetSkipUntapNextTurn {
        player_id: PlayerId,
        /// Value the field held BEFORE this action (for undo).
        old_value: bool,
        /// Value the field was SET TO by this action.
        new_value: bool,
    },

    /// Records that a brand-new entity (a token, via `Effect::CreateToken`) was
    /// minted: `next_card_id()` advanced `next_entity_id`, the instance was
    /// inserted into the EntityStore, and it was added to the battlefield — all
    /// previously UNLOGGED. On a rewind the token LEAKED (stayed in `cards` +
    /// `battlefield`) AND `next_entity_id` stayed advanced, so a forward replay
    /// minted a SECOND token at a higher id → duplicate token + diverged
    /// (hashed) state (mtg-ba6uq #3). `undo()` removes the card from the
    /// battlefield, clears it from the store, and rolls `next_entity_id` back to
    /// `card_id` (LIFO undo restores the exact counter the mint consumed).
    CreateEntity { card_id: CardId },

    /// Snapshot of a card's FULL counter set captured BEFORE an `add_counters`
    /// that may trigger +1/+1 ⟷ -1/-1 annihilation (CR 122.3). The plain
    /// `AddCounter { type, amount }` reversal cannot restore counters that
    /// annihilation CANCELLED: adding one -1/-1 to a card holding one +1/+1
    /// removes BOTH, but `AddCounter`'s undo only does `remove_counter(-1/-1, 1)`
    /// — which removes the (now-absent) -1/-1 and leaves the +1/+1 PERMANENTLY
    /// lost (mtg-ba6uq #4). Restoring this captured snapshot reverses the net
    /// change exactly, annihilation or not. `counters` is a tiny inline SmallVec
    /// so the snapshot is allocation-free in the common case.
    SetCardCounters {
        card_id: CardId,
        prev_counters: SmallVec<[(CounterType, u8); 2]>,
    },

    /// Snapshot of a player's library order (and, for surveil, graveyard order)
    /// captured BEFORE a deterministic reorder that mutates the zone Vec(s) with
    /// RAW ops rather than logged `MoveCard`s: scry (reorder top + put-on-bottom),
    /// surveil (reorder top + mill to graveyard), and Dig "rest to bottom". None
    /// of these were undo-logged, so a mid-turn rewind left the library in its
    /// reordered state and a replay re-derived a DIFFERENT order (the
    /// rewind-verifier hash diverged; MCTS per-action undo desynced). The NETWORK
    /// hash excludes library order (hidden info), so this is NOT a cross-machine
    /// desync — but it is a real undo-log hole (mtg-ba6uq #2). `undo()` restores
    /// the captured order(s). Mirrors `ShuffleLibrary`'s `previous_order` restore.
    /// `previous_graveyard` is `Some` only for surveil (which also mills).
    ReorderLibrary {
        player: PlayerId,
        previous_order: Vec<CardId>,
        previous_graveyard: Option<Vec<CardId>>,
    },

    /// Records a regeneration-shield application (CR 701.15a:
    /// `GameState::apply_regeneration_shield`) so the per-action undo path can
    /// reverse it. Applying a shield consumes one `regeneration_shields`, clears
    /// the creature's `damage`, and removes it from combat — three mutations
    /// that were NOT undo-logged (only the tap was, via `tap_permanent`). A
    /// turn-start rewind is safe (it blanket-clears combat + damage + shields),
    /// but the per-action UndoTest / human / MCTS undo desynced (mtg-ba6uq #5).
    /// `undo()` restores all three; the tap is undone by its own `TapCard`.
    /// `prev_combat` is boxed to keep `GameAction` small (mirrors `ClearCombat`).
    RegenerateReplaceDestroy {
        card_id: CardId,
        prev_shields: u8,
        prev_damage: i32,
        prev_combat: Box<crate::game::combat::CombatState>,
    },

    /// Snapshot of a player's `source_prevention_shields` (Circle of Protection
    /// style colored/source-filtered damage-prevention shields, CR 615) captured
    /// BEFORE a mutation: installing a shield (`Effect::PreventDamageFromSource`)
    /// or consuming/retiring spent shields (`apply_source_prevention_shields`).
    /// Neither was undo-logged; a turn-start rewind blanket-clears the list
    /// (safe), but the per-action UndoTest / human / MCTS undo desynced
    /// (mtg-ba6uq #6). `undo()` restores the captured list.
    SetSourcePreventionShields {
        player_id: PlayerId,
        prev: Vec<crate::core::DamagePreventionShield>,
    },

    /// Snapshot of a player's `combat_mana_pool` (Avatar Firebending combat-only
    /// mana, CR 701.65) captured BEFORE a mutation: adding combat mana, spending
    /// it via `pay_from_total_mana`, or emptying it at end of combat. None of
    /// these were undo-logged; a turn-start rewind now blanket-clears it (it is a
    /// per-combat transient, None at any turn boundary), but the per-action
    /// UndoTest / human / MCTS undo desynced (mtg-ba6uq #7). `undo()` restores
    /// the captured pool. `ManaPool` is `Copy`, so the snapshot is cheap.
    SetCombatManaPool {
        player_id: PlayerId,
        prev: Option<crate::core::ManaPool>,
    },

    /// Snapshot of a player's regular `mana_pool` captured BEFORE a payment
    /// (`pay_from_total_mana` / `ManaPool::pay_cost`) consumed it (mtg-733).
    /// These spends mutate the pool with no other covering action. The pool
    /// empties at every step boundary (CR 500.4), so a turn-start rewind always
    /// lands on `mana_pool == 0` and was already safe; but a PARTIAL (per-action
    /// MCTS / human / UndoTest) rewind stopping BETWEEN an `AddMana` and its
    /// consuming payment would observe the wrong pool. `undo()` restores the
    /// captured pool. `ManaPool` is `Copy`, so the snapshot is cheap. Mirrors
    /// `SetCombatManaPool` (mtg-ba6uq #7).
    SetManaPool {
        player_id: PlayerId,
        prev: crate::core::ManaPool,
    },

    /// Snapshot of a creature's marked `damage` captured BEFORE a `card.damage +=`
    /// mutation (`deal_damage_to_creature` — e.g. Triskelion's ping — and
    /// `Effect::DamageAll`) (mtg-728 sig-2f). Marked damage was applied with no
    /// covering GameAction, so a turn-start cleanup (CR 514.2) clears it and any
    /// blanket turn-start rewind blanket-clears it (safe), BUT a mid-turn
    /// rewind+replay (network/WASM blocking; per-action MCTS/human undo) left the
    /// marked damage STALE and replay DOUBLE-applied it — the robots42 within-side
    /// "cards[N].damage changed across rewinds" REWIND/REPLAY FATAL. `undo()`
    /// restores the captured value. `get_mut` tolerates a missing card (it may
    /// have left the battlefield), matching the other card-field undos.
    SetDamage { card_id: CardId, prev: i32 },

    /// Snapshot of a card's `x_paid` (the X value chosen when an X-spell/ability
    /// was cast/activated, CR 107.3) captured BEFORE it is overwritten in the
    /// priority loop (mtg-728 sig-2g). `x_paid` is set with no covering
    /// GameAction, so a mid-turn rewind+replay (network/WASM blocking; per-action
    /// MCTS/human undo) left the chosen X STALE on the card — the robots42
    /// within-side "cards[N].x_paid changed across rewinds" REWIND/REPLAY FATAL
    /// (the residual after sig-2f, seeds 1 & 7). `undo()` restores the captured
    /// value. `get_mut` tolerates a missing card, matching the other card-field
    /// undos.
    SetXPaid { card_id: CardId, prev: u8 },

    /// Snapshot of a card's `times_kicked` (number of times Multikicker was paid,
    /// CR 702.33a) captured BEFORE it is overwritten in the priority loop. Mirrors
    /// `SetXPaid` — the same mid-turn rewind+replay hazard applies here.
    SetTimesKicked { card_id: CardId, prev: u8 },

    /// Snapshot of a card's `bargain_paid` flag (CR 702.162) captured BEFORE it is
    /// set in the priority loop. Mirrors `SetTimesKicked` — same rewind semantics.
    SetBargainPaid { card_id: CardId, prev: bool },

    /// Restore the full state snapshot of a card (e.g. when leaving battlefield
    /// the transient state is reset, and undoing it restores the snapshotted state).
    RestoreCardState {
        card_id: CardId,
        snapshot: Box<CardStateSnapshot>,
    },
}

impl fmt::Display for GameAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GameAction::MoveCard {
                card_id,
                from_zone,
                to_zone,
                owner,
                from_position,
            } => write!(
                f,
                "MoveCard({} {:?}@{:?} -> {:?} owner=P{})",
                card_id.as_u32(),
                from_zone,
                from_position,
                to_zone,
                owner.as_u32()
            ),
            GameAction::TapCard { card_id, tapped } => {
                if *tapped {
                    write!(f, "Tap({})", card_id.as_u32())
                } else {
                    write!(f, "Untap({})", card_id.as_u32())
                }
            }
            GameAction::SetChosenColor { card_id, prev } => {
                write!(f, "SetChosenColor({}, prev={:?})", card_id.as_u32(), prev)
            }
            GameAction::SetChosenPlayer { card_id, prev } => {
                write!(
                    f,
                    "SetChosenPlayer({}, prev={:?})",
                    card_id.as_u32(),
                    prev.map(|p| p.as_u32())
                )
            }
            GameAction::SetChosenMode { card_id, prev } => {
                write!(f, "SetChosenMode({}, prev={:?})", card_id.as_u32(), prev)
            }
            GameAction::ModifyLife { player_id, delta } => {
                write!(f, "Life(P{} {:+})", player_id.as_u32(), delta)
            }
            GameAction::AddMana { player_id, mana } => {
                write!(f, "AddMana(P{} {})", player_id.as_u32(), mana)
            }
            GameAction::EmptyManaPool { player_id, .. } => {
                write!(f, "EmptyMana(P{})", player_id.as_u32())
            }
            GameAction::AddCounter {
                card_id,
                counter_type,
                amount,
            } => write!(f, "AddCounter({} {:?}x{})", card_id.as_u32(), counter_type, amount),
            GameAction::RemoveCounter {
                card_id,
                counter_type,
                amount,
            } => write!(f, "RemoveCounter({} {:?}x{})", card_id.as_u32(), counter_type, amount),
            GameAction::AdvanceStep { from_step, to_step } => {
                write!(f, "Step({:?} -> {:?})", from_step, to_step)
            }
            GameAction::ChangeTurn {
                from_player,
                to_player,
                turn_number,
                ..
            } => write!(
                f,
                "Turn({} P{} -> P{})",
                turn_number,
                from_player.as_u32(),
                to_player.as_u32()
            ),
            GameAction::PumpCreature {
                card_id,
                power_delta,
                toughness_delta,
                keywords_granted,
            } => {
                if keywords_granted.is_empty() {
                    write!(f, "Pump({} {:+}/{:+})", card_id.as_u32(), power_delta, toughness_delta)
                } else {
                    write!(
                        f,
                        "Pump({} {:+}/{:+} +{:?})",
                        card_id.as_u32(),
                        power_delta,
                        toughness_delta,
                        keywords_granted
                    )
                }
            }
            GameAction::DebuffCreature {
                card_id,
                keywords_removed,
            } => {
                write!(f, "Debuff({} -{:?})", card_id.as_u32(), keywords_removed)
            }
            GameAction::SetTurnEnteredBattlefield { card_id, new_value, .. } => {
                write!(f, "SetETB({} turn={:?})", card_id.as_u32(), new_value)
            }
            GameAction::SetLandsPlayedThisTurn {
                player_id, new_value, ..
            } => write!(f, "LandsPlayed(P{} = {})", player_id.as_u32(), new_value),
            GameAction::SetCardsDrawnThisTurn {
                player_id, new_value, ..
            } => write!(f, "CardsDrawn(P{} = {})", player_id.as_u32(), new_value),
            GameAction::SetSpellsCastThisTurn {
                player_id, new_value, ..
            } => write!(f, "SpellsCast(P{} = {})", player_id.as_u32(), new_value),
            GameAction::ChangeController {
                card_id,
                new_controller,
                ..
            } => write!(f, "ChangeCtrl({} -> P{})", card_id.as_u32(), new_controller.as_u32()),
            GameAction::SetAttachedTo {
                equipment_id,
                new_target,
                ..
            } => write!(f, "Attach({} -> {:?})", equipment_id.as_u32(), new_target),
            GameAction::ChoicePoint {
                player_id,
                choice_id,
                choice,
            } => write!(f, "Choice(P{} #{} = {:?})", player_id.as_u32(), choice_id, choice),
            GameAction::RevealCard {
                card_id,
                name,
                revealed_to,
                old_mask,
            } => {
                let target = match revealed_to {
                    RevealTarget::Player(pid) => format!("P{}", pid.as_u32()),
                    RevealTarget::All => "ALL".to_string(),
                };
                match name {
                    Some(n) => write!(
                        f,
                        "RevealCard({} = \"{}\" to {} mask:0x{:02x})",
                        card_id.as_u32(),
                        n,
                        target,
                        old_mask
                    ),
                    None => write!(
                        f,
                        "RevealCard({} = ??? to {} mask:0x{:02x})",
                        card_id.as_u32(),
                        target,
                        old_mask
                    ),
                }
            }
            GameAction::SetRevealedToMask {
                card_id,
                old_value,
                new_value,
            } => write!(
                f,
                "SetRevealedMask({} 0x{:02x} -> 0x{:02x})",
                card_id.as_u32(),
                old_value,
                new_value
            ),
            GameAction::ShuffleLibrary {
                player, previous_order, ..
            } => {
                write!(f, "ShuffleLibrary(P{} {} cards)", player.as_u32(), previous_order.len())
            }
            GameAction::SetLoyaltyActivated { card_id, new_value, .. } => {
                write!(f, "SetLoyaltyActivated({} = {})", card_id.as_u32(), new_value)
            }
            GameAction::SetCommanderCastCount {
                player_id, new_value, ..
            } => write!(f, "CmdrCastCount(P{} = {})", player_id.as_u32(), new_value),
            GameAction::SetCommanderDamage {
                player_id,
                from_player,
                new_damage,
                ..
            } => write!(
                f,
                "CmdrDmg(P{} from P{} = {})",
                player_id.as_u32(),
                from_player.as_u32(),
                new_damage
            ),
            GameAction::RegisterDelayedTrigger { id } => {
                write!(f, "RegisterDelayedTrigger(#{})", id.as_u32())
            }
            GameAction::FireDelayedTrigger { trigger } => {
                write!(f, "FireDelayedTrigger(#{})", trigger.id.as_u32())
            }
            GameAction::SetRememberedAmount { previous } => {
                write!(f, "SetRememberedAmount(prev={:?})", previous)
            }
            GameAction::DeclareAttacker {
                card_id,
                prev_combat_active,
            } => write!(
                f,
                "DeclareAttacker({} prev_active={})",
                card_id.as_u32(),
                prev_combat_active
            ),
            GameAction::DeclareBlocker { blocker_id, attackers } => write!(
                f,
                "DeclareBlocker({} blocking {} attacker(s))",
                blocker_id.as_u32(),
                attackers.len()
            ),
            GameAction::ClearCombat { prev } => {
                write!(f, "ClearCombat(restored {} attacker(s))", prev.attackers.len())
            }
            GameAction::SetTempBaseStats {
                card_id,
                prev_power,
                prev_toughness,
            } => write!(
                f,
                "SetTempBaseStats({} prev={:?}/{:?})",
                card_id.as_u32(),
                prev_power,
                prev_toughness
            ),
            GameAction::AnimateTypeline {
                card_id, prev_subtypes, ..
            } => write!(
                f,
                "AnimateTypeline({} prev_subtypes={})",
                card_id.as_u32(),
                prev_subtypes.len()
            ),
            GameAction::MarkDamagedBy { target, source } => {
                write!(
                    f,
                    "MarkDamagedBy(target={} source={})",
                    target.as_u32(),
                    source.as_u32()
                )
            }
            GameAction::MarkDealtDamageToOpponent { card, prev } => {
                write!(f, "MarkDealtDamageToOpponent(card={} prev={})", card.as_u32(), prev)
            }
            GameAction::MarkAttackedThisTurn { card, prev } => {
                write!(f, "MarkAttackedThisTurn(card={} prev={})", card.as_u32(), prev)
            }
            GameAction::CloneCard { card_id, prev } => {
                write!(f, "CloneCard({} prev_name={})", card_id.as_u32(), prev.name.as_str())
            }
            GameAction::PushExtraTurn { player } => {
                write!(f, "PushExtraTurn(P{})", player.as_u32())
            }
            GameAction::SetSkipUntapNextTurn {
                player_id,
                old_value,
                new_value,
            } => write!(
                f,
                "SetSkipUntapNextTurn(P{} {} -> {})",
                player_id.as_u32(),
                old_value,
                new_value
            ),
            GameAction::CreateEntity { card_id } => {
                write!(f, "CreateEntity(card={})", card_id.as_u32())
            }
            GameAction::SetCardCounters { card_id, prev_counters } => {
                write!(
                    f,
                    "SetCardCounters(card={}, prev={} types)",
                    card_id.as_u32(),
                    prev_counters.len()
                )
            }
            GameAction::ReorderLibrary {
                player,
                previous_order,
                previous_graveyard,
            } => {
                write!(
                    f,
                    "ReorderLibrary(P{}, lib={}{})",
                    player.as_u32(),
                    previous_order.len(),
                    match previous_graveyard {
                        Some(g) => format!(", gy={}", g.len()),
                        None => String::new(),
                    }
                )
            }
            GameAction::RegenerateReplaceDestroy {
                card_id,
                prev_shields,
                prev_damage,
                ..
            } => {
                write!(
                    f,
                    "RegenerateReplaceDestroy(card={}, shields={}, dmg={})",
                    card_id.as_u32(),
                    prev_shields,
                    prev_damage
                )
            }
            GameAction::SetSourcePreventionShields { player_id, prev } => {
                write!(
                    f,
                    "SetSourcePreventionShields(P{}, prev={})",
                    player_id.as_u32(),
                    prev.len()
                )
            }
            GameAction::SetDamage { card_id, prev } => {
                write!(f, "SetDamage(card={}, prev={})", card_id, prev)
            }
            GameAction::SetXPaid { card_id, prev } => {
                write!(f, "SetXPaid(card={}, prev={})", card_id, prev)
            }
            GameAction::SetTimesKicked { card_id, prev } => {
                write!(f, "SetTimesKicked(card={}, prev={})", card_id, prev)
            }
            GameAction::SetBargainPaid { card_id, prev } => {
                write!(f, "SetBargainPaid(card={}, prev={})", card_id, prev)
            }
            GameAction::SetManaPool { player_id, prev } => {
                write!(f, "SetManaPool(P{}, prev={})", player_id.as_u32(), prev.total())
            }
            GameAction::SetCombatManaPool { player_id, prev } => {
                write!(
                    f,
                    "SetCombatManaPool(P{}, prev={})",
                    player_id.as_u32(),
                    if prev.is_some() { "Some" } else { "None" }
                )
            }
            GameAction::RestoreCardState { card_id, .. } => {
                write!(f, "RestoreCardState(card={})", card_id.as_u32())
            }
        }
    }
}

impl GameAction {
    /// Apply the inverse of this action to undo it
    ///
    /// Returns Ok(()) if successful, Err if the action cannot be undone
    ///
    /// # Errors
    ///
    /// Returns an error string if the action cannot be undone (e.g., card/player not found).
    pub fn undo(&self, game: &mut GameState) -> Result<(), String> {
        match self {
            GameAction::MoveCard {
                card_id,
                from_zone,
                to_zone,
                owner,
                from_position,
            } => {
                // Reverse the move: move from to_zone back to from_zone.
                // Done with raw zone ops (NOT `game.move_card`) so we can
                // reinsert the card at its original `from_position` and
                // skip the forward-direction logging/reveal side effects.
                use crate::zones::Zone;
                let removed = match to_zone {
                    Zone::Battlefield => game.battlefield.remove(*card_id),
                    Zone::Stack => game.stack.remove(*card_id),
                    Zone::Library | Zone::Hand | Zone::Graveyard | Zone::Exile | Zone::Command => {
                        if let Some(zones) = game.get_player_zones_mut(*owner) {
                            if let Some(zone) = zones.get_zone_mut(*to_zone) {
                                zone.remove(*card_id)
                            } else {
                                false
                            }
                        } else {
                            false
                        }
                    }
                };

                if !removed {
                    // Some forward `move_card` calls log a MoveCard even
                    // when the source/destination removal silently no-ops
                    // (e.g. shadow-mode tolerance). Don't fail undo for
                    // those; just nothing to put back.
                    return Ok(());
                }

                // Reinsert at the original `from_position` when known so
                // Hand/Library/Stack order is preserved across the undo
                // cycle. Without this, `add()` appends to the end and the
                // WASM rewind/replay verifier sees a "hand reordering"
                // drift (root cause of `bug-rewind-infinite-loop`'s
                // turn-start hash divergence).
                let pos = from_position.as_ref().map(|p| *p as usize);
                match from_zone {
                    Zone::Battlefield => match pos {
                        Some(p) => game.battlefield.add_at(*card_id, p),
                        None => game.battlefield.add(*card_id),
                    },
                    Zone::Stack => match pos {
                        Some(p) => game.stack.add_at(*card_id, p),
                        None => game.stack.add(*card_id),
                    },
                    Zone::Library | Zone::Hand | Zone::Graveyard | Zone::Exile | Zone::Command => {
                        if let Some(zones) = game.get_player_zones_mut(*owner) {
                            if let Some(zone) = zones.get_zone_mut(*from_zone) {
                                match pos {
                                    Some(p) => zone.add_at(*card_id, p),
                                    None => zone.add(*card_id),
                                }
                            }
                        }
                    }
                }
            }

            GameAction::TapCard { card_id, tapped } => {
                // Reverse tap state
                if let Ok(card) = game.cards.get_mut(*card_id) {
                    card.tapped = !tapped;
                    // Increment mana version since tap state changed
                    game.increment_mana_version();
                } else {
                    return Err(format!("Card {} not found for TapCard undo", card_id.as_u32()));
                }
            }

            GameAction::SetChosenColor { card_id, prev } => {
                // Restore the previous ETB-chosen color (mtg-ba6uq #1).
                if let Ok(card) = game.cards.get_mut(*card_id) {
                    card.chosen_color = *prev;
                } else {
                    return Err(format!("Card {} not found for SetChosenColor undo", card_id.as_u32()));
                }
            }

            GameAction::SetChosenPlayer { card_id, prev } => {
                // Restore the previous ETB-chosen player (mtg-ba6uq #1).
                if let Ok(card) = game.cards.get_mut(*card_id) {
                    card.chosen_player = *prev;
                } else {
                    return Err(format!("Card {} not found for SetChosenPlayer undo", card_id.as_u32()));
                }
            }

            GameAction::SetChosenMode { card_id, prev } => {
                // Restore the previous ETB-chosen mode (Palace Siege Khans/Dragons).
                if let Ok(card) = game.cards.get_mut(*card_id) {
                    card.chosen_mode = prev.clone();
                } else {
                    return Err(format!("Card {} not found for SetChosenMode undo", card_id.as_u32()));
                }
            }

            GameAction::ModifyLife { player_id, delta } => {
                // Reverse the life change
                if let Some(player) = game.players.iter_mut().find(|p| p.id == *player_id) {
                    player.life -= delta;
                    // `has_lost` is a DERIVED flag set as a side effect of
                    // `lose_life()` when life drops to <= 0 (player.rs); it is
                    // not separately undo-logged. Reversing the life delta must
                    // also reverse the derived loss flag, otherwise a rewind
                    // that restores life back above 0 leaves a stale
                    // `has_lost = true`, diverging the turn-start hash across
                    // rewinds (mtg-610). This mirrors the redo path in
                    // state.rs (apply ModifyLife re-derives has_lost). Only the
                    // life-based loss condition is re-derived here; other loss
                    // conditions (empty-library draw, etc.) are recorded by
                    // their own logged actions and replayed forward.
                    if player.life > 0 {
                        player.has_lost = false;
                    }
                } else {
                    return Err(format!("Player {} not found for ModifyLife undo", player_id.as_u32()));
                }
            }

            GameAction::AddMana { player_id, mana } => {
                // Remove the mana that was added
                if let Some(player) = game.players.iter_mut().find(|p| p.id == *player_id) {
                    player.mana_pool.white = player.mana_pool.white.saturating_sub(mana.white);
                    player.mana_pool.blue = player.mana_pool.blue.saturating_sub(mana.blue);
                    player.mana_pool.black = player.mana_pool.black.saturating_sub(mana.black);
                    player.mana_pool.red = player.mana_pool.red.saturating_sub(mana.red);
                    player.mana_pool.green = player.mana_pool.green.saturating_sub(mana.green);
                    player.mana_pool.colorless = player.mana_pool.colorless.saturating_sub(mana.colorless);
                } else {
                    return Err(format!("Player {} not found for AddMana undo", player_id.as_u32()));
                }
            }

            GameAction::EmptyManaPool {
                player_id,
                prev_white,
                prev_blue,
                prev_black,
                prev_red,
                prev_green,
                prev_colorless,
            } => {
                // Restore previous mana pool state
                if let Some(player) = game.players.iter_mut().find(|p| p.id == *player_id) {
                    player.mana_pool.white = *prev_white;
                    player.mana_pool.blue = *prev_blue;
                    player.mana_pool.black = *prev_black;
                    player.mana_pool.red = *prev_red;
                    player.mana_pool.green = *prev_green;
                    player.mana_pool.colorless = *prev_colorless;
                } else {
                    return Err(format!(
                        "Player {} not found for EmptyManaPool undo",
                        player_id.as_u32()
                    ));
                }
            }

            GameAction::AddCounter {
                card_id,
                counter_type,
                amount,
            } => {
                // Remove the counters that were added. Use the direct card
                // mutator, NOT `game.remove_counters`: the latter LOGS a
                // RemoveCounter GameAction, which would pollute the live undo
                // log on the per-action undo path (an undo must never re-log).
                if let Some(card) = game.cards.try_get_mut(*card_id) {
                    card.remove_counter(*counter_type, *amount);
                } else {
                    return Err(format!("Card {} not found for AddCounter undo", card_id.as_u32()));
                }
            }

            GameAction::RemoveCounter {
                card_id,
                counter_type,
                amount,
            } => {
                // Add back the counters that were removed. Use the direct card
                // mutator, NOT `game.add_counters` (which would re-log an
                // AddCounter GameAction — an undo must never re-log).
                if let Some(card) = game.cards.try_get_mut(*card_id) {
                    card.add_counter(*counter_type, *amount);
                } else {
                    return Err(format!("Card {} not found for RemoveCounter undo", card_id.as_u32()));
                }
            }

            GameAction::AdvanceStep { from_step, to_step: _ } => {
                // Restore previous step
                game.turn.current_step = *from_step;
            }

            GameAction::ChangeTurn {
                from_player,
                to_player: _,
                turn_number,
                rng_state,
            } => {
                // Restore previous turn state.
                game.turn.active_player = *from_player;
                // NOTE: do NOT touch `active_player_idx` here. The forward turn
                // machinery (`TurnStructure::next_turn`) updates `active_player`
                // but NEVER `active_player_idx` — that field is set once at game
                // construction (the starting seat) and left constant thereafter.
                // "Restoring" it from `from_player`'s position would write a
                // value the forward game never produces, diverging the per-action
                // undo hash (mtg-732: this was a silent divergence between the
                // two undo impls — the old GameState::undo correctly left it
                // alone). `rewind_to_turn_start` never reaches this arm (it stops
                // AT the ChangeTurn boundary without undoing it).

                // Restore turn number to the previous turn.
                // ChangeTurn logs the NEW turn number, so previous is turn_number - 1.
                // EXCEPTION (mtg-610): the turn-1 start boundary marker
                // (`GameState::ensure_turn_one_boundary`) logs turn_number == 1.
                // There is no turn 0 — undoing past the start of turn 1 lands at
                // the game's initial turn, which is 1 — so keep it at 1 rather
                // than `saturating_sub(1)`-ing to 0.
                game.turn.turn_number = if *turn_number <= 1 { 1 } else { turn_number - 1 };

                // Restore RNG state if available (using bincode + SmallVec)
                if let Some(rng_bytes) = rng_state {
                    // SmallVec derefs to &[u8], which is what bincode::deserialize expects
                    if let Ok(rng) = bincode::deserialize::<rand_chacha::ChaCha12Rng>(rng_bytes) {
                        *game.rng.borrow_mut() = rng;
                    } else {
                        return Err("Failed to deserialize RNG state".to_string());
                    }
                }
            }

            GameAction::PumpCreature {
                card_id,
                power_delta,
                toughness_delta,
                keywords_granted,
            } => {
                // Reverse the pump by applying negative deltas
                if let Ok(card) = game.cards.get_mut(*card_id) {
                    // Reverse the power/toughness bonus
                    card.power_bonus -= power_delta;
                    card.toughness_bonus -= toughness_delta;
                    // Remove granted keywords from BOTH the live set and the
                    // until-EOT tracking set so the tracking field round-trips
                    // exactly across the per-action undo oracle (mtg-610).
                    for keyword in keywords_granted {
                        card.keywords.remove(*keyword);
                        card.temp_keywords_until_eot.remove(*keyword);
                    }
                } else {
                    return Err(format!("Card {} not found for PumpCreature undo", card_id.as_u32()));
                }
            }

            GameAction::DebuffCreature {
                card_id,
                keywords_removed,
            } => {
                // Reverse the debuff by re-adding the removed keywords
                if let Ok(card) = game.cards.get_mut(*card_id) {
                    for keyword in keywords_removed {
                        card.keywords.insert(*keyword);
                    }
                } else {
                    return Err(format!("Card {} not found for DebuffCreature undo", card_id.as_u32()));
                }
            }

            GameAction::SetTurnEnteredBattlefield {
                card_id,
                old_value,
                new_value: _,
            } => {
                // Restore the previous turn_entered_battlefield value
                if let Ok(card) = game.cards.get_mut(*card_id) {
                    card.turn_entered_battlefield = *old_value;
                } else {
                    return Err(format!(
                        "Card {} not found for SetTurnEnteredBattlefield undo",
                        card_id.as_u32()
                    ));
                }
            }

            GameAction::SetLandsPlayedThisTurn {
                player_id,
                old_value,
                new_value: _,
            } => {
                // Restore the previous lands_played_this_turn count
                if let Some(player) = game.players.iter_mut().find(|p| p.id == *player_id) {
                    player.lands_played_this_turn = *old_value;
                } else {
                    return Err(format!(
                        "Player {} not found for SetLandsPlayedThisTurn undo",
                        player_id.as_u32()
                    ));
                }
            }

            GameAction::SetCardsDrawnThisTurn {
                player_id,
                old_value,
                new_value: _,
            } => {
                // Restore the previous cards_drawn_this_turn count
                if let Some(player) = game.players.iter_mut().find(|p| p.id == *player_id) {
                    player.cards_drawn_this_turn = *old_value;
                } else {
                    return Err(format!(
                        "Player {} not found for SetCardsDrawnThisTurn undo",
                        player_id.as_u32()
                    ));
                }
            }

            GameAction::SetSpellsCastThisTurn {
                player_id,
                old_value,
                new_value: _,
            } => {
                // Restore the previous spells_cast_this_turn count
                if let Some(player) = game.players.iter_mut().find(|p| p.id == *player_id) {
                    player.spells_cast_this_turn = *old_value;
                } else {
                    return Err(format!(
                        "Player {} not found for SetSpellsCastThisTurn undo",
                        player_id.as_u32()
                    ));
                }
            }

            GameAction::ChangeController {
                card_id,
                old_controller,
                new_controller: _,
            } => {
                // Restore the previous controller
                if let Ok(card) = game.cards.get_mut(*card_id) {
                    card.controller = *old_controller;
                } else {
                    return Err(format!("Card {} not found for ChangeController undo", card_id.as_u32()));
                }
            }

            GameAction::SetAttachedTo {
                equipment_id,
                old_target,
                new_target: _,
            } => {
                // Restore the previous attached_to value
                if let Ok(equipment) = game.cards.get_mut(*equipment_id) {
                    equipment.attached_to = *old_target;
                } else {
                    return Err(format!(
                        "Equipment {} not found for SetAttachedTo undo",
                        equipment_id.as_u32()
                    ));
                }
            }

            GameAction::ChoicePoint { .. } => {
                // ChoicePoints don't modify game state, nothing to undo
            }

            GameAction::RevealCard {
                card_id,
                name,
                old_mask,
                ..
            } => {
                // Undo reveal: restore the previous mask state.
                //
                // Late-binding network reveals (mtg-610): a shadow client that
                // draws a card it does not yet know logs
                // `RevealCard{name:None, old_mask:0}` — the instance does NOT
                // exist at log time (that is precisely why `name` is None).
                // The card's identity arrives LATER via an asynchronous
                // state-sync `RevealCard` message, which `process_card_reveal`
                // applies by `game.cards.insert(...)` (NOT an undo-logged
                // action). On rewind we therefore find an instance present that
                // did not exist at this log point. Restoring only the mask
                // would LEAK that async-inserted instance into the rewound
                // turn-start state, making the undo log a non-faithful inverse
                // (the same turn-start rewound twice would hash differently,
                // because the second rewind retains the leaked instance). So
                // when undoing a late-binding reveal (`name` is None) we CLEAR
                // any instance present — its async insertion logically belongs
                // strictly after this point, and a forward replay re-applies
                // the state-sync reveal to re-instantiate it deterministically.
                let is_late_binding = name.is_none() && *old_mask == 0;
                if let Ok(card) = game.cards.get_mut(*card_id) {
                    if is_late_binding {
                        // Async-inserted-after-this-point instance; remove it so
                        // the rewound state matches the (instance-free) state at
                        // the moment this late-binding reveal was logged.
                        game.cards.clear(*card_id);
                    } else {
                        // Card existed at log time — restore the mask only.
                        card.revealed_to_mask = *old_mask;
                    }
                } else if *old_mask == 0 && name.is_some() {
                    // Card doesn't exist but was created by this (named) reveal.
                    game.cards.clear(*card_id);
                }
            }

            GameAction::SetRevealedToMask {
                card_id,
                old_value,
                new_value: _,
            } => {
                // Restore the previous revealed_to_mask value. A missing
                // instance is tolerated (Ok, no-op): on a viewer's shadow a
                // reserved (instance-less) opponent card carries a count-parity
                // SetRevealedToMask{0->0} with nothing to restore (mtg-mb668
                // sig-2d). Erroring here would spam rewind/undo warnings.
                if let Ok(card) = game.cards.get_mut(*card_id) {
                    card.revealed_to_mask = *old_value;
                }
            }

            GameAction::ShuffleLibrary {
                player,
                previous_order,
                rng_state,
            } => {
                // Restore the library to its previous order
                if let Some(zones) = game
                    .player_zones
                    .iter_mut()
                    .find(|(id, _)| *id == *player)
                    .map(|(_, z)| z)
                {
                    zones.library.cards = previous_order.clone();
                } else {
                    return Err(format!(
                        "Player {} zones not found for ShuffleLibrary undo",
                        player.as_u32()
                    ));
                }

                // Restore the pre-shuffle RNG state (mtg-728 sig-2) so a
                // partial-rewind replay re-runs the shuffle from the SAME RNG
                // and byte-reproduces the forward library order. Mirrors the
                // ChangeTurn arm above. `None` only for legacy logs.
                if let Some(rng_bytes) = rng_state {
                    if let Ok(rng) = bincode::deserialize::<rand_chacha::ChaCha12Rng>(rng_bytes) {
                        *game.rng.borrow_mut() = rng;
                    } else {
                        return Err("Failed to deserialize ShuffleLibrary RNG state".to_string());
                    }
                }
            }

            GameAction::SetLoyaltyActivated {
                card_id,
                old_value,
                new_value: _,
            } => {
                if let Ok(card) = game.cards.get_mut(*card_id) {
                    card.loyalty_activated_this_turn = *old_value;
                } else {
                    return Err(format!(
                        "Card {} not found for SetLoyaltyActivated undo",
                        card_id.as_u32()
                    ));
                }
            }

            GameAction::SetCommanderCastCount {
                player_id,
                old_value,
                new_value: _,
            } => {
                if let Some(player) = game.players.iter_mut().find(|p| p.id == *player_id) {
                    player.commander_cast_count = *old_value;
                } else {
                    return Err(format!(
                        "Player {} not found for SetCommanderCastCount undo",
                        player_id.as_u32()
                    ));
                }
            }

            GameAction::SetCommanderDamage {
                player_id,
                from_player,
                old_damage,
                new_damage: _,
            } => {
                if let Some(player) = game.players.iter_mut().find(|p| p.id == *player_id) {
                    if let Some(entry) = player
                        .commander_damage_taken
                        .iter_mut()
                        .find(|(pid, _)| *pid == *from_player)
                    {
                        entry.1 = *old_damage;
                    }
                    // If old_damage was 0 and there's no entry, the entry was added during the
                    // forward action - remove it on undo
                    if *old_damage == 0 {
                        player.commander_damage_taken.retain(|(pid, _)| *pid != *from_player);
                    }
                    // Re-derive the has_lost flag (mtg-ba6uq #8). The forward
                    // SetCommanderDamage sets `has_lost = true` when a single
                    // commander reaches 21 (CR 903.10a), but that derived flag is
                    // NOT separately undo-logged — so undoing the lethal damage
                    // left a stale `has_lost = true`, diverging the (hashed) state
                    // across a rewind. Mirrors ModifyLife.undo's life-based
                    // re-derivation: clear `has_lost` only when NO loss condition
                    // we can evaluate here still holds (life > 0 AND no single
                    // commander still at 21+). Other loss conditions
                    // (empty-library draw, etc.) are recorded by their own logged
                    // actions and replayed forward; 2-player commander ends the
                    // game so no further rewind, so this matters for 3+ player.
                    let still_lost_by_commander = player.commander_damage_taken.iter().any(|(_, d)| *d >= 21);
                    if player.life > 0 && !still_lost_by_commander {
                        player.has_lost = false;
                    }
                } else {
                    return Err(format!(
                        "Player {} not found for SetCommanderDamage undo",
                        player_id.as_u32()
                    ));
                }
            }

            GameAction::RegisterDelayedTrigger { id } => {
                // Undo registration: remove the trigger AND roll next_id back so
                // a replay re-add reuses the same id (stable state hash).
                game.delayed_triggers.undo_add(*id);
            }

            GameAction::FireDelayedTrigger { trigger } => {
                // Undo firing: restore the removed trigger. (The mana/effects
                // produced by firing are logged as their own actions and undone
                // separately.)
                game.delayed_triggers.restore((**trigger).clone());
            }

            GameAction::SetRememberedAmount { previous } => {
                game.remembered_amount = *previous;
            }

            GameAction::DeclareAttacker {
                card_id,
                prev_combat_active,
            } => {
                // Reverse the attacker declaration: remove from the attackers map
                // and restore the prior combat_active flag. (The tap and any
                // attack triggers are logged as their own actions and undone
                // separately.)
                game.combat.undo_declare_attacker(*card_id, *prev_combat_active);
            }

            GameAction::DeclareBlocker { blocker_id, attackers } => {
                // Reverse the blocker declaration: remove the blocker mapping and
                // prune the reverse attacker_blockers entries it added.
                game.combat.undo_declare_blocker(*blocker_id, attackers);
            }

            GameAction::ClearCombat { prev } => {
                // Restore the full CombatState that end_combat_step cleared.
                game.combat = (**prev).clone();
            }

            GameAction::SetTempBaseStats {
                card_id,
                prev_power,
                prev_toughness,
            } => {
                // Restore the previous temp base P/T overrides (Animate / base-set
                // / clear). get_mut tolerates a missing card (the card may have
                // left the battlefield) the same way other card-field undos do.
                if let Ok(card) = game.cards.get_mut(*card_id) {
                    card.restore_temp_base_stats(*prev_power, *prev_toughness);
                }
            }

            GameAction::AnimateTypeline {
                card_id,
                prev_types,
                prev_subtypes,
                prev_temp_animate_types,
                prev_temp_animate_subtypes,
                prev_temp_removed_subtypes,
                granted_keywords,
            } => {
                // Restore the exact pre-animate typeline + tracking vectors and
                // refresh the definition cache. get_mut tolerates a missing card
                // (it may have left the battlefield), matching the other
                // card-field undos.
                if let Ok(card) = game.cards.get_mut(*card_id) {
                    card.types = prev_types.clone();
                    card.subtypes = prev_subtypes.clone();
                    card.temp_animate_types = prev_temp_animate_types.clone();
                    card.temp_animate_subtypes = prev_temp_animate_subtypes.clone();
                    card.temp_removed_subtypes = prev_temp_removed_subtypes.clone();
                    // Remove the keywords this animate newly granted (e.g.
                    // Vigilance from Soulstone Sanctuary). Only those the insert
                    // site actually added are listed, so this never strips a
                    // printed or otherwise-granted keyword (mtg-610).
                    for kw in granted_keywords {
                        card.keywords.remove(*kw);
                    }
                    let types = card.types.clone();
                    let subtypes = card.subtypes.clone();
                    let name = card.name.clone();
                    card.definition.cache.update_from_types(&types);
                    card.definition.cache.update_from_subtypes(&subtypes, name.as_str());
                }
            }

            GameAction::MarkDamagedBy { target, source } => {
                // Remove the source that was appended when damage was recorded.
                // get_mut tolerates a missing card (it may have left the
                // battlefield), matching the other card-field undos.
                if let Ok(card) = game.cards.get_mut(*target) {
                    if let Some(pos) = card.damaged_by_this_turn.iter().rposition(|s| s == source) {
                        card.damaged_by_this_turn.remove(pos);
                    }
                }
            }
            GameAction::MarkDealtDamageToOpponent { card, prev } => {
                // Restore the flag to its exact pre-set value. get_mut tolerates
                // a missing card (it may have left the battlefield).
                if let Ok(card) = game.cards.get_mut(*card) {
                    card.dealt_damage_to_opponent_this_turn = *prev;
                }
            }
            GameAction::MarkAttackedThisTurn { card, prev } => {
                // Restore the flag to its exact pre-set value. get_mut tolerates
                // a missing card (it may have left the battlefield).
                if let Ok(card) = game.cards.get_mut(*card) {
                    card.attacked_this_turn = *prev;
                }
            }
            GameAction::CloneCard { card_id, prev } => {
                // Restore the copiable characteristics overwritten by the clone
                // (CR 707.2) so the cloning permanent reverts to its pre-clone
                // identity exactly (mtg-559/mtg-610). Refresh the type-flag cache
                // afterward so is_artifact()/is_creature()/etc. match the
                // restored type line.
                if let Ok(card) = game.cards.get_mut(*card_id) {
                    card.restore_copiable_state((**prev).clone());
                    let types = card.types.clone();
                    let subtypes = card.subtypes.clone();
                    let name = card.name.clone();
                    card.definition.cache.update_from_types(&types);
                    card.definition.cache.update_from_subtypes(&subtypes, name.as_str());
                }
            }
            GameAction::PushExtraTurn { player } => {
                // Reverse the AddTurn push: pop the matching entry off the BACK
                // of the queue (mtg-559/mtg-610). The forward push always
                // appends to the back, so the most-recently-pushed entry is the
                // one to remove. Tolerate an unexpectedly-empty queue (the
                // boundary pop_front may have already drained it).
                if game.extra_turns.back() == Some(player) {
                    game.extra_turns.pop_back();
                }
            }
            GameAction::SetSkipUntapNextTurn {
                player_id,
                old_value,
                new_value: _,
            } => {
                // Restore the previous skip_untap_next_turn flag value.
                if let Some(player) = game.players.iter_mut().find(|p| p.id == *player_id) {
                    player.skip_untap_next_turn = *old_value;
                }
            }
            GameAction::CreateEntity { card_id } => {
                // Reverse a token mint (mtg-ba6uq #3): remove from the
                // battlefield, clear the instance from the EntityStore, and roll
                // `next_entity_id` back to this id. Because the undo log is LIFO
                // and the forward mint did `id = next_entity_id; next_entity_id
                // += 1`, the most-recently-minted entity is undone first, so
                // setting `next_entity_id = card_id` exactly restores the counter
                // the mint consumed. Without this the token leaks through a
                // rewind and replay mints a duplicate at a higher id.
                game.battlefield.remove(*card_id);
                // clear_and_truncate (not plain clear) so the dense entity Vec
                // shrinks back to its pre-mint length — a leftover trailing None
                // slot serializes as an extra `null` and diverges the hash.
                game.cards.clear_and_truncate(*card_id);
                game.set_next_entity_id(card_id.as_u32());
            }
            GameAction::SetCardCounters { card_id, prev_counters } => {
                // Restore the exact pre-add counter set (mtg-ba6uq #4). This is
                // the only reversal that survives +1/+1 ⟷ -1/-1 annihilation,
                // which destroys counters of BOTH types and cannot be undone by
                // a per-type remove. get_mut tolerates a missing card (it may
                // have left the battlefield), matching the other card-field
                // undos.
                if let Ok(card) = game.cards.get_mut(*card_id) {
                    card.counters = prev_counters.clone();
                }
            }
            GameAction::ReorderLibrary {
                player,
                previous_order,
                previous_graveyard,
            } => {
                // Restore the captured library order (and graveyard, for surveil)
                // (mtg-ba6uq #2). Mirrors ShuffleLibrary's previous_order restore.
                if let Some(zones) = game
                    .player_zones
                    .iter_mut()
                    .find(|(id, _)| *id == *player)
                    .map(|(_, z)| z)
                {
                    zones.library.cards = previous_order.clone();
                    if let Some(prev_gy) = previous_graveyard {
                        zones.graveyard.cards = prev_gy.clone();
                    }
                }
            }
            GameAction::RegenerateReplaceDestroy {
                card_id,
                prev_shields,
                prev_damage,
                prev_combat,
            } => {
                // Restore the regeneration-shield count and the damage that the
                // shield cleared, and the full combat state it left (mtg-ba6uq
                // #5). The tap is reversed by its own logged TapCard. get_mut
                // tolerates a missing card, matching the other card-field undos.
                if let Ok(card) = game.cards.get_mut(*card_id) {
                    card.regeneration_shields = *prev_shields;
                    card.damage = *prev_damage;
                }
                game.combat = (**prev_combat).clone();
            }
            GameAction::SetSourcePreventionShields { player_id, prev } => {
                // Restore the captured source-prevention-shield list (mtg-ba6uq
                // #6).
                if let Some(player) = game.players.iter_mut().find(|p| p.id == *player_id) {
                    player.source_prevention_shields = prev.clone();
                }
            }
            GameAction::SetManaPool { player_id, prev } => {
                // Restore the captured regular mana pool (mtg-733).
                if let Some(player) = game.players.iter_mut().find(|p| p.id == *player_id) {
                    player.mana_pool = *prev;
                }
            }
            GameAction::SetDamage { card_id, prev } => {
                // Restore the captured marked damage (mtg-728 sig-2f). get_mut
                // tolerates a missing card (it may have left the battlefield),
                // matching the other card-field undos.
                if let Ok(card) = game.cards.get_mut(*card_id) {
                    card.damage = *prev;
                }
            }
            GameAction::SetXPaid { card_id, prev } => {
                // Restore the captured X-paid value (mtg-728 sig-2g). get_mut
                // tolerates a missing card, matching the other card-field undos.
                if let Ok(card) = game.cards.get_mut(*card_id) {
                    card.x_paid = *prev;
                }
            }
            GameAction::SetTimesKicked { card_id, prev } => {
                // Restore the captured times-kicked value. get_mut tolerates a
                // missing card, matching the other card-field undos.
                if let Ok(card) = game.cards.get_mut(*card_id) {
                    card.times_kicked = *prev;
                }
            }
            GameAction::SetBargainPaid { card_id, prev } => {
                // Restore the captured bargain-paid flag. get_mut tolerates a
                // missing card, matching the other card-field undos.
                if let Ok(card) = game.cards.get_mut(*card_id) {
                    card.bargain_paid = *prev;
                }
            }
            GameAction::SetCombatManaPool { player_id, prev } => {
                // Restore the captured combat mana pool (mtg-ba6uq #7).
                if let Some(player) = game.players.iter_mut().find(|p| p.id == *player_id) {
                    player.combat_mana_pool = *prev;
                }
            }
            GameAction::RestoreCardState { card_id, snapshot } => {
                // Restore the full state snapshot of a card (leaves battlefield).
                if let Ok(card) = game.cards.get_mut(*card_id) {
                    card.restore_state_snapshot((**snapshot).clone());
                } else {
                    return Err(format!("Card {} not found for RestoreCardState undo", card_id.as_u32()));
                }
            }
        }

        Ok(())
    }
}

/// Undo log for tracking and rewinding game actions
///
/// This allows efficient tree search by mutating game state forward
/// and then rewinding via the log, instead of expensive deep copies.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UndoLog {
    /// Stack of actions (most recent at end)
    actions: Vec<GameAction>,

    /// Is logging enabled? (can be compiled out for replay benchmarks)
    enabled: bool,

    /// Mark positions for choice points
    choice_points: Vec<usize>,

    /// Log buffer sizes BEFORE each action (for synchronizing log truncation on undo)
    log_sizes: Vec<usize>,
}

impl UndoLog {
    pub fn new() -> Self {
        // Pre-allocate capacity based on typical game length
        // Empirically measured: ~50 actions per turn × 20 turns = ~1000 actions
        // This avoids Vec growth allocations during gameplay
        const ESTIMATED_ACTIONS_PER_TURN: usize = 50;
        const TYPICAL_GAME_LENGTH: usize = 20;
        let estimated_capacity = ESTIMATED_ACTIONS_PER_TURN * TYPICAL_GAME_LENGTH;

        UndoLog {
            actions: Vec::with_capacity(estimated_capacity),
            enabled: true,
            choice_points: Vec::new(), // Small, can grow naturally
            log_sizes: Vec::with_capacity(estimated_capacity),
        }
    }

    /// Create a disabled undo log (for benchmarking)
    pub fn disabled() -> Self {
        UndoLog {
            actions: Vec::new(),
            enabled: false,
            choice_points: Vec::new(),
            log_sizes: Vec::new(),
        }
    }

    /// Log an action along with the log buffer size BEFORE this action
    ///
    /// The prior_log_size allows us to truncate the log buffer to the correct
    /// size when undoing this action, removing all log entries generated by it.
    pub fn log(&mut self, action: GameAction, prior_log_size: usize) {
        if self.enabled {
            self.actions.push(action);
            self.log_sizes.push(prior_log_size);
        }
    }

    /// Mark a choice point in the log
    pub fn mark_choice_point(&mut self) {
        if self.enabled {
            self.choice_points.push(self.actions.len());
        }
    }

    /// Get the most recent action without removing it
    pub fn peek(&self) -> Option<&GameAction> {
        self.actions.last()
    }

    /// Pop and return the most recent action along with its prior log size
    ///
    /// Returns (action, prior_log_size) tuple. The prior_log_size can be used
    /// to truncate the game log to remove entries generated by this action.
    pub fn pop(&mut self) -> Option<(GameAction, usize)> {
        if let Some(action) = self.actions.pop() {
            let log_size = self.log_sizes.pop().unwrap_or(0);
            Some((action, log_size))
        } else {
            None
        }
    }

    /// Get number of actions in log
    pub fn len(&self) -> usize {
        self.actions.len()
    }

    /// Reconstruct the net `tapped` state of every card touched by a `TapCard`
    /// action in this log by replaying it forward (last `TapCard` per card wins).
    ///
    /// Every forward tap/untap site (`tap_permanent`, `untap_permanent`, the
    /// mana-ability tap in `actions/mod.rs`, and the ETB-tapped self-replacement
    /// in `move_card`) logs `TapCard { tapped: <new state> }`, so the value of the
    /// LAST `TapCard` for a card equals its current `tapped`. The undo log itself
    /// is the deterministic, position-keyed record of state at its head, so this
    /// is a pure function of game position.
    ///
    /// mtg-752: the network shadow's per-turn rewind re-materialises a revealed
    /// OPPONENT permanent with a NON-undo-logged `cards.insert` that starts the
    /// instance untapped. The undo rewind cannot restore tap-state set by a
    /// `TapCard` BEFORE the rewind point (it only reverses actions AFTER it, and
    /// the re-created instance never saw the earlier tap), so `unwind_state_sync_to`
    /// uses this to re-derive the position-R `tapped` of each re-materialised
    /// opponent permanent. Returns only cards that have at least one `TapCard`
    /// entry; absence means the card was never tapped at this position.
    pub fn reconstruct_tapped_states(&self) -> std::collections::HashMap<CardId, bool> {
        let mut states = std::collections::HashMap::new();
        for action in &self.actions {
            if let GameAction::TapCard { card_id, tapped } = action {
                states.insert(*card_id, *tapped);
            }
        }
        states
    }

    /// Reconstruct the net `controller` of every card touched by a
    /// `ChangeController` action in this log by replaying it forward (last
    /// `ChangeController` per card wins → its `new_controller`).
    ///
    /// mtg-797: the exact twin of [`reconstruct_tapped_states`] for the OTHER
    /// hashed per-card battlefield field. A re-materialised revealed OPPONENT
    /// permanent is rebuilt from the blank card template, which defaults
    /// `controller = owner`; if a `ChangeController` (Control Magic, Old Man of
    /// the Sea, Steal Artifact, …) moved it to a different controller at an
    /// action_count ≤ R, neither the reveal (carries no per-instance state) nor
    /// the forward replay (re-executes only actions AFTER R) restores it, so it
    /// silently reverts to `owner` — a non-undo-logged divergence from the
    /// server. The retained undo log (truncated to ≤ R) is the deterministic
    /// record of controller at R; replay it forward to re-derive each card's
    /// position-R controller. Returns only cards with at least one
    /// `ChangeController` entry; absence means the card was never re-controlled.
    pub fn reconstruct_controller_states(&self) -> std::collections::HashMap<CardId, PlayerId> {
        let mut states = std::collections::HashMap::new();
        for action in &self.actions {
            if let GameAction::ChangeController {
                card_id,
                new_controller,
                ..
            } = action
            {
                states.insert(*card_id, *new_controller);
            }
        }
        states
    }

    pub fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }

    /// Get the last N actions as formatted strings for debugging
    ///
    /// Returns a vector of "[index] ActionDescription" strings.
    pub fn last_n_display(&self, n: usize) -> Vec<String> {
        let start = self.actions.len().saturating_sub(n);
        self.actions[start..]
            .iter()
            .enumerate()
            .map(|(i, a)| format!("[{}] {}", start + i, a))
            .collect()
    }

    /// Clear all actions up to the most recent choice point
    pub fn rewind_to_choice_point(&mut self) {
        if let Some(checkpoint) = self.choice_points.pop() {
            self.actions.truncate(checkpoint);
            self.log_sizes.truncate(checkpoint);
        }
    }

    /// Rewind to the most recent ChangeTurn action, extracting all ChoicePoint actions
    /// encountered along the way (in forward chronological order).
    ///
    /// This method actually UNDOES the game state by applying the inverse of each action.
    ///
    /// Returns (turn_number, intra_turn_choices, actions_rewound, log_size_at_turn_boundary) where:
    /// - turn_number: The turn number from the most recent ChangeTurn action
    /// - intra_turn_choices: All ChoicePoint actions that occurred after that turn change
    /// - actions_rewound: Total number of actions popped from the log
    /// - log_size_at_turn_boundary: The log buffer size at the turn boundary (for truncation)
    ///
    /// Returns None if undo log is disabled.
    ///
    /// Note: Wildcard is intentional for the inner match - we want to undo ALL GameAction
    /// variants except ChangeTurn (stop point) and ChoicePoint (non-mutating).
    #[allow(clippy::wildcard_enum_match_arm)]
    pub fn rewind_to_turn_start(&mut self, game: &mut GameState) -> Option<(u32, Vec<GameAction>, usize, usize)> {
        if !self.enabled {
            return None;
        }

        let mut choices_reversed = Vec::new();
        let mut turn_number = None;
        let mut actions_rewound = 0;
        let mut log_size_at_turn_boundary = 0;

        // Pop actions in reverse until we find ChangeTurn
        while let Some((action, log_size)) = self.pop() {
            actions_rewound += 1;
            match action {
                GameAction::ChangeTurn { turn_number: tn, .. } => {
                    // DON'T undo the ChangeTurn action - we want the snapshot to represent
                    // the START of this turn, not the END of the previous turn.
                    // Put it back on the log so the game state stays at the turn boundary.
                    self.actions.push(action);
                    self.log_sizes.push(log_size);
                    actions_rewound -= 1; // Don't count this as rewound since we kept it
                    turn_number = Some(tn);
                    log_size_at_turn_boundary = log_size;
                    break;
                }
                GameAction::ChoicePoint { .. } => {
                    // Collect choice points in reverse (don't need to undo, they're non-mutating)
                    choices_reversed.push(action);
                }
                _ => {
                    // Undo all other actions to restore game state
                    if let Err(e) = action.undo(game) {
                        eprintln!("WARNING: Failed to undo action {:?}: {}", action, e);
                    }
                }
            }
        }

        // If we found a ChangeTurn, use that turn number.
        // Otherwise (turn 1), use turn 1 as the turn number.
        // The game state has been rewound either way.
        let effective_turn = turn_number.unwrap_or(1);

        // (mtg-610: the per-turn re-entry guard family — draw_step_executed_turn
        // and friends — was deleted now that WASM re-entry resumes via
        // rewind+replay rather than re-running steps without a rewind. There is no
        // longer a transient guard set to reset after the rewind; the replay re-runs
        // each step from the restored turn-start state exactly once.)

        // Invalidate mana engine cache. Undo actions restore the battlefield
        // but the ManaEngine memoization (keyed on mana_state_version) may
        // retain stale capacity from a later game state. Bumping the version
        // forces re-scan on the next can_pay() query.
        game.mana_state_version = game.mana_state_version.wrapping_add(1);

        // Clear mana source caches. These live on GameState (not the undo log)
        // and accumulate sources during replay that are no longer on the
        // battlefield after rewind. Without clearing, ManaEngine::update_mut
        // reads stale caches and reports inflated mana capacity.
        for (_, cache) in &mut game.mana_caches {
            cache.clear();
        }

        // Reset priority state that persists across NeedInput returns.
        // These fields are NOT #[serde(skip)] (they must survive serialization) and NOT
        // tracked by the undo log (they're updated directly by priority_round).
        // After rewinding, the replay re-executes priority rounds from scratch, so stale
        // values from the interrupted execution would cause the wrong player to get priority
        // first, or skip players entirely (e.g., consecutive_passes=1 from an interrupted
        // End step would cause the Turn 3 Upkeep priority round to end after just one pass,
        // producing a 1-action DESYNC).
        game.turn.priority_player = None;
        game.turn.consecutive_passes = 0;

        // Clear pending activation state (not tracked by undo log)
        game.sub_action_scratch.pending_activation = None;
        game.sub_action_scratch.pending_cycling_search = None;
        game.sub_action_scratch.spell_targets.clear();

        // Clear the server-side library-reorder broadcast queue (not tracked by
        // the undo log). `pending_library_reorders` is a transient `RefCell<Vec>`
        // populated by `scry_apply_decision` / `surveil_apply_decision` on the
        // GOLDEN (non-shadow, network) state and drained by `NetworkController`
        // when it assembles a `ChoiceRequest`. It is NOT a logged GameAction, so
        // a rewind leaves whatever was queued at the rewind point — and a replay
        // re-runs the scry/surveil and re-pushes the same entries, double-queuing
        // a reorder that the original forward pass had already drained.
        //
        // Clearing it here makes rewind+replay re-derive the reorder broadcast
        // identically: the same scry on replay re-enqueues exactly the same
        // entries from a clean queue. This is the replay-safe counterpart of the
        // non-destructive `state_sync` ActionLog on the client (the server side
        // re-derives reorders from the library state, which IS undo-logged,
        // rather than from a destructively-drained side queue) — see mtg-610 /
        // docs/NETWORK_ACTION_LOG.md § 3.2. For current native play this is a
        // no-op: the golden state is never rewound mid-game, and the server
        // drains the queue at every ChoiceRequest within the turn, so it is
        // already empty at any turn boundary. It only matters once server-side
        // rewind+replay (or MCTS rollouts on the golden state) drive this path.
        game.sub_action_scratch.pending_library_reorders.borrow_mut().clear();

        // Clear combat state (not tracked by undo log).
        // CombatState is modified directly by declare_attacker/declare_blocker and
        // cleared at end_of_combat. After rewinding to turn start, combat hasn't
        // begun yet, so all combat maps must be empty. Without this, a creature
        // that was declared as attacker before the rewind would still show as
        // attacking (is_attacking=true), preventing it from being selected as an
        // attacker during replay and causing the replay to miss choices.
        game.combat.clear();

        // Clear per-turn transient state on battlefield permanents (not tracked by undo log).
        // These fields are modified directly during the turn but are NOT logged as undo
        // actions. The cleanup step at end of previous turn resets them all, so at turn
        // start they must be zero/None. Without this:
        // - damage persists after rewind and accumulates during replay (original + replayed
        //   = 2x damage), causing spurious state-based action kills
        // - power_bonus/toughness_bonus are handled by PumpCreature undo, but we clear them
        //   defensively since cleanup already set them to 0 at turn boundary
        // - temp_base_power/toughness (from Animate effects) have NO undo support at all
        for card_id in game.battlefield.cards.iter() {
            if let Ok(card) = game.cards.get_mut(*card_id) {
                card.damage = 0;
                card.power_bonus = 0;
                card.toughness_bonus = 0;
                card.clear_temp_base_stats();
                // The cleanup step (game_loop/steps.rs) also clears these
                // per-card "until end of turn" transients on every battlefield
                // permanent (CR 514.2): the per-source damage-prevention pool
                // and the two combat replacement flags. None are undo-logged,
                // so at any turn boundary they hold their post-cleanup value.
                card.damage_prevention = 0;
                card.exile_if_would_die_this_turn = false;
                card.prevent_all_combat_damage_this_turn = false;
            }
        }

        // Regeneration shields are "until end of turn" (CR 614.8 / 701.15):
        // created when a regenerate ability resolves, removed in the cleanup
        // step. They are NOT undo-logged, so at any turn boundary the count
        // must be 0 for EVERY card — not just battlefield permanents. A
        // reanimated creature (e.g. Sedge Troll) can regenerate in combat and
        // then leave the battlefield (die / be exiled) within the same turn, so
        // a stale shield can ride along on a card now in the graveyard/exile;
        // the battlefield-only loop above would miss it, leaving the rewound
        // turn-start `regeneration_shields` history-dependent (mtg-610). Reset
        // it for all cards; a forward replay re-activates the regen and re-sets
        // the shield where appropriate. (Same per-turn-transient class as
        // `damage`, but zone-independent.)
        //
        // `damaged_by_this_turn` is NOT blanket-reset here: it is made
        // reversible via `GameAction::MarkDamagedBy` (logged when combat damage
        // records a source), so the undo-action phase above restores its exact
        // pre-rewind value. A blanket clear would break the per-action undo
        // oracle, whose replay re-derives combat damage by REDOING the logged
        // marks rather than re-running the auto combat-damage step.
        //
        // `dealt_damage_to_opponent_this_turn` (Whirling Dervish) follows the
        // SAME contract: it is set+logged as a reversible
        // `GameAction::MarkDealtDamageToOpponent` in the combat-damage step, so
        // the undo-action phase above already restored its exact pre-rewind
        // value — do NOT blanket-clear it here.
        for card in game.cards.values_mut() {
            card.regeneration_shields = 0;
            // Until-end-of-turn granted keywords (Rockface Village "gains haste
            // until EOT", AnimateAll, ...) are removed at the forward cleanup
            // step (GameState::cleanup_temporary_effects) but are NOT all
            // undo-logged for the cleanup transition, so at any turn boundary
            // the until-EOT keyword set must be empty for EVERY card — same
            // per-turn-transient, zone-independent class as regeneration_shields
            // (a pumped creature can leave the battlefield same turn). A forward
            // replay re-grants the keyword via PumpCreature/AnimateAll where
            // appropriate. Without this the Haste bit from a turn-N grant rode
            // through a rewind to turn N+1 start, making turn-start keywords
            // history-dependent across rewinds (mtg-610: Rockface Village
            // turn-12-start haste drift). clear_temp_keywords_until_eot removes
            // exactly the granted-until-EOT bits from the live set, never a
            // printed/permanent keyword.
            card.clear_temp_keywords_until_eot();
        }
        // NOTE: until-end-of-turn `AB$ Animate` typeline changes (Mishra's
        // Factory / robots-deck animated artifacts) are NOW reverted by the
        // undo-action phase above via the reversible `GameAction::AnimateTypeline`
        // (mtg-610): its `undo()` restores the exact prior `types`/`subtypes` +
        // tracking vectors and refreshes the definition cache. We deliberately
        // do NOT also call `revert_temp_animation()` here — doing so would
        // double-revert (drain tracking vectors the undo already restored),
        // breaking the rewind+replay round-trip. The cleanup step still uses
        // `revert_temp_animation` for the forward end-of-turn path.

        // Per-player "until end of turn" damage-prevention shields (CR 514.2),
        // including source-filtered shields (Circle of Protection). The cleanup
        // step clears both on every player; neither is undo-logged, so at any
        // turn boundary they hold their post-cleanup (empty/zero) value. A
        // forward replay re-establishes any shields cast this turn.
        for player in &mut game.players {
            player.damage_prevention = 0;
            player.source_prevention_shields.clear();
            // Firebending combat mana (CR 701.65) is a per-combat transient that
            // empties at end of combat; it is always None at a turn boundary, so
            // clear it on rewind for explicit correctness (mtg-ba6uq #7). A
            // forward replay re-adds any combat mana produced this turn.
            player.combat_mana_pool = None;
        }

        // SHADOW-ONLY: clear card instances that leaked into a hidden library
        // zone (mtg-610 undo-completeness hole). On a network CLIENT shadow,
        // library cards are reserved-but-vacant slots — their identity is only
        // known once the server's asynchronous `RevealCard` state-sync entry is
        // applied (which `process_card_reveal` does via a NON-undo-logged
        // `game.cards.insert`). When the shadow draws such a card and is then
        // rewound, the `MoveCard` is undone (card returns to the library) but
        // the async-inserted instance is NOT (the insert was never an undo-log
        // action). Worse, WHEN the async reveal lands relative to the draw is
        // history-dependent (the state-sync entry is keyed by SERVER
        // action_count, which diverges from the shadow's after a rewind), so
        // the post-rewind `cards` set differs across repeated rewinds to the
        // same turn — failing the turn-start determinism invariant. Because a
        // library card in the shadow must be a vacant reserved slot at any
        // turn boundary, we deterministically clear every instance currently
        // sitting in a library zone. A forward replay re-applies the reveal and
        // re-instantiates it identically. This is a no-op on the SERVER / native
        // (non-shadow) game, where library cards are legitimately instantiated.
        if game.is_shadow_game {
            let library_card_ids: SmallVec<[crate::core::CardId; 64]> = game
                .players
                .iter()
                .filter_map(|p| game.get_player_zones(p.id))
                .flat_map(|z| z.library.cards.iter().copied())
                .collect();
            for card_id in library_card_ids {
                if game.cards.contains(card_id) {
                    game.cards.clear(card_id);
                }
            }
        }

        // Reverse the choices to get forward chronological order
        choices_reversed.reverse();
        Some((
            effective_turn,
            choices_reversed,
            actions_rewound,
            log_size_at_turn_boundary,
        ))
    }

    /// Get the most recent turn number from the log, if any ChangeTurn exists
    pub fn current_turn(&self) -> Option<u32> {
        self.actions.iter().rev().find_map(|action| {
            if let GameAction::ChangeTurn { turn_number, .. } = action {
                Some(*turn_number)
            } else {
                None
            }
        })
    }

    /// Clear the entire log
    pub fn clear(&mut self) {
        self.actions.clear();
        self.choice_points.clear();
        self.log_sizes.clear();
    }

    /// Get all actions (for debugging/serialization)
    pub fn actions(&self) -> &[GameAction] {
        &self.actions
    }

    /// Rebuild parsed_svars in RestoreCardState snapshots after deserialization
    pub fn rebuild_parsed_svars(&mut self) {
        for action in &mut self.actions {
            if let GameAction::RestoreCardState { snapshot, .. } = action {
                snapshot.definition.rebuild_parsed_svars();
            }
        }
    }

    /// Format the last N actions as a multi-line string for debugging
    ///
    /// Returns a string with one action per line, most recent last.
    /// Each line is prefixed with its index in the full action log.
    pub fn format_last_n(&self, n: usize) -> String {
        let len = self.actions.len();
        let start = len.saturating_sub(n);
        let mut result = String::new();
        for (i, action) in self.actions[start..].iter().enumerate() {
            use std::fmt::Write;
            let _ = writeln!(result, "  [{:4}] {}", start + i, action);
        }
        result
    }
}

impl Default for UndoLog {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_undo_log() {
        let mut log = UndoLog::new();
        assert_eq!(log.len(), 0);

        let action = GameAction::ModifyLife {
            player_id: PlayerId::new(1),
            delta: -3,
        };

        log.log(action, 0);
        assert_eq!(log.len(), 1);

        let (popped, log_size) = log.pop().unwrap();
        assert!(matches!(popped, GameAction::ModifyLife { .. }));
        assert_eq!(log_size, 0);
        assert_eq!(log.len(), 0);
    }

    #[test]
    fn test_reconstruct_tapped_states() {
        // mtg-752: the network shadow uses this to re-derive the position-R
        // tapped state of a reveal-materialised opponent permanent. Last TapCard
        // per card wins; absence means never tapped.
        let mut log = UndoLog::new();
        let a = CardId::new(14);
        let b = CardId::new(7);
        let c = CardId::new(99);
        // a: tapped, then untapped, then tapped again -> final true
        log.log(
            GameAction::TapCard {
                card_id: a,
                tapped: true,
            },
            0,
        );
        log.log(
            GameAction::TapCard {
                card_id: b,
                tapped: true,
            },
            0,
        );
        log.log(
            GameAction::TapCard {
                card_id: a,
                tapped: false,
            },
            0,
        );
        log.log(
            GameAction::TapCard {
                card_id: a,
                tapped: true,
            },
            0,
        );
        // b: tapped then untapped -> final false
        log.log(
            GameAction::TapCard {
                card_id: b,
                tapped: false,
            },
            0,
        );

        let states = log.reconstruct_tapped_states();
        assert_eq!(states.get(&a), Some(&true), "a's last TapCard is tapped=true");
        assert_eq!(states.get(&b), Some(&false), "b's last TapCard is tapped=false");
        assert_eq!(
            states.get(&c),
            None,
            "c never tapped -> absent (caller defaults untapped)"
        );
    }

    #[test]
    fn test_reconstruct_controller_states() {
        // mtg-797: twin of test_reconstruct_tapped_states for the `controller`
        // field. Last ChangeController per card wins (→ its new_controller);
        // absence means the card was never re-controlled (caller defaults owner).
        let mut log = UndoLog::new();
        let a = CardId::new(14);
        let b = CardId::new(7);
        let c = CardId::new(99);
        let p0 = PlayerId::new(0);
        let p1 = PlayerId::new(1);
        // a: owner p0 -> p1 (Control Magic), then back p1 -> p0 (effect ended),
        // then p0 -> p1 again -> final p1.
        log.log(
            GameAction::ChangeController {
                card_id: a,
                old_controller: p0,
                new_controller: p1,
            },
            0,
        );
        log.log(
            GameAction::ChangeController {
                card_id: b,
                old_controller: p1,
                new_controller: p0,
            },
            0,
        );
        log.log(
            GameAction::ChangeController {
                card_id: a,
                old_controller: p1,
                new_controller: p0,
            },
            0,
        );
        log.log(
            GameAction::ChangeController {
                card_id: a,
                old_controller: p0,
                new_controller: p1,
            },
            0,
        );

        let states = log.reconstruct_controller_states();
        assert_eq!(states.get(&a), Some(&p1), "a's last ChangeController -> p1");
        assert_eq!(states.get(&b), Some(&p0), "b's last ChangeController -> p0");
        assert_eq!(
            states.get(&c),
            None,
            "c never re-controlled -> absent (caller defaults owner)"
        );
    }

    #[test]
    fn test_choice_points() {
        let mut log = UndoLog::new();

        log.log(
            GameAction::ModifyLife {
                player_id: PlayerId::new(1),
                delta: -1,
            },
            0,
        );
        log.log(
            GameAction::ModifyLife {
                player_id: PlayerId::new(1),
                delta: -1,
            },
            0,
        );

        log.mark_choice_point();

        log.log(
            GameAction::ModifyLife {
                player_id: PlayerId::new(1),
                delta: -1,
            },
            0,
        );
        log.log(
            GameAction::ModifyLife {
                player_id: PlayerId::new(1),
                delta: -1,
            },
            0,
        );

        assert_eq!(log.len(), 4);

        log.rewind_to_choice_point();
        assert_eq!(log.len(), 2);
    }

    #[test]
    fn test_disabled_log() {
        let mut log = UndoLog::disabled();

        log.log(
            GameAction::ModifyLife {
                player_id: PlayerId::new(1),
                delta: -1,
            },
            0,
        );

        assert_eq!(log.len(), 0); // Nothing logged when disabled
    }

    #[test]
    fn test_rewind_to_turn_start() {
        let mut log = UndoLog::new();
        let mut game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);

        // Simulate turn 1 starting
        log.log(
            GameAction::ChangeTurn {
                from_player: PlayerId::new(0),
                to_player: PlayerId::new(1),
                turn_number: 1,
                rng_state: None,
            },
            0,
        );

        // Some actions during turn 1
        log.log(
            GameAction::ModifyLife {
                player_id: PlayerId::new(1),
                delta: -1,
            },
            0,
        );

        log.log(
            GameAction::ChoicePoint {
                player_id: PlayerId::new(1),
                choice_id: 1,
                choice: None,
            },
            0,
        );

        log.log(
            GameAction::TapCard {
                card_id: CardId::new(1),
                tapped: true,
            },
            0,
        );

        log.log(
            GameAction::ChoicePoint {
                player_id: PlayerId::new(1),
                choice_id: 2,
                choice: None,
            },
            0,
        );

        assert_eq!(log.len(), 5);

        // Rewind to turn start (now requires GameState)
        let result = log.rewind_to_turn_start(&mut game);
        assert!(result.is_some());

        let (turn_number, choices, actions_rewound, _log_size) = result.unwrap();
        assert_eq!(turn_number, 1);
        assert_eq!(choices.len(), 2);
        assert_eq!(actions_rewound, 4); // All 4 actions after ChangeTurn (ChangeTurn is kept)

        // Verify choices are in forward chronological order
        assert!(matches!(
            choices[0],
            GameAction::ChoicePoint {
                player_id: _,
                choice_id: 1,
                choice: None
            }
        ));
        assert!(matches!(
            choices[1],
            GameAction::ChoicePoint {
                player_id: _,
                choice_id: 2,
                choice: None
            }
        ));

        // Log should have the ChangeTurn action still (we stopped AT the turn boundary)
        assert_eq!(log.len(), 1);
        assert!(matches!(log.peek().unwrap(), GameAction::ChangeTurn { .. }));
    }

    #[test]
    fn test_rewind_to_turn_start_no_turn() {
        let mut log = UndoLog::new();
        let mut game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);

        // Add some actions but no ChangeTurn (simulates turn 1)
        log.log(
            GameAction::ModifyLife {
                player_id: PlayerId::new(1),
                delta: -1,
            },
            0,
        );

        log.log(
            GameAction::ChoicePoint {
                player_id: PlayerId::new(1),
                choice_id: 1,
                choice: None,
            },
            0,
        );

        // When no ChangeTurn is found, rewind should still succeed with turn 1
        // This is important for turn 1 where no ChangeTurn has been logged yet
        let result = log.rewind_to_turn_start(&mut game);
        assert!(result.is_some(), "rewind_to_turn_start should return Some for turn 1");

        let (turn_number, choice_actions, actions_rewound, _log_size) = result.unwrap();
        assert_eq!(turn_number, 1, "Turn number should be 1 when no ChangeTurn found");
        assert_eq!(choice_actions.len(), 1, "Should have 1 ChoicePoint action");
        assert_eq!(actions_rewound, 2, "Should have rewound 2 actions");

        // Undo log should be empty after rewinding everything
        assert!(log.is_empty(), "Undo log should be empty after full rewind");
    }

    #[test]
    fn test_current_turn() {
        let mut log = UndoLog::new();

        assert_eq!(log.current_turn(), None);

        log.log(
            GameAction::ChangeTurn {
                from_player: PlayerId::new(0),
                to_player: PlayerId::new(1),
                turn_number: 1,
                rng_state: None,
            },
            0,
        );

        assert_eq!(log.current_turn(), Some(1));

        log.log(
            GameAction::ModifyLife {
                player_id: PlayerId::new(1),
                delta: -1,
            },
            0,
        );

        log.log(
            GameAction::ChangeTurn {
                from_player: PlayerId::new(1),
                to_player: PlayerId::new(0),
                turn_number: 2,
                rng_state: None,
            },
            0,
        );

        // Should return the most recent turn
        assert_eq!(log.current_turn(), Some(2));
    }

    // =========================================================================
    // RevealCard tests (mtg-218)
    // =========================================================================

    #[test]
    fn test_reveal_card_display_with_name() {
        let action = GameAction::RevealCard {
            card_id: CardId::new(5),
            name: Some("Lightning Bolt".to_string()),
            revealed_to: RevealTarget::All,
            old_mask: 0,
        };

        let display = format!("{}", action);
        assert_eq!(display, "RevealCard(5 = \"Lightning Bolt\" to ALL mask:0x00)");
    }

    #[test]
    fn test_reveal_card_display_to_single_player() {
        let action = GameAction::RevealCard {
            card_id: CardId::new(5),
            name: Some("Lightning Bolt".to_string()),
            revealed_to: RevealTarget::Player(PlayerId::new(1)),
            old_mask: 0,
        };

        let display = format!("{}", action);
        assert_eq!(display, "RevealCard(5 = \"Lightning Bolt\" to P1 mask:0x00)");
    }

    #[test]
    fn test_reveal_card_display_without_name() {
        // Opponent perspective - doesn't know the card name
        let action = GameAction::RevealCard {
            card_id: CardId::new(42),
            name: None,
            revealed_to: RevealTarget::Player(PlayerId::new(0)),
            old_mask: 0,
        };

        let display = format!("{}", action);
        assert_eq!(display, "RevealCard(42 = ??? to P0 mask:0x00)");
    }

    #[test]
    fn test_reveal_card_undo_with_name() {
        use crate::core::Card;

        let mut game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);

        // Reserve a slot for the card (as would be done at game start)
        game.cards.reserve(CardId::new(100));
        assert!(!game.cards.is_revealed(CardId::new(100)));

        // Create a test card and insert (simulating forward execution)
        let mut card = Card::new(CardId::new(100), "Test Card", PlayerId::new(0));
        // Mark as revealed to all (simulating forward execution of reveal)
        card.mark_revealed_to_all();
        game.cards.insert(CardId::new(100), card);
        assert!(game.cards.is_revealed(CardId::new(100)));
        assert!(game.cards.get(CardId::new(100)).unwrap().is_revealed_to_all());

        // Create the RevealCard action with old_mask=0 (was unrevealed before)
        let action = GameAction::RevealCard {
            card_id: CardId::new(100),
            name: Some("Test Card".to_string()),
            revealed_to: RevealTarget::All,
            old_mask: 0,
        };

        // Undo the reveal
        action.undo(&mut game).unwrap();

        // Card should still exist but mask restored to 0
        assert!(game.cards.is_revealed(CardId::new(100))); // card still exists
        assert_eq!(game.cards.get(CardId::new(100)).unwrap().revealed_to_mask, 0);
    }

    #[test]
    fn test_reveal_card_undo_dummy_reveal() {
        // Dummy reveal (opponent perspective) - name is None
        let mut game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);

        // Reserve a slot (slot stays empty for opponent)
        game.cards.reserve(CardId::new(100));
        assert!(!game.cards.is_revealed(CardId::new(100)));

        // Create dummy RevealCard (opponent doesn't learn the card)
        // revealed_to is Player(0), but since we're the opponent (Player 1), name is None
        let action = GameAction::RevealCard {
            card_id: CardId::new(100),
            name: None,
            revealed_to: RevealTarget::Player(PlayerId::new(0)),
            old_mask: 0,
        };

        // Undo should succeed without error (no-op)
        action.undo(&mut game).unwrap();

        // Slot should still be unrevealed
        assert!(!game.cards.is_revealed(CardId::new(100)));
    }

    #[test]
    fn test_reveal_card_round_trip_via_undo_log() {
        use crate::core::Card;

        let mut game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
        let mut log = UndoLog::new();

        // Reserve slot
        game.cards.reserve(CardId::new(50));

        // Create and insert card (forward execution)
        let mut card = Card::new(CardId::new(50), "Mountain", PlayerId::new(0));
        // Mark as revealed to all (simulating forward execution of reveal)
        card.mark_revealed_to_all();
        game.cards.insert(CardId::new(50), card);

        // Log the reveal action
        log.log(
            GameAction::RevealCard {
                card_id: CardId::new(50),
                name: Some("Mountain".to_string()),
                revealed_to: RevealTarget::All,
                old_mask: 0,
            },
            0,
        );

        // Verify card is revealed
        assert!(game.cards.is_revealed(CardId::new(50)));
        assert!(game.cards.get(CardId::new(50)).unwrap().is_revealed_to_all());
        assert_eq!(log.len(), 1);

        // Pop and undo
        let (action, _) = log.pop().unwrap();
        action.undo(&mut game).unwrap();

        // Card still exists but mask is restored to 0
        assert!(game.cards.is_revealed(CardId::new(50))); // card still exists
        assert_eq!(game.cards.get(CardId::new(50)).unwrap().revealed_to_mask, 0);
        assert!(log.is_empty());
    }

    /// Regression test for `bug-desync-seed41` (WASM rewind/replay desync).
    ///
    /// The WASM rewind/replay loop relies on a key invariant: rewinding the
    /// game to the same turn boundary must always reproduce the same Replay
    /// hash, regardless of how many rewinds (or how much forward play between
    /// them) have happened. The replay verifier in `wasm/replay_verifier.rs`
    /// caches the first turn-start hash for each turn and treats any drift
    /// as a fatal "REWIND/REPLAY FATAL: turn-start state hash for turn N
    /// changed across rewinds" error.
    ///
    /// Before the fix, `rewind_to_turn_start` unconditionally bumped
    /// `mana_state_version` (a `ManaEngine` cache invalidation counter that
    /// was — incorrectly — included in the Replay hash). Two consecutive
    /// rewinds to the same turn therefore produced different hashes and
    /// blew up the verifier on the user's *second* WASM input on turn 1
    /// (e.g. play a Mox, then play a Bayou).
    ///
    /// This test directly exercises the property: rewind, then forward-play
    /// some actions that themselves bump `mana_state_version` (taps, untaps),
    /// then rewind again — the post-rewind Replay hash must be identical.
    #[test]
    fn rewind_to_turn_start_produces_stable_replay_hash() {
        use crate::game::compute_state_hash;
        use crate::game::GameState;

        let mut game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
        let p1 = game.players[0].id;

        // Establish a turn boundary (the "turn 1 start") in the undo log.
        // Subsequent forward play happens after this marker; rewinding to
        // turn start should reverse everything down to (but not including)
        // this marker.
        let prior_log_size = game.logger.log_count();
        game.undo_log.log(
            GameAction::ChangeTurn {
                from_player: p1,
                to_player: p1,
                turn_number: 1,
                rng_state: None,
            },
            prior_log_size,
        );

        // Snapshot the canonical turn-start Replay hash.
        let h_turn_start = compute_state_hash(&game);

        // Forward-play and rewind, several times, asserting hash stability.
        for cycle in 0..5 {
            // Mutate life via a properly-logged GameAction so it can be
            // undone, mimicking what real forward play does.
            let prior_log_size = game.logger.log_count();
            if let Some(player) = game.players.iter_mut().find(|p| p.id == p1) {
                player.life -= 1;
            }
            game.undo_log.log(
                GameAction::ModifyLife {
                    player_id: p1,
                    delta: -1,
                },
                prior_log_size,
            );
            // Several bumps to make sure the counter has clearly advanced —
            // mimics taps/untaps/etb-events in real play.
            for _ in 0..3 {
                game.increment_mana_version();
            }

            // Rewind to turn start (same pattern as WasmFancyTuiState).
            let mut undo_log = std::mem::take(&mut game.undo_log);
            let result = undo_log.rewind_to_turn_start(&mut game);
            game.undo_log = undo_log;
            assert!(
                result.is_some(),
                "rewind cycle {cycle}: rewind_to_turn_start must succeed"
            );

            // Sanity: mana_state_version really did change inside rewind, so
            // this test would FAIL on Replay hash inclusion of the counter.
            assert!(
                game.mana_state_version > 0,
                "rewind cycle {cycle}: mana_state_version should have been bumped"
            );

            let h_after = compute_state_hash(&game);
            assert_eq!(
                h_after, h_turn_start,
                "rewind cycle {cycle}: post-rewind Replay hash must equal the original \
                 turn-start hash. mana_state_version bumps inside rewind_to_turn_start \
                 must NOT affect the Replay hash (bug-desync-seed41 regression)."
            );
        }
    }

    /// `rewind_to_turn_start` must clear the server-side
    /// `pending_library_reorders` broadcast queue (mtg-610 / mtg-559).
    ///
    /// That queue is a transient `RefCell<Vec>` NOT tracked by the undo log; it
    /// is populated by `scry`/`surveil` on the golden network state and drained
    /// by `NetworkController`. If a rewind left a stale entry queued, a replay
    /// would re-run the scry and re-enqueue the SAME reorder on top of the stale
    /// one — double-broadcasting a `LibraryReordered` and desyncing the client
    /// shadow. This test asserts the queue is empty after a rewind so replay
    /// re-derives reorders from a clean queue (the replay-safe counterpart of
    /// the non-destructive client-side `state_sync` ActionLog).
    #[test]
    fn rewind_to_turn_start_clears_pending_library_reorders() {
        use crate::game::GameState;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1 = game.players[0].id;

        // Establish a turn-2 boundary so the rewind has something to rewind TO.
        let prior_log_size = game.logger.log_count();
        game.undo_log.log(
            GameAction::ChangeTurn {
                from_player: p1,
                to_player: p1,
                turn_number: 2,
                rng_state: None,
            },
            prior_log_size,
        );

        // Simulate a scry/surveil having queued a library-reorder broadcast for
        // P1 (as `scry_apply_decision` does on the golden state). This is the
        // stale-side-queue entry that a rewind must not leave behind.
        game.sub_action_scratch
            .pending_library_reorders
            .borrow_mut()
            .push((p1, 0));
        assert_eq!(
            game.sub_action_scratch.pending_library_reorders.borrow().len(),
            1,
            "precondition: one reorder queued before rewind"
        );

        let mut undo_log = std::mem::take(&mut game.undo_log);
        let result = undo_log.rewind_to_turn_start(&mut game);
        game.undo_log = undo_log;
        assert!(result.is_some(), "rewind_to_turn_start should succeed");

        assert!(
            game.sub_action_scratch.pending_library_reorders.borrow().is_empty(),
            "rewind_to_turn_start must clear pending_library_reorders so a replay \
             re-derives the reorder broadcast from a clean queue (no double-broadcast). \
             See mtg-610 / docs/NETWORK_ACTION_LOG.md § 3.2."
        );
    }
}
