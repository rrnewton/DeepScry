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
}

impl SpellAbility {
    /// Get the card ID associated with this ability
    pub fn card_id(&self) -> CardId {
        match self {
            SpellAbility::PlayLand { card_id } => *card_id,
            SpellAbility::CastSpell { card_id } => *card_id,
            SpellAbility::ActivateAbility { card_id, .. } => *card_id,
            SpellAbility::CastFromExile { card_id, .. } => *card_id,
        }
    }

    /// Check if this is a land ability
    pub fn is_land_ability(&self) -> bool {
        matches!(self, SpellAbility::PlayLand { .. })
    }

    /// Check if this is a spell (includes casting from exile)
    pub fn is_spell(&self) -> bool {
        matches!(
            self,
            SpellAbility::CastSpell { .. } | SpellAbility::CastFromExile { .. }
        )
    }

    /// Check if this is casting from exile with an alternative cost
    pub fn is_cast_from_exile(&self) -> bool {
        matches!(self, SpellAbility::CastFromExile { .. })
    }

    /// Check if this is an activated ability
    pub fn is_activated_ability(&self) -> bool {
        matches!(self, SpellAbility::ActivateAbility { .. })
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
