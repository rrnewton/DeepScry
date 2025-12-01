//! Card effects and ability system

use crate::core::{CardId, PlayerId};
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;

/// Target reference for effects
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TargetRef {
    /// Target a player
    Player(PlayerId),
    /// Target a creature or other permanent
    Permanent(CardId),
    /// No target (e.g., "each player", "all creatures")
    None,
}

/// Types of permanents that can be targeted
///
/// Used by spells like Disenchant (Artifact, Enchantment) or Terror (Creature)
/// to restrict what can be legally targeted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TargetType {
    /// Any permanent (no restriction)
    Any,
    /// Must be an artifact
    Artifact,
    /// Must be an enchantment
    Enchantment,
    /// Must be a creature
    Creature,
    /// Must be a land
    Land,
    /// Must be a planeswalker
    Planeswalker,
}

impl TargetType {
    /// Check if a card matches this target type restriction
    pub fn matches(&self, card: &crate::core::Card) -> bool {
        match self {
            TargetType::Any => true,
            TargetType::Artifact => card.is_artifact(),
            TargetType::Enchantment => card.is_enchantment(),
            TargetType::Creature => card.is_creature(),
            TargetType::Land => card.is_land(),
            TargetType::Planeswalker => card.is_planeswalker(),
        }
    }
}

/// Restrictions on what types of permanents can be targeted
///
/// For spells like Disenchant ("destroy target artifact or enchantment"),
/// this would contain [Artifact, Enchantment].
/// For Terror ("destroy target creature"), this would contain [Creature].
/// An empty vec means any permanent can be targeted.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetRestriction {
    /// Valid target types (if empty, any permanent is valid)
    pub types: SmallVec<[TargetType; 2]>,
}

impl TargetRestriction {
    /// Create a restriction allowing any permanent
    pub fn any() -> Self {
        Self { types: SmallVec::new() }
    }

    /// Create a restriction from a list of target types
    pub fn from_types(types: impl IntoIterator<Item = TargetType>) -> Self {
        Self {
            types: types.into_iter().collect(),
        }
    }

    /// Check if a card matches this restriction
    ///
    /// Returns true if:
    /// - types is empty (any permanent allowed), OR
    /// - card matches at least one of the specified types
    pub fn matches(&self, card: &crate::core::Card) -> bool {
        if self.types.is_empty() {
            return true; // No restriction
        }
        self.types.iter().any(|t| t.matches(card))
    }

    /// Parse ValidTgts string from Java Forge format
    ///
    /// Examples:
    /// - "Artifact,Enchantment" -> [Artifact, Enchantment]
    /// - "Creature" -> [Creature]
    /// - "Creature.nonArtifact+nonBlack" -> [Creature] (modifiers ignored for now)
    pub fn parse(valid_tgts: &str) -> Self {
        let mut types = SmallVec::new();

        for part in valid_tgts.split(',') {
            // Strip any modifiers like ".nonArtifact+nonBlack"
            let base_type = part.split('.').next().unwrap_or(part).trim();

            match base_type {
                "Artifact" => types.push(TargetType::Artifact),
                "Enchantment" => types.push(TargetType::Enchantment),
                "Creature" => types.push(TargetType::Creature),
                "Land" => types.push(TargetType::Land),
                "Planeswalker" => types.push(TargetType::Planeswalker),
                // "Any", "Permanent", or unrecognized - allow any
                _ => {}
            }
        }

        Self { types }
    }
}

