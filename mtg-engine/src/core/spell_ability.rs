//! Spell ability representation
//!
//! A SpellAbility represents any playable action a player can take:
//! - Playing a land
//! - Casting a spell
//! - Activating an ability
//! - Casting from exile with an alternative cost (Airbend, Suspend, etc.)
//!
//! This matches the Java Forge SpellAbility hierarchy.

use crate::core::{CardId, ManaCost, PersistentEffectId};

/// A playable ability that can be chosen by a controller
///
/// Matches the Java Forge SpellAbility concept where lands, spells, and
/// activated abilities are all represented as spell abilities that can be
/// chosen from a unified list.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SpellAbility {
    /// Play a land card from hand
    ///
    /// Lands don't use the stack - they resolve immediately when played.
    /// A player can normally play one land per turn during a main phase.
    PlayLand { card_id: CardId },

    /// Cast a spell from hand
    ///
    /// Spells go on the stack and follow the 8-step casting process:
    /// 1. Propose (move to stack)
    /// 2. Make choices (modes, X values)
    /// 3. Choose targets
    /// 4. Divide effects
    /// 5. Determine total cost
    /// 6. Activate mana abilities (tap lands for mana)
    /// 7. Pay costs
    /// 8. Spell becomes cast (trigger abilities)
    CastSpell { card_id: CardId },

    /// Activate an ability of a permanent
    ///
    /// Activated abilities have a cost and an effect, formatted as
    /// "\[Cost\]: \[Effect\]" on the card. For example, tapping a creature
    /// to deal damage.
    ///
    /// The ability_index distinguishes multiple abilities on the same card.
    ActivateAbility { card_id: CardId, ability_index: usize },

    /// Cast a spell from exile with an alternative cost
    ///
    /// Used by Airbend, Suspend, and similar effects that allow casting
    /// from exile with a different mana cost than printed.
    ///
    /// When this resolves:
    /// 1. Pay the alternative_cost instead of the card's mana cost
    /// 2. The card moves from exile to the stack
    /// 3. Resolution proceeds normally
    /// 4. The associated PersistentEffect is cleaned up
    CastFromExile {
        card_id: CardId,
        /// The alternative cost to pay (e.g., {2} for Airbend)
        alternative_cost: ManaCost,
        /// The persistent effect that grants this cast permission
        effect_id: PersistentEffectId,
    },

    /// Cast the commander from the command zone (Commander format)
    ///
    /// The commander can always be cast from the command zone by paying its
    /// mana cost plus the commander tax ({2} per previous cast from command zone).
    /// MTG CR 903.8.
    CastFromCommand {
        card_id: CardId,
        /// The total cost to cast (base cost + commander tax)
        total_cost: ManaCost,
    },

    /// Activate a cycling ability from hand
    ///
    /// Cycling abilities are activated from hand (not battlefield).
    /// When activated:
    /// 1. Pay the cycling cost
    /// 2. Discard the card
    /// 3. For regular Cycling: draw a card
    /// 4. For Typecycling: search library for a card of that type
    ///
    /// MTG CR 702.29: "Cycling is an activated ability that functions only
    /// while the card with cycling is in a player's hand."
    Cycle {
        card_id: CardId,
        /// The mana cost to activate cycling
        cost: ManaCost,
        /// For Typecycling: the type to search for (e.g., "Mountain")
        /// None for regular cycling (just draw a card)
        search_type: Option<crate::core::Subtype>,
    },

    /// Cast a creature spell from graveyard with a MayPlayFromGraveyard effect
    ///
    /// Used by Leonardo, Sewer Samurai: "During your turn, you may cast creature
    /// spells with power or toughness 1 or less from your graveyard. If you cast
    /// a spell this way, that creature enters with a finality counter on it."
    CastFromGraveyard {
        card_id: CardId,
        /// The persistent effect granting this permission
        effect_id: PersistentEffectId,
        /// If true, the creature enters with a finality counter
        add_finality_counter: bool,
    },

    /// Cast the Adventure (instant/sorcery) half of an Adventurer card from hand.
    ///
    /// Adventurer cards (CR 715) are creatures that also have an Adventure — an
    /// alternate instant/sorcery spell. The card is held in hand as the creature;
    /// this ability casts the Adventure face instead. When the Adventure spell
    /// resolves it is EXILED with an "on an adventure" marker (rather than going
    /// to the graveyard); while exiled that way, the owner may cast the CREATURE
    /// half from exile (offered as a `MayPlayFromExile` cast).
    ///
    /// Dispatch reuses the normal `CastSpell` casting pipeline: the priority loop
    /// swaps the card's spell-relevant characteristics (name, cost, types,
    /// effects, ...) to the Adventure face before running the standard 8-step
    /// cast, so targeting / cost / modal / X handling are shared, not duplicated.
    CastAdventure { card_id: CardId },

    /// Cast a spell from hand with an alternative cost.
    ///
    /// Used by cards like Summoning Trap that may be cast for a different (usually
    /// cheaper) mana cost when a specific condition is met — e.g. "You may cast this
    /// spell for {0} if a creature spell you cast this turn was countered."
    ///
    /// The alternative cost replaces the card's normal mana cost during the 8-step
    /// casting process; all other steps (targeting, etc.) proceed normally.
    CastFromHandWithAltCost {
        card_id: CardId,
        /// The alternative mana cost to pay instead of the card's printed cost.
        alternative_cost: ManaCost,
    },

    /// Cast the top card of the library as a spell (Experimental Frenzy, Future Sight).
    ///
    /// The card moves from the library to the stack and resolves normally,
    /// paying its printed mana cost (no alternative cost). The card must be
    /// the top card of the controller's library.
    ///
    /// MTG CR 702.150 (Future Sight); CR 601 applies normally after the zone grant.
    CastFromLibrary { card_id: CardId },

    /// Play the top card of the library as a land (Experimental Frenzy, Future Sight).
    ///
    /// The land moves from the library directly to the battlefield (or is
    /// played normally), consuming the player's land-play for the turn.
    PlayLandFromLibrary { card_id: CardId },
}

