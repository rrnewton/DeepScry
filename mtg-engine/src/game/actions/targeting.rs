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

/// Check if a card can be legally targeted by a spell or ability.
///
/// Returns false if:
/// - Card has shroud (cannot be targeted by anyone) (CR 702.18a)
/// - Card has hexproof and source_controller is an opponent (CR 702.19a)
///
/// # Arguments
/// * `card` - The potential target card
/// * `source_controller` - The controller of the targeting spell/ability
///
/// # Note
///
/// This is the canonical targeting validation check. All targeting code
/// should use this function instead of inline hexproof/shroud checks.
/// Per CR 702.19a, hexproof protects from spells/abilities controlled by
/// opponents (using card.controller, not card.owner).
///
/// # Example
///
/// ```ignore
/// // In a closure filtering valid targets:
/// .filter(|card| is_legal_target(card, controller))
/// ```
#[inline]
pub fn is_legal_target(card: &crate::core::card::Card, source_controller: PlayerId) -> bool {
    // Shroud prevents targeting by anyone (CR 702.18a)
    if card.has_shroud() {
        return false;
    }

    // Hexproof only protects from opponent's spells/abilities (CR 702.19a)
    // Note: Uses controller, not owner - hexproof protects from opponent CONTROLLERS
    if card.has_hexproof() && card.controller != source_controller {
        return false;
    }

    true
}