/// Basic card effects that can be executed
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Effect {
    /// Deal damage to a target
    /// Example: "Lightning Bolt deals 3 damage to any target"
    DealDamage { target: TargetRef, amount: i32 },

    /// Draw cards
    /// Example: "Draw a card"
    DrawCards { player: PlayerId, count: u8 },

    /// Gain life
    /// Example: "You gain 3 life"
    GainLife { player: PlayerId, amount: i32 },

    /// Destroy a permanent
    /// Example: "Destroy target creature" or "Destroy target artifact or enchantment"
    DestroyPermanent {
        target: CardId,
        /// Restriction on what types can be targeted (e.g., [Artifact, Enchantment] for Disenchant)
        restriction: TargetRestriction,
    },

    /// Tap a permanent
    /// Example: "Tap target creature"
    TapPermanent { target: CardId },

    /// Untap a permanent
    /// Example: "Untap target land"
    UntapPermanent { target: CardId },

    /// Pump (temporary stat boost) until end of turn
    /// Example: "Target creature gets +3/+3 until end of turn"
    PumpCreature {
        target: CardId,
        power_bonus: i32,
        toughness_bonus: i32,
    },

    /// Mill cards from library to graveyard
    /// Example: "Target player mills 3 cards"
    Mill { player: PlayerId, count: u8 },

    /// Counter a spell on the stack
    /// Example: "Counter target spell"
    CounterSpell { target: CardId },

    /// Add mana to a player's mana pool
    /// Example: "Add {G}" or "Add {C}{C}"
    AddMana {
        player: PlayerId,
        mana: crate::core::ManaCost,
    },

    /// Put counters on a permanent
    /// Example: "Put a +1/+1 counter on target creature"
    PutCounter {
        target: CardId,
        counter_type: crate::core::CounterType,
        amount: u8,
    },

    /// Remove counters from a permanent
    /// Example: "Remove a +1/+1 counter from target creature"
    RemoveCounter {
        target: CardId,
        counter_type: crate::core::CounterType,
        amount: u8,
    },

    /// Exile a permanent
    /// Example: "Exile target creature" (Swords to Plowshares)
    /// Moves a card from the battlefield to the exile zone
    ExilePermanent { target: CardId },

    /// Search library for a card and put it into a zone
    /// Example: "Search your library for a basic land card, put it onto the battlefield tapped, then shuffle"
    /// Corresponds to: AB$ ChangeZone | Origin$ Library | Destination$ Battlefield | ChangeType$ Land.Basic
    SearchLibrary {
        /// Player whose library to search
        player: PlayerId,
        /// Card type filter (e.g., "Land.Basic", "Creature", "Land")
        card_type_filter: String,
        /// Destination zone for the found card
        destination: crate::zones::Zone,
        /// Whether the card enters tapped (for battlefield)
        enters_tapped: bool,
        /// Whether to shuffle after searching
        shuffle: bool,
    },

    /// Attach Equipment to target creature
    /// Example: Spider-Suit's Equip ability
    /// Corresponds to: K:Equip:3
    /// The source_equipment field is filled in when the ability is activated
    AttachEquipment {
        /// The Equipment to attach (filled in during activation)
        source_equipment: CardId,
        /// Target creature to attach to
        target_creature: CardId,
    },

    /// Create token(s) under a player's control
    /// Example: Spider-Ham creates a Food token
    /// Corresponds to: DB$ Token | TokenAmount$ 1 | TokenScript$ c_a_food_sac | TokenOwner$ You
    CreateToken {
        /// Player who will control the tokens
        controller: PlayerId,
        /// Token script name (e.g., "c_a_food_sac" for Food token)
        token_script: String,
        /// Number of tokens to create
        amount: u8,
    },
}

/// Events that can trigger abilities
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TriggerEvent {
    /// When a card enters the battlefield
    /// Corresponds to: T:Mode$ ChangesZone | Origin$ Any | Destination$ Battlefield | ValidCard$ Card.Self
    EntersBattlefield,

    /// When a card leaves the battlefield
    /// Corresponds to: T:Mode$ ChangesZone | Origin$ Battlefield | Destination$ Any | ValidCard$ Card.Self
    LeavesBattlefield,

    /// At the beginning of upkeep
    /// Corresponds to: T:Mode$ Phase | Phase$ Upkeep | ValidPlayer$ You
    BeginningOfUpkeep,

    /// At the beginning of end step
    /// Corresponds to: T:Mode$ Phase | Phase$ EndOfTurn | ValidPlayer$ You
    BeginningOfEndStep,

    /// When a spell is cast
    /// Corresponds to: T:Mode$ SpellCast | ValidCard$ ...
    SpellCast,

    /// When a creature attacks
    /// Corresponds to: T:Mode$ Attacks | ValidCard$ Card.Self
    Attacks,

    /// When a creature blocks
    /// Corresponds to: T:Mode$ Blocks | ValidCard$ Card.Self
    Blocks,

    /// When a creature deals combat damage
    /// Corresponds to: T:Mode$ DamageDone | ValidSource$ Card.Self | CombatDamage$ True
    DealsCombatDamage,
}

