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
pub mod spell_ability;
pub mod types;

pub use card::{Card, CardCache, CardType};
pub use costs::Cost;
pub use delayed_trigger::{
    DelayedEffect, DelayedTrigger, DelayedTriggerCondition, DelayedTriggerExpiry, DelayedTriggerId,
    DelayedTriggerStore, TriggerPhase, TurnOwner,
};
pub use effects::{
    AbilityCache, ActivatedAbility, AffectedSelector, ControllerRestriction, CostReductionCondition,
    CostReductionTarget, CountExpression, DigFilter, Effect, ImmediateTriggerCondition, ModalMode, RaisedCost,
    RaisedCostAmount, StaticAbility, StaticCondition, TargetRef, TargetRestriction, TargetType, Trigger, TriggerEvent,
};
pub use entity::{EntityId, EntityStore, GameEntity};
pub use keyword_set::{Keyword, KeywordArgs, KeywordSet};
pub use mana::{Color, ManaCost, ManaPool};
pub use mana_production::{ManaColor, ManaProduction, ManaProductionKind, ManaSideCost};
pub use persistent_effect::{
    CleanupCondition, PersistentEffect, PersistentEffectId, PersistentEffectKind, PersistentEffectStore,
};
pub use player::Player;
pub use spell_ability::SpellAbility;
pub use types::{CardName, CounterType, PlayerName, Subtype};

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
    if v >= entity::PLAYER_TARGET_BASE && v < entity::PLAYER_TARGET_BASE + 64 {
        Some(PlayerId::new(v - entity::PLAYER_TARGET_BASE))
    } else {
        None
    }
}