impl SpellAbility {
    /// Get the card ID associated with this ability
    pub fn card_id(&self) -> CardId {
        match self {
            SpellAbility::PlayLand { card_id }
            | SpellAbility::CastSpell { card_id }
            | SpellAbility::CastFromExile { card_id, .. }
            | SpellAbility::CastFromCommand { card_id, .. }
            | SpellAbility::Cycle { card_id, .. } => *card_id,
            SpellAbility::ActivateAbility { card_id, .. } => *card_id,
            SpellAbility::CastFromGraveyard { card_id, .. } => *card_id,
            SpellAbility::CastAdventure { card_id } => *card_id,
            SpellAbility::CastFromHandWithAltCost { card_id, .. } => *card_id,
            SpellAbility::CastFromLibrary { card_id } | SpellAbility::PlayLandFromLibrary { card_id } => *card_id,
        }
    }

    /// Check if this is a land ability
    pub fn is_land_ability(&self) -> bool {
        matches!(self, SpellAbility::PlayLand { .. })
    }

    /// Check if this is a spell (includes casting from exile, command zone, graveyard, or library)
    pub fn is_spell(&self) -> bool {
        matches!(
            self,
            SpellAbility::CastSpell { .. }
                | SpellAbility::CastFromExile { .. }
                | SpellAbility::CastFromCommand { .. }
                | SpellAbility::CastFromGraveyard { .. }
                | SpellAbility::CastAdventure { .. }
                | SpellAbility::CastFromHandWithAltCost { .. }
                | SpellAbility::CastFromLibrary { .. }
        )
    }

    /// Check if this is casting the Adventure (instant/sorcery) face from hand.
    pub fn is_cast_adventure(&self) -> bool {
        matches!(self, SpellAbility::CastAdventure { .. })
    }

    /// Check if this is casting from the command zone
    pub fn is_cast_from_command(&self) -> bool {
        matches!(self, SpellAbility::CastFromCommand { .. })
    }

    /// Check if this is casting from exile with an alternative cost
    pub fn is_cast_from_exile(&self) -> bool {
        matches!(self, SpellAbility::CastFromExile { .. })
    }

    /// Check if this is an activated ability
    pub fn is_activated_ability(&self) -> bool {
        matches!(self, SpellAbility::ActivateAbility { .. })
    }

    /// Check if this is a cycling ability
    pub fn is_cycling_ability(&self) -> bool {
        matches!(self, SpellAbility::Cycle { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::EntityId;

    #[test]
    fn test_spell_ability_creation() {
        let card_id = EntityId::new(1);

        let land = SpellAbility::PlayLand { card_id };
        assert!(land.is_land_ability());
        assert!(!land.is_spell());
        assert_eq!(land.card_id(), card_id);

        let spell = SpellAbility::CastSpell { card_id };
        assert!(spell.is_spell());
        assert!(!spell.is_land_ability());
        assert_eq!(spell.card_id(), card_id);

        let ability = SpellAbility::ActivateAbility {
            card_id,
            ability_index: 0,
        };
        assert!(ability.is_activated_ability());
        assert_eq!(ability.card_id(), card_id);
    }
}
