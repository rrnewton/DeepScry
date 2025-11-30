//! Targeting validation and target selection logic
//!
//! This module handles:
//! - Validating whether a card can be legally targeted (shroud, hexproof)
//! - Finding valid targets for spells
//! - Finding valid targets for activated abilities
//! - Checking if a cost sacrifices the source card itself

use crate::core::{CardId, Cost, Effect, PlayerId, TargetRef, TargetRestriction};
use crate::game::state::GameState;
use crate::{MtgError, Result};
use smallvec::SmallVec;

impl GameState {
    /// Check if a card can be legally targeted by a spell or ability
    ///
    /// Returns false if:
    /// - Card has shroud (cannot be targeted by anyone) (CR 702.18a)
    /// - Card has hexproof and source_controller is an opponent (CR 702.19a)
    ///
    /// # Arguments
    /// * `card` - The potential target card
    /// * `source_controller` - The controller of the spell/ability
    pub(crate) fn is_legal_target(card: &crate::core::card::Card, source_controller: PlayerId) -> bool {
        // Shroud prevents targeting by anyone (CR 702.18a)
        if card.has_shroud() {
            return false;
        }

        // Hexproof only protects from opponent's spells/abilities (CR 702.19a)
        if card.has_hexproof() && card.owner != source_controller {
            return false;
        }

        true
    }

    /// Get all valid targets for a spell card
    ///
    /// This examines the spell's effects and returns all legal targets based on:
    /// - The effect type (damage, destroy, pump, tap, untap, counter, exile)
    /// - Targeting restrictions (shroud, hexproof)
    /// - Zone restrictions (battlefield for permanents, stack for counter spells)
    ///
    /// # Arguments
    /// * `spell_card_id` - The spell card to get targets for
    ///
    /// # Returns
    /// A sorted SmallVec of valid target CardIds
    pub fn get_valid_targets_for_spell(&self, spell_card_id: CardId) -> Result<SmallVec<[CardId; 8]>> {
        let mut valid_targets = SmallVec::new();

        // Get the spell's owner, effects, and targeting restrictions from cache
        let spell_card = self.cards.get(spell_card_id)?;
        let spell_owner = spell_card.owner;
        let effects = spell_card.effects.clone(); // Clone to avoid borrow issues

        // Get cached targeting restrictions (parsed from oracle text at card load time)
        // Example: "Destroy target land" sets spell_targets_land = true
        let targets_land = spell_card.cache.spell_targets_land;
        let targets_creature = spell_card.cache.spell_targets_creature;
        let targets_any = spell_card.cache.spell_targets_any;

        // For each effect, determine what targets are valid
        for effect in &effects {
            match effect {
                Effect::DealDamage {
                    target: TargetRef::None,
                    ..
                } => {
                    // Damage can target any creature or player
                    // Add all creatures that can be legally targeted
                    for &card_id in &self.battlefield.cards {
                        if let Ok(target_card) = self.cards.get(card_id) {
                            if target_card.is_creature() && Self::is_legal_target(target_card, spell_owner) {
                                valid_targets.push(card_id);
                            }
                        }
                    }
                    // Note: Players are also valid targets, but we handle them separately
                    // via TargetRef::Player since they don't have CardIds
                }
                Effect::DestroyPermanent { target, restriction } if target.as_u32() == 0 => {
                    // Destroy effect - check targeting restrictions
                    // Priority: 1) TargetRestriction from ValidTgts, 2) CardCache from oracle text
                    for &card_id in &self.battlefield.cards {
                        if let Ok(target_card) = self.cards.get(card_id) {
                            // First check TargetRestriction from ValidTgts (if specified)
                            let restriction_matches = restriction.matches(target_card);

                            // Then check CardCache targeting flags from oracle text
                            let cache_matches = if targets_land {
                                // "Destroy target land" - only lands valid
                                target_card.is_land()
                            } else if targets_creature {
                                // "Destroy target creature" - only creatures valid
                                target_card.is_creature()
                            } else {
                                // No type restriction from oracle text
                                true
                            };

                            // Both checks must pass
                            if restriction_matches && cache_matches && Self::is_legal_target(target_card, spell_owner) {
                                valid_targets.push(card_id);
                            }
                        }
                    }
                }
                Effect::PumpCreature { target, .. } if target.as_u32() == 0 => {
                    // Pump can target any creature
                    for &card_id in &self.battlefield.cards {
                        if let Ok(target_card) = self.cards.get(card_id) {
                            if target_card.is_creature() && Self::is_legal_target(target_card, spell_owner) {
                                valid_targets.push(card_id);
                            }
                        }
                    }
                }
                Effect::TapPermanent { target } if target.as_u32() == 0 => {
                    // Tap can target untapped permanents
                    for &card_id in &self.battlefield.cards {
                        if let Ok(target_card) = self.cards.get(card_id) {
                            if !target_card.tapped && Self::is_legal_target(target_card, spell_owner) {
                                valid_targets.push(card_id);
                            }
                        }
                    }
                }
                Effect::UntapPermanent { target } if target.as_u32() == 0 => {
                    // Untap can target tapped permanents
                    for &card_id in &self.battlefield.cards {
                        if let Ok(target_card) = self.cards.get(card_id) {
                            if target_card.tapped && Self::is_legal_target(target_card, spell_owner) {
                                valid_targets.push(card_id);
                            }
                        }
                    }
                }
                Effect::CounterSpell { target } if target.as_u32() == 0 => {
                    // Counter can target spells on the stack (except self)
                    for &card_id in &self.stack.cards {
                        if card_id != spell_card_id {
                            valid_targets.push(card_id);
                        }
                    }
                }
                Effect::ExilePermanent { target } if target.as_u32() == 0 => {
                    // Exile can target any permanent (typically creatures, like Swords to Plowshares)
                    // Use cached targeting restrictions like DestroyPermanent
                    for &card_id in &self.battlefield.cards {
                        if let Ok(target_card) = self.cards.get(card_id) {
                            // Check targeting restrictions based on card text
                            let type_matches = if targets_land {
                                target_card.is_land()
                            } else if targets_creature || targets_any {
                                // Most exile spells target creatures (Swords to Plowshares)
                                // or "any target" (for damage that exiles)
                                target_card.is_creature()
                            } else {
                                // Default: only creatures (safest assumption for exile effects)
                                target_card.is_creature()
                            };

                            if type_matches && Self::is_legal_target(target_card, spell_owner) {
                                valid_targets.push(card_id);
                            }
                        }
                    }
                }
                _ => {
                    // Other effects either don't need targets or already have them specified
                    // (DrawCards, GainLife, Mill, AddMana all specify player directly)
                }
            }
        }

        // Sort for deterministic ordering (critical for snapshot/resume)
        valid_targets.sort();
        Ok(valid_targets)
    }

