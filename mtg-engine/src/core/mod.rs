//! Core game types and entities

pub mod card;
pub mod costs;
pub mod effects;
pub mod entity;
pub mod mana;
pub mod player;
pub mod spell_ability;
pub mod types;

pub use card::{Card, CardType};
pub use costs::Cost;
pub use effects::{ActivatedAbility, Effect, Keyword, TargetRef, Trigger, TriggerEvent};
pub use entity::{EntityId, EntityStore, GameEntity};
pub use mana::{Color, ManaCost, ManaPool};
pub use player::Player;
pub use spell_ability::SpellAbility;
pub use types::{CardName, CounterType, PlayerName, Subtype};

// Type aliases for strongly-typed entity IDs
/// Strongly-typed ID for Player entities
pub type PlayerId = EntityId<Player>;

/// Strongly-typed ID for Card entities
pub type CardId = EntityId<Card>;
