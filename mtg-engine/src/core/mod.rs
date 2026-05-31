//! Core game types and entities

pub mod card;
pub mod costs;
pub mod delayed_trigger;
pub mod effects;
pub mod entity;
pub mod keyword_set;
pub mod mana;
pub mod mana_production;
pub mod persistent_effect;
pub mod player;
pub mod prevention;
pub mod spell_ability;
pub mod types;

pub use card::{Card, CardCache, CardType};
pub use costs::Cost;
pub use delayed_trigger::{
    DelayedEffect, DelayedTrigger, DelayedTriggerCondition, DelayedTriggerExpiry, DelayedTriggerId,
    DelayedTriggerStore, TriggerPhase, TurnOwner,
};
pub use effects::{
    AbilityCache, ActivatedAbility, ActivationCondition, AffectedSelector, CombatDamageTarget, CompareOp,
    ControllerRestriction, CostReductionCondition, CostReductionTarget, CountExpression, DigFilter, DynamicAmount,
    Effect, ImmediateTriggerCondition, ModalMode, RaisedCost, RaisedCostAmount, SelfCounterCondition, StaticAbility,
    StaticCondition, TargetRef, TargetRestriction, TargetType, Trigger, TriggerEvent,
};
pub use entity::{EntityId, EntityStore, GameEntity};
pub use keyword_set::{Keyword, KeywordArgs, KeywordSet};
pub use mana::{Color, ManaCost, ManaPool};
pub use mana_production::{ManaColor, ManaProduction, ManaProductionKind, ManaSideCost};
pub use persistent_effect::{
    CleanupCondition, PersistentEffect, PersistentEffectId, PersistentEffectKind, PersistentEffectStore,
};
pub use player::Player;
pub use prevention::{DamagePreventionShield, DamageSourceFilter, PreventionScope};
pub use spell_ability::SpellAbility;
pub use types::{CardName, CounterType, PlayerName, SetCode, Subtype};

// Type aliases for strongly-typed entity IDs
/// Strongly-typed ID for Player entities
pub type PlayerId = EntityId<Player>;

/// Strongly-typed ID for Card entities
pub type CardId = EntityId<Card>;

/// Encode a `PlayerId` as a `CardId`-shaped sentinel so it can ride inside
/// `valid_targets` for `Controller::choose_targets`. This lets controllers
/// offer Players as targets for `ValidTgts$ Any`-style effects (Lightning
/// Bolt, Shock, Drain Life) without churning the trait signature.
/// We encode as `BASE + player_id` (not BASE - player_id) so that
/// `valid_targets.sort()` (called for snapshot/resume determinism in
/// `get_valid_targets_for_spell`) preserves PlayerId order. See
/// `entity::PLAYER_TARGET_BASE`.
#[inline]
pub fn player_as_target_sentinel(p: PlayerId) -> CardId {
    CardId::new(entity::PLAYER_TARGET_BASE + p.as_u32())
}

/// Decode a sentinel CardId back into a PlayerId, if it is one.
/// Returns `None` for ordinary CardIds (which lie in the low range).
#[inline]
pub fn player_target_from_sentinel(c: CardId) -> Option<PlayerId> {
    let v = c.as_u32();
    // Accept up to 64 player IDs above the base. Real CardIds grow from 0
    // and never approach u32::MAX - 1000.
    if (entity::PLAYER_TARGET_BASE..entity::PLAYER_TARGET_BASE + 64).contains(&v) {
        Some(PlayerId::new(v - entity::PLAYER_TARGET_BASE))
    } else {
        None
    }
}

/// Reorder the player-target sentinels in a target list so that an opponent's
/// player sentinel is offered BEFORE the caster's own. Most targeted spells
/// (Lightning Bolt, Shock, ...) are aimed at an opponent, so listing the
/// opponent first matches the common case and lets a default "first player"
/// pick do the right thing.
///
/// Card targets keep their relative order; only the player sentinels are
/// reordered, and the operation is a *stable* partition keyed solely on
/// `viewer` (the casting player). Because `viewer` is derivable identically on
/// server and client (it is the spell's controller, not hidden information),
/// this preserves network determinism — every controller sees the same order.
///
/// FUTURE (mtg-605): beneficial spells (gain life, regeneration, ...) should
/// flip this to offer the caster's own player first. That classification is
/// deliberately deferred; this helper unconditionally orders opponents first.
pub fn reorder_player_targets_opponents_first(targets: &mut [CardId], viewer: PlayerId) {
    // Stable partition: opponents' player sentinels sort before the viewer's.
    // Non-player CardIds compare equal to each other and to players-of-equal
    // class, so a stable sort leaves them (and their relative order) untouched
    // ahead of any reordering among the player sentinels they precede.
    targets.sort_by_key(|&c| match player_target_from_sentinel(c) {
        // Card target: keep ahead of players, all equal rank 0.
        None => 0u8,
        // Opponent's player sentinel: rank 1 (before the viewer's own).
        Some(pid) if pid != viewer => 1,
        // Viewer's own player sentinel: rank 2 (last).
        Some(_) => 2,
    });
}

/// Decode a chosen target `CardId` (as stored in `chosen_targets`) into a
/// `TargetRef`. Player-target sentinels (e.g. Lightning Bolt aimed at a
/// player) decode to `TargetRef::Player`; everything else is a permanent.
///
/// This centralizes the sentinel-vs-permanent branch that effect resolution
/// would otherwise repeat at every "any target" damage site. See
/// `player_as_target_sentinel`.
#[inline]
pub fn target_ref_from_chosen_target(c: CardId) -> TargetRef {
    match player_target_from_sentinel(c) {
        Some(pid) => TargetRef::Player(pid),
        None => TargetRef::Permanent(c),
    }
}
