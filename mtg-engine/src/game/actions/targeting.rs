//! Targeting validation and target selection logic
//!
//! This module handles:
//! - Validating whether a card can be legally targeted (shroud, hexproof)
//! - Finding valid targets for spells
//! - Finding valid targets for activated abilities
//! - Checking if a cost sacrifices the source card itself

use crate::core::{CardId, Cost, Effect, PlayerId, TargetRef};
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

        // Get the spell's owner and effects
        let card = self.cards.get(spell_card_id)?;
        let spell_owner = card.owner;
        let effects = &card.effects;

        // For each effect, determine what targets are valid
        for effect in effects {
            match effect {
                Effect::DealDamage {
                    target: TargetRef::None,
                    ..
                } => {
                    // Damage can target any creature or player
                    // Add all creatures that can be legally targeted
                    for &card_id in &self.battlefield.cards {
                        if let Ok(card) = self.cards.get(card_id) {
                            if card.is_creature() && Self::is_legal_target(card, spell_owner) {
                                valid_targets.push(card_id);
                            }
                        }
                    }
                    // Note: Players are also valid targets, but we handle them separately
                    // via TargetRef::Player since they don't have CardIds
                }
                Effect::DestroyPermanent { target } if target.as_u32() == 0 => {
                    // Destroy can target any permanent (typically creatures)
                    for &card_id in &self.battlefield.cards {
                        if let Ok(card) = self.cards.get(card_id) {
                            if Self::is_legal_target(card, spell_owner) {
                                valid_targets.push(card_id);
                            }
                        }
                    }
                }
                Effect::PumpCreature { target, .. } if target.as_u32() == 0 => {
                    // Pump can target any creature
                    for &card_id in &self.battlefield.cards {
                        if let Ok(card) = self.cards.get(card_id) {
                            if card.is_creature() && Self::is_legal_target(card, spell_owner) {
                                valid_targets.push(card_id);
                            }
                        }
                    }
                }
                Effect::TapPermanent { target } if target.as_u32() == 0 => {
                    // Tap can target untapped permanents
                    for &card_id in &self.battlefield.cards {
                        if let Ok(card) = self.cards.get(card_id) {
                            if !card.tapped && Self::is_legal_target(card, spell_owner) {
                                valid_targets.push(card_id);
                            }
                        }
                    }
                }
                Effect::UntapPermanent { target } if target.as_u32() == 0 => {
                    // Untap can target tapped permanents
                    for &card_id in &self.battlefield.cards {
                        if let Ok(card) = self.cards.get(card_id) {
                            if card.tapped && Self::is_legal_target(card, spell_owner) {
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
                    // In Swords to Plowshares: ValidTgts$ Creature
                    for &card_id in &self.battlefield.cards {
                        if let Ok(card) = self.cards.get(card_id) {
                            // For now, target any permanent that doesn't have shroud
                            // Swords to Plowshares specifically targets creatures
                            if card.is_creature() && Self::is_legal_target(card, spell_owner) {
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
                Effect::DestroyPermanent { target } if target.as_u32() == 0 => {
                    // Destroy effect needs targets
                    for &card_id in &self.battlefield.cards {
                        if let Ok(card) = self.cards.get(card_id) {
                            // Check targeting restrictions
                            let mut is_valid = true;

                            // Cannot target the source card if it will be sacrificed as part of the cost
                            // (e.g., Strip Mine sacrifices itself, so it can't be the target)
                            if sacrifices_self && card_id == source_card_id {
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