impl GameState {
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
    ///
    /// # Errors
    ///
    /// Returns an error if the spell card cannot be found.
    pub fn get_valid_targets_for_spell(&self, spell_card_id: CardId) -> Result<SmallVec<[CardId; 8]>> {
        let mut valid_targets = SmallVec::new();

        // Get the spell's owner, effects count, and targeting restrictions from cache
        // Extract primitives first to avoid holding a borrow while iterating
        let (spell_owner, num_effects, targets_land, targets_creature, targets_any) = {
            let spell_card = self.cards.get(spell_card_id)?;
            (
                spell_card.owner,
                spell_card.effects.len(),
                spell_card.cache.spell_targets_land,
                spell_card.cache.spell_targets_creature,
                spell_card.cache.spell_targets_any,
            )
        };

        // For each effect, determine what targets are valid
        // Use index-based iteration to avoid cloning the effects Vec
        for effect_idx in 0..num_effects {
            // Re-fetch effect each iteration - this is just a Vec index lookup
            let effect = &self.cards.get(spell_card_id)?.effects[effect_idx];
            match effect {
                Effect::DealDamage {
                    target: TargetRef::None,
                    ..
                } => {
                    // Damage can target any creature or player
                    // Add all creatures that can be legally targeted
                    for &card_id in &self.battlefield.cards {
                        if let Ok(target_card) = self.cards.get(card_id) {
                            if target_card.is_creature() && is_legal_target(target_card, spell_owner) {
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
                            if restriction_matches && cache_matches && is_legal_target(target_card, spell_owner) {
                                valid_targets.push(card_id);
                            }
                        }
                    }
                }
                Effect::PumpCreature { target, .. } if target.as_u32() == 0 => {
                    // Pump can target any creature
                    for &card_id in &self.battlefield.cards {
                        if let Ok(target_card) = self.cards.get(card_id) {
                            if target_card.is_creature() && is_legal_target(target_card, spell_owner) {
                                valid_targets.push(card_id);
                            }
                        }
                    }
                }
                Effect::TapPermanent { target } if target.as_u32() == 0 => {
                    // Tap can target untapped permanents
                    for &card_id in &self.battlefield.cards {
                        if let Ok(target_card) = self.cards.get(card_id) {
                            if !target_card.tapped && is_legal_target(target_card, spell_owner) {
                                valid_targets.push(card_id);
                            }
                        }
                    }
                }
                Effect::UntapPermanent { target } if target.as_u32() == 0 => {
                    // Untap can target tapped permanents
                    for &card_id in &self.battlefield.cards {
                        if let Ok(target_card) = self.cards.get(card_id) {
                            if target_card.tapped && is_legal_target(target_card, spell_owner) {
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

                            if type_matches && is_legal_target(target_card, spell_owner) {
                                valid_targets.push(card_id);
                            }
                        }
                    }
                }
                Effect::Airbend { target } if target.as_u32() == 0 => {
                    // Airbend targets creatures (CR 701.65b)
                    // Some Airbend cards target "nonland permanent" but creature is default
                    for &card_id in &self.battlefield.cards {
                        if let Ok(target_card) = self.cards.get(card_id) {
                            // Use cached targeting flags or default to creatures
                            let type_matches = if targets_any {
                                // "airbend target nonland permanent" or similar
                                !target_card.is_land()
                            } else {
                                // Default: creatures only
                                target_card.is_creature()
                            };

                            if type_matches && is_legal_target(target_card, spell_owner) {
                                valid_targets.push(card_id);
                            }
                        }
                    }
                }
                Effect::GrantCantBeBlocked { target } if target.as_u32() == 0 => {
                    // GrantCantBeBlocked targets creatures you control
                    // Deserter's Disciple: "Another target creature you control with power 2 or less"
                    // For now, we support basic creature targeting - power restriction checked at ability parse time
                    for &card_id in &self.battlefield.cards {
                        if let Ok(target_card) = self.cards.get(card_id) {
                            // Must be a creature we control
                            if target_card.is_creature()
                                && target_card.controller == spell_owner
                                && is_legal_target(target_card, spell_owner)
                            {
                                // Note: "Other" and power restrictions are validated at ability level
                                // via ValidTgts$ parsing (e.g., Creature.Other+YouCtrl+powerLE2)
                                valid_targets.push(card_id);
                            }
                        }
                    }
                }
                Effect::RemoveCounter { target, .. } if target.as_u32() == 0 => {
                    // RemoveCounter targets creatures (e.g., Heartless Act mode 2)
                    // TODO: Some RemoveCounter effects can target any permanent
                    for &card_id in &self.battlefield.cards {
                        if let Ok(target_card) = self.cards.get(card_id) {
                            if target_card.is_creature() && is_legal_target(target_card, spell_owner) {
                                valid_targets.push(card_id);
                            }
                        }
                    }
                }
                Effect::CreateDelayedTrigger { tracked_card, .. } if tracked_card.as_u32() == 0 => {
                    // CreateDelayedTrigger targets creatures (e.g., Fatal Fissure: "Choose target creature")
                    // The tracked_card field holds the target that will be watched for death
                    for &card_id in &self.battlefield.cards {
                        if let Ok(target_card) = self.cards.get(card_id) {
                            // Default to targeting any creature (Fatal Fissure targets any creature)
                            // Controller restriction from ValidTgts$ is checked at ability level
                            if target_card.is_creature() && is_legal_target(target_card, spell_owner) {
                                valid_targets.push(card_id);
                            }
                        }
                    }
                }
                Effect::PutCounter { target, .. } if target.as_u32() == 0 => {
                    // PutCounter targets creatures (e.g., +1/+1 counter effects)
                    // TODO: Some PutCounter effects can target any permanent
                    for &card_id in &self.battlefield.cards {
                        if let Ok(target_card) = self.cards.get(card_id) {
                            if target_card.is_creature() && is_legal_target(target_card, spell_owner) {
                                valid_targets.push(card_id);
                            }
                        }
                    }
                }
                Effect::CopyPermanent {
                    target, restriction, ..
                } if target.as_u32() == 0 => {
                    // CopyPermanent targets creatures with controller restrictions
                    // Cackling Counterpart: "target creature you control" (YouCtrl)
                    // Ember Island Production mode 2: "target creature an opponent controls" (OppCtrl)
                    for &card_id in &self.battlefield.cards {
                        if let Ok(target_card) = self.cards.get(card_id) {
                            // Use restriction which includes type AND controller filtering
                            // Default: creatures if no type specified in restriction
                            let type_matches = if targets_any {
                                // "Copy target permanent" - any permanent
                                true
                            } else if restriction.types.is_empty() {
                                // Default to creatures when no explicit type restriction
                                target_card.is_creature()
                            } else {
                                // Use restriction's type filtering
                                restriction.matches(target_card)
                            };

                            // Check controller restriction (YouCtrl, OppCtrl, Any)
                            let controller_matches =
                                restriction.matches_with_controller(target_card, spell_owner, target_card.owner);

                            if type_matches && controller_matches && is_legal_target(target_card, spell_owner) {
                                valid_targets.push(card_id);
                            }
                        }
                    }
                }
                Effect::ModalChoice { modes, .. } => {
                    // Modal spells: Mode selection should happen BEFORE targeting.
                    // When this code runs, modes should already be selected and the
                    // ModalChoice effect replaced with the selected mode's effects.
                    //
                    // If we reach here, it means mode selection hasn't happened yet.
                    // For now, collect targets from ALL modes (will be filtered later).
                    for mode in modes {
                        match mode.effect.as_ref() {
                            Effect::DestroyPermanent { target, restriction } if target.as_u32() == 0 => {
                                // This mode destroys a permanent
                                for &card_id in &self.battlefield.cards {
                                    if let Ok(target_card) = self.cards.get(card_id) {
                                        if restriction.matches(target_card) && is_legal_target(target_card, spell_owner)
                                        {
                                            valid_targets.push(card_id);
                                        }
                                    }
                                }
                            }
                            Effect::CopyPermanent {
                                target, restriction, ..
                            } if target.as_u32() == 0 => {
                                // This mode copies a permanent (e.g., Ember Island Production)
                                // Use restriction for controller filtering (YouCtrl, OppCtrl)
                                for &card_id in &self.battlefield.cards {
                                    if let Ok(target_card) = self.cards.get(card_id) {
                                        // Check type (default to creature) and controller restriction
                                        let type_matches = if restriction.types.is_empty() {
                                            target_card.is_creature()
                                        } else {
                                            restriction.matches(target_card)
                                        };
                                        let controller_matches = restriction.matches_with_controller(
                                            target_card,
                                            spell_owner,
                                            target_card.owner,
                                        );
                                        if type_matches
                                            && controller_matches
                                            && is_legal_target(target_card, spell_owner)
                                        {
                                            valid_targets.push(card_id);
                                        }
                                    }
                                }
                            }
                            // Modal effects that don't need permanent/creature targets
                            // (target players, self, or have targets pre-specified)
                            Effect::DealDamage { .. }
                            | Effect::DrawCards { .. }
                            | Effect::DiscardCards { .. }
                            | Effect::Loot { .. }
                            | Effect::GainLife { .. }
                            | Effect::Mill { .. }
                            | Effect::Scry { .. }
                            | Effect::AddMana { .. }
                            | Effect::Balance { .. }
                            | Effect::CreateToken { .. }
                            | Effect::Dig { .. }
                            | Effect::SearchLibrary { .. }
                            | Effect::Firebend { .. }
                            | Effect::SetBasePowerToughness { .. }
                            | Effect::CounterSpell { .. }
                            | Effect::PumpCreature { .. }
                            | Effect::TapPermanent { .. }
                            | Effect::UntapPermanent { .. }
                            | Effect::ExilePermanent { .. }
                            | Effect::Airbend { .. }
                            | Effect::Earthbend { .. }
                            | Effect::GrantCantBeBlocked { .. }
                            | Effect::RemoveCounter { .. }
                            | Effect::PutCounter { .. }
                            | Effect::AttachEquipment { .. }
                            | Effect::ModalChoice { .. }
                            | Effect::PumpAllCreatures { .. }
                            | Effect::CreateDelayedTrigger { .. }
                            | Effect::CopySpellAbility { .. } => {
                                // Non-Destroy/Copy modes in modal spells
                                // TODO(mtg-30): Add handlers for targeting modes that need them
                            }
                            // Guards failed for Destroy/Copy - target already specified
                            Effect::DestroyPermanent { .. } | Effect::CopyPermanent { .. } => {
                                // Target already specified (guard failed: target.as_u32() != 0)
                            }
                        }
                    }
                }
                // ===== EXHAUSTIVE EFFECT HANDLING FOR TARGET COLLECTION =====
                // These effects either:
                // 1. Don't need creature/permanent targets (target players, self, or no target)
                // 2. Already have targets specified (non-zero target field)
                //
                // IMPORTANT: When adding new Effect variants, the compiler will force you to
                // handle them here. If the new effect needs targeting, add a handler above.
                // If it doesn't need targeting, add it to this exhaustive list.
                //
                // Effects targeting players or with no target
                Effect::DrawCards { .. }
                | Effect::DiscardCards { .. }
                | Effect::Loot { .. }
                | Effect::GainLife { .. }
                | Effect::Mill { .. }
                | Effect::Scry { .. }
                | Effect::AddMana { .. }
                | Effect::Balance { .. }
                | Effect::CreateToken { .. }
                | Effect::Dig { .. }
                | Effect::SearchLibrary { .. }
                | Effect::Firebend { .. }
                | Effect::SetBasePowerToughness { .. }
                | Effect::Earthbend { .. }
                | Effect::AttachEquipment { .. } => {
                    // These effects target players or have no targeting requirements
                    // AttachEquipment targeting is handled via Equip keyword abilities
                }
                // Effects with already-specified targets (non-zero target field)
                // The handlers above only match when target.as_u32() == 0
                Effect::DealDamage { .. } => {
                    // Either TargetRef::Player (already specified) or TargetRef::Permanent (already specified)
                    // TargetRef::None case handled above
                }
                Effect::DestroyPermanent { .. }
                | Effect::PumpCreature { .. }
                | Effect::TapPermanent { .. }
                | Effect::UntapPermanent { .. }
                | Effect::CounterSpell { .. }
                | Effect::ExilePermanent { .. }
                | Effect::Airbend { .. }
                | Effect::GrantCantBeBlocked { .. }
                | Effect::RemoveCounter { .. }
                | Effect::PutCounter { .. }
                | Effect::CopyPermanent { .. }
                | Effect::PumpAllCreatures { .. }
                | Effect::CreateDelayedTrigger { .. }
                | Effect::CopySpellAbility { .. } => {
                    // Target already specified (guard failed: target.as_u32() != 0)
                    // This means the effect has a concrete target already assigned
                    // PumpAllCreatures doesn't use explicit targets - it affects all matching creatures
                    // CreateDelayedTrigger with non-zero tracked_card already has target
                    // CopySpellAbility doesn't use explicit targets - copies triggering spell
                }
            }
        }

        // MTG Rule 303.4a: Auras target objects or players as they're being cast
        // Check if spell is an Aura and add valid targets based on "Enchant X" keyword
        let is_aura = self.cards.get(spell_card_id).map(|c| c.is_aura()).unwrap_or(false);
        if is_aura {
            // Get the enchant restriction (e.g., "creature", "land", "permanent")
            // May include zone qualifiers like "creature.inzonegraveyard" (Animate Dead)
            let enchant_type = self
                .cards
                .get(spell_card_id)
                .ok()
                .and_then(|c| c.keywords.get_args(crate::core::Keyword::Enchant))
                .and_then(|args| {
                    if let crate::core::KeywordArgs::Enchant { card_type } = args {
                        Some(card_type.as_str().to_lowercase())
                    } else {
                        None
                    }
                })
                .unwrap_or_else(|| "creature".to_string()); // Default to creature

            // Parse zone qualifier from enchant type
            // Format: "creature.inzonegraveyard" -> base_type="creature", target_zone=Some("graveyard")
            // CR 303.4f: Animate Dead can only target creature cards in graveyards when cast
            let (base_type, target_zone) = if let Some((type_part, zone_part)) = enchant_type.split_once(".inzone") {
                (type_part, Some(zone_part))
            } else {
                (enchant_type.as_str(), None)
            };

            // Helper closure to check if a card matches the enchant type restriction
            let matches_type = |target_card: &crate::core::card::Card| -> bool {
                match base_type {
                    "creature" => target_card.is_creature(),
                    "land" => target_card.is_land(),
                    "artifact" => target_card.is_artifact(),
                    "enchantment" => target_card.is_type(&crate::core::CardType::Enchantment),
                    "instant" => target_card.is_type(&crate::core::CardType::Instant),
                    "sorcery" => target_card.is_type(&crate::core::CardType::Sorcery),
                    "permanent" => true,            // Any permanent
                    _ => target_card.is_creature(), // Default fallback
                }
            };

            // Choose which zone to search based on the zone qualifier
            match target_zone {
                Some("graveyard") => {
                    // Search ALL graveyards for matching cards (CR 303.4f)
                    // Auras like Animate Dead, Dance of the Dead, Spellweaver Volute
                    for (_, zones) in &self.player_zones {
                        for &card_id in &zones.graveyard.cards {
                            if let Ok(target_card) = self.cards.get(card_id) {
                                if matches_type(target_card) {
                                    // Cards in graveyard can't have hexproof/shroud protection
                                    // (those only apply to permanents on the battlefield)
                                    valid_targets.push(card_id);
                                }
                            }
                        }
                    }
                }
                _ => {
                    // Default: search battlefield (standard Auras)
                    for &card_id in &self.battlefield.cards {
                        if let Ok(target_card) = self.cards.get(card_id) {
                            if matches_type(target_card) && is_legal_target(target_card, spell_owner) {
                                valid_targets.push(card_id);
                            }
                        }
                    }
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
            Cost::Tap
            | Cost::Untap
            | Cost::Mana(_)
            | Cost::TapAndMana(_)
            | Cost::Sacrifice { .. }
            | Cost::PayLife { .. }
            | Cost::Discard { .. }
            | Cost::Waterbend { .. } => false,
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
    ///
    /// # Errors
    ///
    /// Returns an error if the card or ability cannot be found.
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
                            if !is_legal_target(card, ability_controller) {
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
                            let is_valid = !card.tapped && is_legal_target(card, ability_controller);

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
                            let is_valid = card.tapped && is_legal_target(card, ability_controller);

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
                            let is_valid = card.is_creature() && is_legal_target(card, ability_controller);

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
                            if !is_legal_target(card, ability_controller) {
                                is_valid = false;
                            }

                            if is_valid {
                                valid_targets.push(card_id);
                            }
                        }
                    }
                }
                Effect::Airbend { target } if target.as_u32() == 0 => {
                    // Airbend targets creatures (or other permanents based on ValidTgts)
                    // CR 701.65b: Airbend exiles a target permanent
                    for &card_id in &self.battlefield.cards {
                        if let Ok(card) = self.cards.get(card_id) {
                            // By default, Airbend targets creatures
                            // TODO: Could be extended with ValidTgts parsing for nonland permanents
                            let mut is_valid = card.is_creature();

                            // Check shroud/hexproof
                            if !is_legal_target(card, ability_controller) {
                                is_valid = false;
                            }

                            if is_valid {
                                valid_targets.push(card_id);
                            }
                        }
                    }
                }
                Effect::GrantCantBeBlocked { target } if target.as_u32() == 0 => {
                    // GrantCantBeBlocked targets creatures you control
                    // Deserter's Disciple: "Another target creature you control with power 2 or less"
                    // The "Other" and "powerLE2" restrictions should ideally be parsed from ValidTgts
                    for &card_id in &self.battlefield.cards {
                        if let Ok(card) = self.cards.get(card_id) {
                            // Must be a creature we control
                            let mut is_valid = card.is_creature() && card.controller == ability_controller;

                            // "Other" - can't target the source card (Deserter's Disciple itself)
                            if card_id == source_card.id {
                                is_valid = false;
                            }

                            // Power restriction: "powerLE2" means power <= 2
                            // Check current power including counters and bonuses
                            if is_valid && card.current_power() > 2 {
                                is_valid = false;
                            }

                            // Check shroud/hexproof (though typically not relevant for own creatures)
                            if !is_legal_target(card, ability_controller) {
                                is_valid = false;
                            }

                            if is_valid {
                                valid_targets.push(card_id);
                            }
                        }
                    }
                }
                Effect::Earthbend { target, .. } if target.as_u32() == 0 => {
                    // Earthbend targets lands you control
                    // CR 701.65: Earthbend makes a land into a creature with counters
                    for &card_id in &self.battlefield.cards {
                        if let Ok(card) = self.cards.get(card_id) {
                            // Must be a land we control
                            let is_valid = card.is_land() && card.controller == ability_controller;

                            // Note: Lands typically can't have shroud/hexproof,
                            // but check anyway for completeness
                            if is_valid && is_legal_target(card, ability_controller) {
                                valid_targets.push(card_id);
                            }
                        }
                    }
                }
                // ===== EXHAUSTIVE EFFECT HANDLING FOR ABILITY TARGETING =====
                // Effects that don't need targets or have targets pre-specified
                Effect::DrawCards { .. }
                | Effect::DiscardCards { .. }
                | Effect::Loot { .. }
                | Effect::GainLife { .. }
                | Effect::Mill { .. }
                | Effect::Scry { .. }
                | Effect::AddMana { .. }
                | Effect::Balance { .. }
                | Effect::CreateToken { .. }
                | Effect::Dig { .. }
                | Effect::SearchLibrary { .. }
                | Effect::Firebend { .. }
                | Effect::SetBasePowerToughness { .. }
                | Effect::ModalChoice { .. }
                | Effect::CreateDelayedTrigger { .. }
                | Effect::CopySpellAbility { .. } => {
                    // These effects target players or have no targeting requirements
                    // CreateDelayedTrigger targets creatures - handled via ValidTgts$ Creature
                    // CopySpellAbility doesn't need explicit targets - copies triggering spell
                }
                // Effects with pre-specified targets (guard failed: target.as_u32() != 0)
                Effect::DealDamage { .. } => {
                    // TargetRef::Player/Permanent - target already specified
                }
                Effect::DestroyPermanent { .. }
                | Effect::PumpCreature { .. }
                | Effect::TapPermanent { .. }
                | Effect::UntapPermanent { .. }
                | Effect::CounterSpell { .. }
                | Effect::ExilePermanent { .. }
                | Effect::Airbend { .. }
                | Effect::GrantCantBeBlocked { .. }
                | Effect::RemoveCounter { .. }
                | Effect::PutCounter { .. }
                | Effect::CopyPermanent { .. }
                | Effect::AttachEquipment { .. }
                | Effect::PumpAllCreatures { .. }
                | Effect::Earthbend { .. } => {
                    // Target already specified (guard failed: target.as_u32() != 0)
                    // PumpAllCreatures doesn't use explicit targets - it affects all matching creatures
                    // Earthbend target was handled above when target.as_u32() == 0
                }
            }
        }

        // Sort for deterministic ordering (critical for snapshot/resume)
        valid_targets.sort();
        Ok(valid_targets)
    }

    /// Check if a spell has a modal choice effect.
    ///
    /// Returns Some with the ModalChoice parameters if the spell is modal,
    /// or None if it's a regular spell.
    ///
    /// Modal spells have "Choose one —" or similar text and require mode
    /// selection before targeting (MTG Rule 601.2b).
    ///
    /// # Errors
    ///
    /// Returns an error if the spell card cannot be found.
    pub fn get_modal_choice_info(&self, spell_card_id: CardId) -> Result<Option<crate::core::Effect>> {
        let spell_card = self.cards.get(spell_card_id)?;

        for effect in &spell_card.effects {
            if matches!(effect, Effect::ModalChoice { .. }) {
                return Ok(Some(effect.clone()));
            }
        }

        Ok(None)
    }

    /// Get the mode descriptions for a modal spell.
    ///
    /// Returns a vector of mode descriptions for display to the player.
    /// Used by controllers when prompting for mode selection.
    ///
    /// # Errors
    ///
    /// Returns an error if the spell card cannot be found.
    pub fn get_modal_mode_descriptions(&self, spell_card_id: CardId) -> Result<Vec<String>> {
        let spell_card = self.cards.get(spell_card_id)?;

        for effect in &spell_card.effects {
            if let Effect::ModalChoice { modes, .. } = effect {
                return Ok(modes.iter().map(|m| m.description.clone()).collect());
            }
        }

        Ok(Vec::new())
    }

    /// Get which mode indices have valid targets for a modal spell.
    ///
    /// This filters modes based on whether their effects have legal targets.
    /// For example, Heartless Act mode 1 ("Destroy target creature with no counters")
    /// requires a creature WITHOUT counters, so it's only valid if such a creature exists.
    ///
    /// # Arguments
    /// * `spell_card_id` - The modal spell card
    /// * `spell_owner` - The player casting the spell (for hexproof checks)
    ///
    /// # Returns
    /// A vector of (mode_index, has_valid_targets) for each mode
    ///
    /// # Errors
    ///
    /// Returns an error if the spell card cannot be found.
    pub fn get_valid_modes_for_spell(
        &self,
        spell_card_id: CardId,
        spell_owner: PlayerId,
    ) -> Result<Vec<(usize, bool)>> {
        let spell_card = self.cards.get(spell_card_id)?;

        for effect in &spell_card.effects {
            if let Effect::ModalChoice { modes, .. } = effect {
                let mut result = Vec::with_capacity(modes.len());

                for (idx, mode) in modes.iter().enumerate() {
                    let has_targets = self.effect_has_valid_targets(&mode.effect, spell_owner);
                    result.push((idx, has_targets));
                }

                return Ok(result);
            }
        }

        // Not a modal spell
        Ok(Vec::new())
    }

    /// Check if a specific effect has any valid targets on the battlefield.
    ///
    /// Returns true if:
    /// - The effect doesn't require targeting (e.g., DrawCards), or
    /// - There exists at least one legal target for the effect
    fn effect_has_valid_targets(&self, effect: &Effect, spell_owner: PlayerId) -> bool {
        match effect {
            Effect::DestroyPermanent { target, restriction } if target.as_u32() == 0 => {
                // Check if any permanent matches the restriction
                self.battlefield.cards.iter().any(|&card_id| {
                    if let Ok(card) = self.cards.get(card_id) {
                        restriction.matches(card) && is_legal_target(card, spell_owner)
                    } else {
                        false
                    }
                })
            }
            Effect::RemoveCounter { target, .. } if target.as_u32() == 0 => {
                // RemoveCounter can target any creature (mode 2 of Heartless Act)
                self.battlefield.cards.iter().any(|&card_id| {
                    if let Ok(card) = self.cards.get(card_id) {
                        card.is_creature() && is_legal_target(card, spell_owner)
                    } else {
                        false
                    }
                })
            }
            Effect::PumpCreature { target, .. } if target.as_u32() == 0 => {
                // Pump requires a creature target
                self.battlefield.cards.iter().any(|&card_id| {
                    if let Ok(card) = self.cards.get(card_id) {
                        card.is_creature() && is_legal_target(card, spell_owner)
                    } else {
                        false
                    }
                })
            }
            Effect::TapPermanent { target } if target.as_u32() == 0 => {
                // Tap requires an untapped permanent
                self.battlefield.cards.iter().any(|&card_id| {
                    if let Ok(card) = self.cards.get(card_id) {
                        !card.tapped && is_legal_target(card, spell_owner)
                    } else {
                        false
                    }
                })
            }
            Effect::UntapPermanent { target } if target.as_u32() == 0 => {
                // Untap requires a tapped permanent
                self.battlefield.cards.iter().any(|&card_id| {
                    if let Ok(card) = self.cards.get(card_id) {
                        card.tapped && is_legal_target(card, spell_owner)
                    } else {
                        false
                    }
                })
            }
            Effect::DealDamage {
                target: TargetRef::None,
                ..
            } => {
                // Damage can target creatures or players - always has targets if there's a creature
                // (players are always valid targets)
                true
            }
            Effect::ExilePermanent { target } if target.as_u32() == 0 => {
                // Exile requires a permanent target
                self.battlefield.cards.iter().any(|&card_id| {
                    if let Ok(card) = self.cards.get(card_id) {
                        is_legal_target(card, spell_owner)
                    } else {
                        false
                    }
                })
            }
            Effect::CounterSpell { target } if target.as_u32() == 0 => {
                // Counter requires a spell on the stack
                !self.stack.is_empty()
            }
            // Effects that don't require targeting always "have targets"
            Effect::DrawCards { .. }
            | Effect::DiscardCards { .. }
            | Effect::Loot { .. }
            | Effect::GainLife { .. }
            | Effect::Mill { .. }
            | Effect::Scry { .. }
            | Effect::AddMana { .. }
            | Effect::Balance { .. }
            | Effect::CreateToken { .. }
            | Effect::Dig { .. }
            | Effect::SearchLibrary { .. }
            | Effect::Firebend { .. }
            | Effect::SetBasePowerToughness { .. }
            | Effect::Earthbend { .. }
            | Effect::AttachEquipment { .. }
            | Effect::ModalChoice { .. }
            | Effect::PumpAllCreatures { .. }
            | Effect::CreateDelayedTrigger { .. }
            | Effect::CopySpellAbility { .. } => true, // PumpAllCreatures uses filter, not explicit targets

            // ===== EXHAUSTIVE EFFECT HANDLING =====
            // Effects with pre-specified targets (guard failed: target.as_u32() != 0)
            // These already have targets, so they "have valid targets"
            Effect::DealDamage { .. } => true, // TargetRef::Player/Permanent already specified
            Effect::DestroyPermanent { .. }
            | Effect::PumpCreature { .. }
            | Effect::TapPermanent { .. }
            | Effect::UntapPermanent { .. }
            | Effect::CounterSpell { .. }
            | Effect::ExilePermanent { .. }
            | Effect::Airbend { .. }
            | Effect::GrantCantBeBlocked { .. }
            | Effect::RemoveCounter { .. }
            | Effect::PutCounter { .. }
            | Effect::CopyPermanent { .. } => {
                // Target already specified (guard failed: target.as_u32() != 0)
                // If a target was pre-assigned, we assume it's valid
                true
            }
        }
    }

    /// Apply selected modes to a modal spell, replacing the ModalChoice effect
    /// with the effects from the selected modes.
    ///
    /// This is called after the player selects modes but before targeting.
    /// The selected mode effects are inserted in place of the ModalChoice.
    ///
    /// # Arguments
    /// * `spell_card_id` - The modal spell card
    /// * `selected_mode_indices` - The indices of selected modes (0-based)
    ///
    /// # Returns
    /// Ok(true) if modes were applied, Ok(false) if spell wasn't modal
    ///
    /// # Errors
    ///
    /// Returns an error if the spell card cannot be found.
    pub fn apply_selected_modes(&mut self, spell_card_id: CardId, selected_mode_indices: &[usize]) -> Result<bool> {
        let spell_card = self.cards.get_mut(spell_card_id)?;

        // Find the ModalChoice effect and get selected mode effects
        let mut new_effects = Vec::new();
        let mut found_modal = false;

        for effect in spell_card.effects.drain(..) {
            if let Effect::ModalChoice { modes, .. } = effect {
                found_modal = true;
                // Add effects from selected modes in order
                for &mode_idx in selected_mode_indices {
                    if let Some(mode) = modes.get(mode_idx) {
                        new_effects.push((*mode.effect).clone());
                    }
                }
            } else {
                // Keep non-modal effects as-is
                new_effects.push(effect);
            }
        }

        spell_card.effects = new_effects;
        Ok(found_modal)
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
        creature_card.add_type(CardType::Creature); // Use add_type() to update cache
        game.cards.insert(creature_id, creature_card);
        game.battlefield.cards.push(creature_id);

        // Add a land to the battlefield
        let land_id = game.cards.next_id();
        let mut land_card = Card::new(land_id, "Swamp", player2);
        land_card.add_type(CardType::Land); // Use add_type() to update cache
        game.cards.insert(land_id, land_card);
        game.battlefield.cards.push(land_id);

        // Create Sinkhole spell card with oracle text
        let sinkhole_id = game.cards.next_id();
        let mut sinkhole_card = Card::new(sinkhole_id, "Sinkhole", player1);
        sinkhole_card.add_type(CardType::Sorcery);
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
        creature_card.add_type(CardType::Creature); // Use add_type() to update cache
        game.cards.insert(creature_id, creature_card);
        game.battlefield.cards.push(creature_id);

        // Add a land to the battlefield
        let land_id = game.cards.next_id();
        let mut land_card = Card::new(land_id, "Swamp", player2);
        land_card.add_type(CardType::Land); // Use add_type() to update cache
        game.cards.insert(land_id, land_card);
        game.battlefield.cards.push(land_id);

        // Create Terror spell card with oracle text
        let terror_id = game.cards.next_id();
        let mut terror_card = Card::new(terror_id, "Terror", player1);
        terror_card.add_type(CardType::Instant);
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

    /// Test that Animate Dead targets creatures in graveyards, NOT on battlefield
    /// This verifies the fix for mtg-s2atg
    #[test]
    fn test_animate_dead_targets_graveyard_not_battlefield() {
        use crate::core::{KeywordArgs, Subtype};
        use crate::game::state::GameState;

        // Create a minimal game state to test targeting
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let player1 = PlayerId::new(0);
        let player2 = PlayerId::new(1);

        // Add a creature to the battlefield (should NOT be targetable by Animate Dead)
        let battlefield_creature_id = game.cards.next_id();
        let mut battlefield_creature = Card::new(battlefield_creature_id, "Sengir Vampire", player2);
        battlefield_creature.add_type(CardType::Creature);
        game.cards.insert(battlefield_creature_id, battlefield_creature);
        game.battlefield.cards.push(battlefield_creature_id);

        // Add a creature to player2's graveyard (SHOULD be targetable by Animate Dead)
        let graveyard_creature_id = game.cards.next_id();
        let mut graveyard_creature = Card::new(graveyard_creature_id, "Serra Angel", player2);
        graveyard_creature.add_type(CardType::Creature);
        game.cards.insert(graveyard_creature_id, graveyard_creature);
        // Add to player2's graveyard
        if let Some(zones) = game.get_player_zones_mut(player2) {
            zones.graveyard.cards.push(graveyard_creature_id);
        }

        // Create Animate Dead - an Aura with "Enchant creature card in a graveyard"
        let animate_dead_id = game.cards.next_id();
        let mut animate_dead = Card::new(animate_dead_id, "Animate Dead", player1);
        animate_dead.add_type(CardType::Enchantment);
        // Add "Aura" subtype - must be after add_type() since is_aura requires is_enchantment
        animate_dead.set_subtypes(smallvec::smallvec![Subtype::new("Aura")]);
        // Set the Enchant keyword with the inZoneGraveyard qualifier
        // insert_complex also inserts the keyword into the set
        animate_dead.keywords.insert_complex(KeywordArgs::Enchant {
            card_type: Subtype::new("Creature.inZoneGraveyard"),
        });
        game.cards.insert(animate_dead_id, animate_dead);

        // Get valid targets for Animate Dead
        let targets = game.get_valid_targets_for_spell(animate_dead_id).unwrap();

        // Should target the creature in graveyard
        assert!(
            targets.contains(&graveyard_creature_id),
            "Animate Dead should target creatures in graveyards"
        );

        // Should NOT target the creature on battlefield
        assert!(
            !targets.contains(&battlefield_creature_id),
            "Animate Dead should NOT target creatures on battlefield (this was the bug in mtg-s2atg!)"
        );
    }
}
