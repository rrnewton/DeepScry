//! Core game types and entities

pub mod card;
pub mod costs;
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
pub use effects::{
    AbilityCache, ActivatedAbility, AffectedSelector, Effect, StaticAbility, TargetRef, TargetRestriction, TargetType,
    Trigger, TriggerEvent,
};
pub use entity::{EntityId, EntityStore, GameEntity};
pub use keyword_set::{Keyword, KeywordArgs, KeywordSet};
pub use mana::{Color, ManaCost, ManaPool};
pub use mana_production::{ManaColor, ManaProduction, ManaProductionKind};
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