/// A triggered ability that executes when an event occurs
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Trigger {
    /// The event that triggers this ability
    pub event: TriggerEvent,

    /// The effects to execute when triggered
    pub effects: Vec<Effect>,

    /// Description of the trigger (for logging)
    pub description: String,

    /// If true, this trigger only fires when the source card itself triggers the event
    /// (e.g., "When this creature enters" only fires for this specific creature)
    /// If false, triggers for any card matching the event (e.g., "When any creature enters")
    pub trigger_self_only: bool,
}

impl Trigger {
    /// Create a new trigger with trigger_self_only defaulting to true
    /// Most ETB/LTB triggers only fire for the card itself
    pub fn new(event: TriggerEvent, effects: Vec<Effect>, description: String) -> Self {
        Trigger {
            event,
            effects,
            description,
            trigger_self_only: true, // Default: only fire for this card
        }
    }

    /// Create a new trigger that fires for any card matching the event
    pub fn new_any(event: TriggerEvent, effects: Vec<Effect>, description: String) -> Self {
        Trigger {
            event,
            effects,
            description,
            trigger_self_only: false,
        }
    }
}

/// Static ability that creates continuous effects
///
/// ## CR 613: Interaction of Continuous Effects
///
/// Static abilities create continuous effects that modify characteristics
/// of game objects. They are always "on" and don't use the stack.
///
/// Example from Spider-Suit:
/// ```text
/// S:Mode$ Continuous | Affected$ Creature.EquippedBy | AddPower$ 2 | AddToughness$ 2
/// ```
///
/// This creates a continuous effect in Layer 7c (MODIFYPT) that gives
/// the equipped creature +2/+2.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StaticAbility {
    /// Continuous effect that modifies power/toughness
    ///
    /// Corresponds to: `S:Mode$ Continuous | AddPower$ X | AddToughness$ Y`
    /// Applied in CR 613 Layer 7c (MODIFYPT)
    ModifyPT {
        /// Selector for which cards are affected
        /// Example: "Creature.EquippedBy" = creature equipped by this Equipment
        /// Example: "Creature.YouCtrl" = creatures you control
        affected: AffectedSelector,

        /// Power bonus (can be negative)
        power: i32,

        /// Toughness bonus (can be negative)
        toughness: i32,

        /// Description for logging
        description: String,
    },
}

/// Selector for which cards are affected by a static ability
///
/// Parsed from the `Affected$` parameter in card scripts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AffectedSelector {
    /// The creature equipped by this Equipment
    /// Corresponds to: `Affected$ Creature.EquippedBy`
    CreatureEquippedBy,

    /// Creatures you control
    /// Corresponds to: `Affected$ Creature.YouCtrl`
    CreaturesYouControl,

    /// All creatures
    /// Corresponds to: `Affected$ Creature`
    AllCreatures,

    /// This card itself
    /// Corresponds to: `Affected$ Card.Self`
    Self_,

    /// The land this Aura is attached to
    /// Corresponds to: `Affected$ Land.AttachedBy`
    /// Used by Auras that grant abilities to enchanted lands (e.g., Friendly Neighborhood)
    LandAttachedBy,

    /// Single creature type you control (tribal lords)
    /// Corresponds to: `Affected$ Goblin.YouCtrl`, `Affected$ Zombie.YouCtrl`, etc.
    /// Used by tribal lord cards that grant bonuses to a single creature type
    CreatureTypeYouControl {
        /// The creature subtype (e.g., "Goblin", "Zombie")
        subtype: crate::core::Subtype,
    },

    /// Single creature type you control, excluding self
    /// Corresponds to: `Affected$ Goblin.Other+YouCtrl`
    /// Used by tribal lord cards that exclude themselves from the bonus
    CreatureTypeOtherYouControl {
        /// The creature subtype (e.g., "Goblin", "Zombie")
        subtype: crate::core::Subtype,
    },

    /// Multiple creature types you control, excluding self
    /// Corresponds to: `Affected$ Spider.Other+YouCtrl,Boar.Other+YouCtrl,...`
    /// Used by cards like Spider-Ham that grant bonuses to multiple creature types
    /// The `Other` qualifier excludes the source card itself
    CreatureTypesOtherYouControl {
        /// List of creature subtypes (e.g., ["Spider", "Boar", "Goat", ...])
        types: Vec<crate::core::Subtype>,
    },

    /// The creature enchanted by this Aura
    /// Corresponds to: `Affected$ Creature.EnchantedBy`
    CreatureEnchantedBy,

    /// Artifact creatures you control, excluding self
    /// Corresponds to: `Affected$ Creature.Artifact+Other+YouCtrl`
    /// Used by cards like Master of Etherium that buff artifact creatures
    CreatureCardTypeOtherYouControl {
        /// The card type (e.g., "Artifact")
        card_type: crate::core::CardType,
    },

    /// Artifact creatures you control, including self
    /// Corresponds to: `Affected$ Creature.Artifact+YouCtrl`
    CreatureCardTypeYouControl {
        /// The card type (e.g., "Artifact")
        card_type: crate::core::CardType,
    },
}