    /// Check if a cost sacrifices the source card itself
    ///
    /// Returns true if the cost includes sacrificing "CARDNAME" (the source card itself).
    /// For example, Strip Mine has cost "T, Sac<1/CARDNAME>" which sacrifices itself.
    fn cost_sacrifices_self(cost: &Cost) -> bool {
        match cost {
            Cost::SacrificePattern { card_type, .. } => card_type.eq_ignore_ascii_case("CARDNAME"),
            Cost::Composite(costs) => costs.iter().any(Self::cost_sacrifices_self),
            _ => false,
        }
    }

    /// Get valid targets for an activated ability
    ///
    /// Similar to get_valid_targets_for_spell(), but for activated abilities.
    /// This handles special restrictions like Royal Assassin's "target tapped creature".
    ///
    /// # Arguments
    /// * `source_card_id` - The card with the activated ability
    /// * `ability_index` - The index of the ability in the card's activated_abilities vec
    ///
    /// # Returns
    /// A sorted SmallVec of valid target CardIds
    pub fn get_valid_targets_for_ability(
        &self,
        source_card_id: CardId,
        ability_index: usize,
    ) -> Result<SmallVec<[CardId; 8]>> {
        let mut valid_targets = SmallVec::new();

        // Get the source card and ability
        let source_card = self.cards.get(source_card_id)?;
        let ability_controller = source_card.controller;

        let ability = source_card.activated_abilities.get(ability_index).ok_or_else(|| {
            MtgError::InvalidAction(format!(
                "Ability index {} out of bounds for card {}",
                ability_index, source_card_id
            ))
        })?;

        // Check if the ability sacrifices the source card itself (e.g., Strip Mine)
        // If so, the source card won't be on the battlefield when the effect resolves
        let sacrifices_self = Self::cost_sacrifices_self(&ability.cost);

        // Check for targeting restrictions in the ability description
        // For Royal Assassin: "Destroy target tapped creature"
        // For Strip Mine: "Destroy target land"
        // Use cached values to avoid allocation
        let requires_tapped = ability.cache.targets_tapped;
        let requires_untapped = ability.cache.targets_untapped;
        let targets_creature = ability.cache.targets_creature;
        let targets_land = ability.cache.targets_land;

        // Check each effect to determine valid targets
        for effect in &ability.effects {
            match effect {
                Effect::DestroyPermanent { target, restriction } if target.as_u32() == 0 => {
                    // Destroy effect needs targets matching restriction
                    for &card_id in &self.battlefield.cards {
                        if let Ok(card) = self.cards.get(card_id) {
                            // Check targeting restrictions
                            let mut is_valid = true;

                            // Cannot target the source card if it will be sacrificed as part of the cost
                            // (e.g., Strip Mine sacrifices itself, so it can't be the target)
                            if sacrifices_self && card_id == source_card_id {
                                is_valid = false;
                            }

                            // Check spell-level type restriction from ValidTgts
                            if !restriction.matches(card) {
                                is_valid = false;
                            }

                            // Must be creature if ability says "creature"
                            if targets_creature && !card.is_creature() {
                                is_valid = false;
                            }

                            // Must be land if ability says "land"
                            if targets_land && !card.is_land() {
                                is_valid = false;
                            }

                            // Must be tapped if ability says "tapped"
                            if requires_tapped && !card.tapped {
                                is_valid = false;
                            }

                            // Must be untapped if ability says "untapped"
                            if requires_untapped && card.tapped {
                                is_valid = false;
                            }

                            // Check shroud/hexproof (CR 702.18, 702.19)
                            if !Self::is_legal_target(card, ability_controller) {
                                is_valid = false;
                            }

                            if is_valid {
                                valid_targets.push(card_id);
                            }
                        }
                    }
                }
                Effect::TapPermanent { target } if target.as_u32() == 0 => {
                    // Tap can target untapped permanents
                    for &card_id in &self.battlefield.cards {
                        if let Ok(card) = self.cards.get(card_id) {
                            let is_valid = !card.tapped && Self::is_legal_target(card, ability_controller);

                            if is_valid {
                                valid_targets.push(card_id);
                            }
                        }
                    }
                }
                Effect::UntapPermanent { target } if target.as_u32() == 0 => {
                    // Untap can target tapped permanents
                    for &card_id in &self.battlefield.cards {
                        if let Ok(card) = self.cards.get(card_id) {
                            let is_valid = card.tapped && Self::is_legal_target(card, ability_controller);

                            if is_valid {
                                valid_targets.push(card_id);
                            }
                        }
                    }
                }
                Effect::DealDamage {
                    target: TargetRef::None,
                    ..
                } => {
                    // Damage can target creatures
                    for &card_id in &self.battlefield.cards {
                        if let Ok(card) = self.cards.get(card_id) {
                            let is_valid = card.is_creature() && Self::is_legal_target(card, ability_controller);

                            if is_valid {
                                valid_targets.push(card_id);
                            }
                        }
                    }
                }
                Effect::AttachEquipment { target_creature, .. } if target_creature.as_u32() == 0 => {
                    // Equip targets "creature you control" (CR 702.6a)
                    for &card_id in &self.battlefield.cards {
                        if let Ok(card) = self.cards.get(card_id) {
                            // Must be a creature
                            let mut is_valid = card.is_creature();

                            // Must be controlled by the ability's controller
                            if card.controller != ability_controller {
                                is_valid = false;
                            }

                            // Check shroud/hexproof (CR 702.18, 702.19)
                            // Note: Hexproof doesn't typically apply when we control both the Equipment
                            // and the target, but we check owner-based targeting for consistency
                            if !Self::is_legal_target(card, ability_controller) {
                                is_valid = false;
                            }

                            if is_valid {
                                valid_targets.push(card_id);
                            }
                        }
                    }
                }
                _ => {
                    // Other effects either don't need targets or already have them specified
                }
            }
        }

        // Sort for deterministic ordering (critical for snapshot/resume)
        valid_targets.sort();
        Ok(valid_targets)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{Card, CardType, Effect, PlayerId};

    /// Test that CardCache correctly parses spell targeting restrictions
    #[test]
    fn test_card_cache_spell_targets_land() {
        // Sinkhole: "Destroy target land."
        let cache = crate::core::CardCache::new("Destroy target land.", "Sinkhole");
        assert!(cache.spell_targets_land, "Sinkhole should have spell_targets_land=true");
        assert!(!cache.spell_targets_creature, "Sinkhole should not target creatures");
        assert!(!cache.spell_targets_any, "Sinkhole should not target any");
    }

    #[test]
    fn test_card_cache_spell_targets_creature() {
        // Terror: "Destroy target nonartifact, nonblack creature."
        let cache = crate::core::CardCache::new(
            "Destroy target nonartifact, nonblack creature. It can't be regenerated.",
            "Terror",
        );
        assert!(
            cache.spell_targets_creature,
            "Terror should have spell_targets_creature=true"
        );
        assert!(!cache.spell_targets_land, "Terror should not target lands");
    }

    #[test]
    fn test_card_cache_spell_targets_any() {
        // Lightning Bolt: "Lightning Bolt deals 3 damage to any target."
        let cache = crate::core::CardCache::new("Lightning Bolt deals 3 damage to any target.", "Lightning Bolt");
        assert!(
            cache.spell_targets_any,
            "Lightning Bolt should have spell_targets_any=true"
        );
        assert!(
            !cache.spell_targets_land,
            "Lightning Bolt with any target should not have land-only restriction"
        );
        assert!(
            !cache.spell_targets_creature,
            "Lightning Bolt with any target should not have creature-only restriction"
        );
    }

    #[test]
    fn test_card_cache_spell_targets_player() {
        // Ancestral Recall: "Target player draws three cards."
        let cache = crate::core::CardCache::new("Target player draws three cards.", "Ancestral Recall");
        assert!(
            cache.spell_targets_player,
            "Ancestral Recall should have spell_targets_player=true"
        );
    }

    #[test]
    fn test_card_cache_mixed_targets_any_wins() {
        // A hypothetical card that says "any target" should not be restricted
        let cache = crate::core::CardCache::new("Deal 3 damage to any target land or creature.", "Hypothetical");
        // "any target" should take precedence
        assert!(cache.spell_targets_any);
        // When "any target" is present, creature/land flags should be false
        assert!(
            !cache.spell_targets_creature,
            "any target should override creature flag"
        );
    }

    #[test]
    fn test_sinkhole_cannot_target_creatures() {
        use crate::game::state::GameState;

        // Create a minimal game state to test targeting
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let player1 = PlayerId::new(0);
        let player2 = PlayerId::new(1);

        // Add a creature to the battlefield
        let creature_id = game.cards.next_id();
        let mut creature_card = Card::new(creature_id, "Grizzly Bears", player2);
        creature_card.types.push(CardType::Creature);
        game.cards.insert(creature_id, creature_card);
        game.battlefield.cards.push(creature_id);

        // Add a land to the battlefield
        let land_id = game.cards.next_id();
        let mut land_card = Card::new(land_id, "Swamp", player2);
        land_card.types.push(CardType::Land);
        game.cards.insert(land_id, land_card);
        game.battlefield.cards.push(land_id);

        // Create Sinkhole spell card with oracle text
        let sinkhole_id = game.cards.next_id();
        let mut sinkhole_card = Card::new(sinkhole_id, "Sinkhole", player1);
        sinkhole_card.types.push(CardType::Sorcery);
        sinkhole_card.text = "Destroy target land.".to_string();
        sinkhole_card.cache = crate::core::CardCache::new(&sinkhole_card.text, sinkhole_card.name.as_str());
        sinkhole_card.effects.push(Effect::DestroyPermanent {
            target: CardId::new(0),
            restriction: crate::core::TargetRestriction::any(),
        });
        game.cards.insert(sinkhole_id, sinkhole_card);

        // Get valid targets for Sinkhole
        let targets = game.get_valid_targets_for_spell(sinkhole_id).unwrap();

        // Should only include the land, not the creature
        assert!(targets.contains(&land_id), "Sinkhole should be able to target lands");
        assert!(
            !targets.contains(&creature_id),
            "Sinkhole should NOT be able to target creatures (this was the bug!)"
        );
    }

    #[test]
    fn test_terror_cannot_target_lands() {
        use crate::game::state::GameState;

        // Create a minimal game state to test targeting
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let player1 = PlayerId::new(0);
        let player2 = PlayerId::new(1);

        // Add a creature to the battlefield
        let creature_id = game.cards.next_id();
        let mut creature_card = Card::new(creature_id, "Grizzly Bears", player2);
        creature_card.types.push(CardType::Creature);
        game.cards.insert(creature_id, creature_card);
        game.battlefield.cards.push(creature_id);

        // Add a land to the battlefield
        let land_id = game.cards.next_id();
        let mut land_card = Card::new(land_id, "Swamp", player2);
        land_card.types.push(CardType::Land);
        game.cards.insert(land_id, land_card);
        game.battlefield.cards.push(land_id);

        // Create Terror spell card with oracle text
        let terror_id = game.cards.next_id();
        let mut terror_card = Card::new(terror_id, "Terror", player1);
        terror_card.types.push(CardType::Instant);
        terror_card.text = "Destroy target nonartifact, nonblack creature. It can't be regenerated.".to_string();
        terror_card.cache = crate::core::CardCache::new(&terror_card.text, terror_card.name.as_str());
        terror_card.effects.push(Effect::DestroyPermanent {
            target: CardId::new(0),
            restriction: crate::core::TargetRestriction::any(),
        });
        game.cards.insert(terror_id, terror_card);

        // Get valid targets for Terror
        let targets = game.get_valid_targets_for_spell(terror_id).unwrap();

        // Should only include the creature, not the land
        assert!(
            targets.contains(&creature_id),
            "Terror should be able to target creatures"
        );
        assert!(!targets.contains(&land_id), "Terror should NOT be able to target lands");
    }
}