/// Cache for expensive string operations on ActivatedAbility
/// Pre-computed at ability creation time to avoid repeated allocations during gameplay
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AbilityCache {
    /// Lowercase version of description (computed once)
    pub description_lowercase: String,

    /// Pre-computed contains() checks for targeting restrictions
    pub targets_tapped: bool,
    pub targets_untapped: bool,
    pub targets_creature: bool,
    pub targets_land: bool,
    pub requires_target: bool,
}

impl AbilityCache {
    /// Create a new cache from ability description
    pub fn new(description: &str) -> Self {
        let desc_lower = description.to_lowercase();

        AbilityCache {
            // Store lowercase version
            description_lowercase: desc_lower.clone(),

            // Targeting restriction flags
            targets_tapped: desc_lower.contains("tapped"),
            targets_untapped: desc_lower.contains("untapped"),
            targets_creature: desc_lower.contains("creature"),
            targets_land: desc_lower.contains("land"),
            requires_target: desc_lower.contains("target"),
        }
    }
}

/// An activated ability that can be activated by paying a cost
/// Example: "{T}: Deal 1 damage to any target" (Prodigal Sorcerer)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivatedAbility {
    /// The cost to activate this ability
    pub cost: crate::core::Cost,

    /// The effects that execute when this ability resolves
    pub effects: Vec<Effect>,

    /// Description of the ability (for logging and display)
    pub description: String,

    /// Whether this is a mana ability (doesn't use the stack)
    pub is_mana_ability: bool,

    /// Whether this ability can only be activated at sorcery speed
    /// "Activate only as a sorcery" (CR 602.5d, CR 307.5)
    /// Requires: priority, main phase, your turn, stack empty
    pub sorcery_speed: bool,

    /// Cache for expensive string operations (computed at creation time)
    pub cache: AbilityCache,
}

impl ActivatedAbility {
    /// Create a new activated ability
    pub fn new(cost: crate::core::Cost, effects: Vec<Effect>, description: String, is_mana_ability: bool) -> Self {
        let cache = AbilityCache::new(&description);

        ActivatedAbility {
            cost,
            effects,
            description,
            is_mana_ability,
            sorcery_speed: false, // Default to instant speed
            cache,
        }
    }

    /// Create a new sorcery-speed activated ability
    pub fn new_sorcery_speed(cost: crate::core::Cost, effects: Vec<Effect>, description: String) -> Self {
        let cache = AbilityCache::new(&description);

        ActivatedAbility {
            cost,
            effects,
            description,
            is_mana_ability: false, // Sorcery-speed abilities are not mana abilities
            sorcery_speed: true,
            cache,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_effect_creation() {
        let player_id = PlayerId::new(1);
        let card_id = CardId::new(100);

        let damage_effect = Effect::DealDamage {
            target: TargetRef::Player(player_id),
            amount: 3,
        };

        match damage_effect {
            Effect::DealDamage { target, amount } => {
                assert_eq!(amount, 3);
                assert_eq!(target, TargetRef::Player(player_id));
            }
            _ => panic!("Wrong effect type"),
        }

        let draw_effect = Effect::DrawCards {
            player: player_id,
            count: 2,
        };

        match draw_effect {
            Effect::DrawCards { player, count } => {
                assert_eq!(player, player_id);
                assert_eq!(count, 2);
            }
            _ => panic!("Wrong effect type"),
        }

        let destroy_effect = Effect::DestroyPermanent {
            target: card_id,
            restriction: TargetRestriction::any(),
        };

        match destroy_effect {
            Effect::DestroyPermanent { target, .. } => {
                assert_eq!(target, card_id);
            }
            _ => panic!("Wrong effect type"),
        }
    }
}
