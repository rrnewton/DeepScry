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
/// .filter(|card| is_legal_target(card, controller, &source_colors))
/// ```
#[inline]
pub fn is_legal_target(
    card: &crate::core::card::Card,
    source_controller: PlayerId,
    source_colors: &[crate::core::Color],
) -> bool {
    // Shroud prevents targeting by anyone (CR 702.18a)
    if card.has_shroud() {
        return false;
    }

    // Hexproof only protects from opponent's spells/abilities (CR 702.19a)
    // Note: Uses controller, not owner - hexproof protects from opponent CONTROLLERS
    if card.has_hexproof() && card.controller != source_controller {
        return false;
    }

    // Protection prevents targeting by sources of the protected color (CR 702.16b)
    for &color in source_colors {
        if card.has_protection_from(color) {
            return false;
        }
    }

    true
}

impl GameState {
    /// Determine the `(min, max)` number of targets a spell requires (CR
    /// 601.2c). `num_valid` is the count returned by
    /// `get_valid_targets_for_spell`.
    ///
    /// Most spells take exactly one target, so the default is `(1, 1)`. A
    /// `DivideEvenly$` X-damage spell (Fireball: `TargetMin$ 0 | TargetMax$
    /// MaxTargets`) takes a variable number — the player may hit anywhere from
    /// 0 up to every legal target. `MaxTargets` in the script is a public,
    /// state-derived count (players + permanents); since the engine only offers
    /// the actually-legal targets, the effective upper bound is `num_valid`.
    /// All inputs are public, so the bounds are network-deterministic.
    pub fn target_count_bounds_for_spell(&self, spell_card_id: CardId, num_valid: usize) -> (usize, usize) {
        let Ok(card) = self.cards.get(spell_card_id) else {
            return (1, 1);
        };
        // DivideEvenly X-damage (Fireball) is the only variable-target shape in
        // scope. min comes from TargetMin$ 0; max is bounded by the legal targets.
        for effect in &card.effects {
            if let Effect::DealDamageXPaid {
                divide: crate::core::DamageDivision::EvenlyRoundedDown,
                ..
            } = effect
            {
                return (0, num_valid);
            }
        }
        (1, 1)
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
    ///
    /// # Errors
    ///
    /// Returns an error if the spell card cannot be found.
    pub fn get_valid_targets_for_spell(&self, spell_card_id: CardId) -> Result<SmallVec<[CardId; 8]>> {
        let mut valid_targets = SmallVec::new();

        // Get the spell's owner, effects count, and targeting restrictions from cache
        // Extract primitives first to avoid holding a borrow while iterating
        let (
            spell_owner,
            num_effects,
            targets_land,
            targets_creature,
            targets_planeswalker,
            targets_any,
            targets_player,
            spell_colors,
        ) = {
            let spell_card = self.cards.get(spell_card_id)?;
            (
                spell_card.owner,
                spell_card.effects.len(),
                spell_card.definition.cache.spell_targets_land,
                spell_card.definition.cache.spell_targets_creature,
                spell_card.definition.cache.spell_targets_planeswalker,
                spell_card.definition.cache.spell_targets_any,
                spell_card.definition.cache.spell_targets_player,
                spell_card.colors.clone(),
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
                }
                | Effect::DealDamageXPaid {
                    target: TargetRef::None,
                    ..
                }
                | Effect::DealDamageDynamic {
                    target: TargetRef::None,
                    ..
                } => {
                    // Damage targets per the spell's ValidTgts (CR 115.4):
                    //   "any target"   (Lightning Bolt) -> creatures + players + planeswalkers
                    //   "target player"                  -> players only
                    //   "target creature" (Magma Rift)   -> creatures only
                    // Add every legally-targetable creature on the battlefield
                    // when creatures are allowed.
                    let allow_creatures = targets_any || targets_creature || !targets_player;
                    if allow_creatures {
                        for &card_id in &self.battlefield.cards {
                            if let Ok(target_card) = self.cards.get(card_id) {
                                if target_card.is_creature() && is_legal_target(target_card, spell_owner, &spell_colors)
                                {
                                    valid_targets.push(card_id);
                                }
                            }
                        }
                    }
                    // Add every legally-targetable planeswalker on the battlefield when allowed.
                    let allow_planeswalkers = targets_any || targets_planeswalker;
                    if allow_planeswalkers {
                        for &card_id in &self.battlefield.cards {
                            if let Ok(target_card) = self.cards.get(card_id) {
                                if target_card.is_planeswalker() && is_legal_target(target_card, spell_owner, &spell_colors)
                                {
                                    valid_targets.push(card_id);
                                }
                            }
                        }
                    }
                    // Add every Player when players are allowed targets, encoded
                    // as a sentinel CardId so the existing
                    // `Controller::choose_targets(&[CardId])` trait can offer
                    // them as picks. The sentinel is decoded back into a
                    // `TargetRef::Player` at effect-resolution time in
                    // `resolve_target_for_effect` (mtg-bolt-player-tgt).
                    // This fixes the user-reported bug where Lightning Bolt
                    // refused to list the opponent as a legal target.
                    if targets_any || targets_player {
                        for player in &self.players {
                            valid_targets.push(crate::core::player_as_target_sentinel(player.id));
                        }
                    }
                }
                Effect::DestroyPermanent {
                    target, restriction, ..
                } if target.is_placeholder() => {
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
                            if restriction_matches
                                && cache_matches
                                && is_legal_target(target_card, spell_owner, &spell_colors)
                            {
                                valid_targets.push(card_id);
                            }
                        }
                    }
                }
                Effect::PumpCreature { target, .. } if target.is_placeholder() => {
                    // Pump can target any creature
                    for &card_id in &self.battlefield.cards {
                        if let Ok(target_card) = self.cards.get(card_id) {
                            if target_card.is_creature() && is_legal_target(target_card, spell_owner, &spell_colors) {
                                valid_targets.push(card_id);
                            }
                        }
                    }
                }
                Effect::DebuffCreature { target, .. } if target.is_placeholder() => {
                    // Debuff can target any creature
                    for &card_id in &self.battlefield.cards {
                        if let Ok(target_card) = self.cards.get(card_id) {
                            if target_card.is_creature() && is_legal_target(target_card, spell_owner, &spell_colors) {
                                valid_targets.push(card_id);
                            }
                        }
                    }
                }
                Effect::PumpCreatureVariable { target, .. } if target.is_placeholder() => {
                    // Variable pump can target any creature
                    for &card_id in &self.battlefield.cards {
                        if let Ok(target_card) = self.cards.get(card_id) {
                            if target_card.is_creature() && is_legal_target(target_card, spell_owner, &spell_colors) {
                                valid_targets.push(card_id);
                            }
                        }
                    }
                }
                Effect::GainControl { target, .. } if target.is_placeholder() => {
                    // GainControl targets opponent's permanents (creatures by default)
                    for &card_id in &self.battlefield.cards {
                        if let Ok(target_card) = self.cards.get(card_id) {
                            // Must be opponent's permanent
                            if target_card.controller == spell_owner {
                                continue;
                            }
                            // Check type from cached flags (default: creatures)
                            let type_matches = if targets_any {
                                true
                            } else if targets_creature {
                                target_card.is_creature()
                            } else {
                                // Default to creatures for GainControl
                                target_card.is_creature()
                            };
                            if type_matches && is_legal_target(target_card, spell_owner, &spell_colors) {
                                valid_targets.push(card_id);
                            }
                        }
                    }
                }
                Effect::Fight { target, .. } if target.is_placeholder() => {
                    // Fight targets an opponent's creature by default
                    for &card_id in &self.battlefield.cards {
                        if let Ok(target_card) = self.cards.get(card_id) {
                            if target_card.is_creature()
                                && target_card.controller != spell_owner
                                && is_legal_target(target_card, spell_owner, &spell_colors)
                            {
                                valid_targets.push(card_id);
                            }
                        }
                    }
                }
                Effect::TapPermanent { target } if target.is_placeholder() => {
                    // Tap can target untapped permanents, honoring the spell's
                    // ValidTgts$ type restriction. Winter Blast ("Tap X target
                    // creatures", ValidTgts$ Creature) must only be able to tap
                    // creatures — before this the branch checked only !tapped +
                    // is_legal_target, so it could tap a land. A spell that taps
                    // "any permanent" sets neither flag and stays permissive.
                    for &card_id in &self.battlefield.cards {
                        if let Ok(target_card) = self.cards.get(card_id) {
                            if target_card.tapped {
                                continue;
                            }
                            if targets_creature && !target_card.is_creature() {
                                continue;
                            }
                            if targets_land && !target_card.is_land() {
                                continue;
                            }
                            if is_legal_target(target_card, spell_owner, &spell_colors) {
                                valid_targets.push(card_id);
                            }
                        }
                    }
                }
                Effect::UntapPermanent { target } if target.is_placeholder() => {
                    // Untap can target tapped permanents
                    for &card_id in &self.battlefield.cards {
                        if let Ok(target_card) = self.cards.get(card_id) {
                            if target_card.tapped && is_legal_target(target_card, spell_owner, &spell_colors) {
                                valid_targets.push(card_id);
                            }
                        }
                    }
                }
                Effect::TapOrUntapPermanent { target } if target.is_placeholder() => {
                    // Tap or untap can target any permanent (creature, land, etc.)
                    for &card_id in &self.battlefield.cards {
                        if let Ok(target_card) = self.cards.get(card_id) {
                            if is_legal_target(target_card, spell_owner, &spell_colors) {
                                valid_targets.push(card_id);
                            }
                        }
                    }
                }
                Effect::CounterSpell {
                    target,
                    spell_restriction,
                    ..
                } if target.is_placeholder() => {
                    // Counter can target spells on the stack (except self).
                    // The spell_restriction encodes all ValidTgts$ constraints:
                    // color (Red Elemental Blast), type (Essence Scatter, Annul),
                    // nonCreature (Negate), and min_cmc (Disdainful Stroke).
                    for &card_id in &self.stack.cards {
                        if card_id == spell_card_id {
                            continue;
                        }
                        let Ok(target_card) = self.cards.get(card_id) else {
                            continue;
                        };
                        // Color restriction (e.g. Red Elemental Blast)
                        if let Some(color) = spell_restriction.required_color {
                            if !target_card.is_color(color) {
                                continue;
                            }
                        }
                        // Type restriction: must match one of the listed types
                        // (e.g. Creature for Essence Scatter, Artifact or Enchantment for Annul)
                        if !spell_restriction.types.is_empty() {
                            let type_ok = spell_restriction.types.iter().any(|t| match t {
                                crate::core::TargetType::Creature => target_card.is_creature(),
                                crate::core::TargetType::Artifact => target_card.is_artifact(),
                                crate::core::TargetType::Enchantment => target_card.is_enchantment(),
                                crate::core::TargetType::Land => target_card.is_land(),
                                crate::core::TargetType::Planeswalker => {
                                    target_card.types.contains(&crate::core::CardType::Planeswalker)
                                }
                                crate::core::TargetType::Any => true,
                            });
                            if !type_ok {
                                continue;
                            }
                        }
                        // nonCreature restriction (Negate)
                        if spell_restriction.requires_noncreature && target_card.is_creature() {
                            continue;
                        }
                        // Minimum CMC restriction (Disdainful Stroke: cmcGE4)
                        if let Some(min) = spell_restriction.min_cmc {
                            if target_card.mana_cost.cmc() < min {
                                continue;
                            }
                        }
                        valid_targets.push(card_id);
                    }
                }
                Effect::ReturnPermanentToHand { target, restriction } if target.is_placeholder() => {
                    // Bounce effect: return target permanent to its owner's hand.
                    // Use the parsed TargetRestriction from ValidTgts$ to filter valid targets.
                    // Examples: Teferi -3 (Artifact,Creature,Enchantment), Petty Theft (nonland OppCtrl).
                    for &card_id in &self.battlefield.cards {
                        if let Ok(target_card) = self.cards.get(card_id) {
                            if restriction.matches(target_card)
                                && is_legal_target(target_card, spell_owner, &spell_colors)
                            {
                                valid_targets.push(card_id);
                            }
                        }
                    }
                }
                Effect::ExilePermanent { target } if target.is_placeholder() => {
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

                            if type_matches && is_legal_target(target_card, spell_owner, &spell_colors) {
                                valid_targets.push(card_id);
                            }
                        }
                    }
                }
                Effect::Airbend { target } if target.is_placeholder() => {
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

                            if type_matches && is_legal_target(target_card, spell_owner, &spell_colors) {
                                valid_targets.push(card_id);
                            }
                        }
                    }
                }
                Effect::GrantCantBeBlocked { target } if target.is_placeholder() => {
                    // GrantCantBeBlocked targets creatures you control
                    // Deserter's Disciple: "Another target creature you control with power 2 or less"
                    // For now, we support basic creature targeting - power restriction checked at ability parse time
                    for &card_id in &self.battlefield.cards {
                        if let Ok(target_card) = self.cards.get(card_id) {
                            // Must be a creature we control
                            if target_card.is_creature()
                                && target_card.controller == spell_owner
                                && is_legal_target(target_card, spell_owner, &spell_colors)
                            {
                                // Note: "Other" and power restrictions are validated at ability level
                                // via ValidTgts$ parsing (e.g., Creature.Other+YouCtrl+powerLE2)
                                valid_targets.push(card_id);
                            }
                        }
                    }
                }
                Effect::Regenerate { target } if target.is_placeholder() => {
                    // Regenerate targets creatures you control (most common: self)
                    // Cards like Yavimaya Hollow use ValidTgts$ Creature for any creature
                    for &card_id in &self.battlefield.cards {
                        if let Ok(target_card) = self.cards.get(card_id) {
                            if target_card.is_creature()
                                && target_card.controller == spell_owner
                                && is_legal_target(target_card, spell_owner, &spell_colors)
                            {
                                valid_targets.push(card_id);
                            }
                        }
                    }
                }
                Effect::PreventDamage {
                    target: TargetRef::None,
                    ..
                } => {
                    // PreventDamage can target any creature (or player, handled separately)
                    for &card_id in &self.battlefield.cards {
                        if let Ok(target_card) = self.cards.get(card_id) {
                            if target_card.is_creature() && is_legal_target(target_card, spell_owner, &spell_colors) {
                                valid_targets.push(card_id);
                            }
                        }
                    }
                }
                Effect::RemoveCounter { target, .. } if target.is_placeholder() => {
                    // RemoveCounter targets creatures (e.g., Heartless Act mode 2)
                    // TODO(mtg-n36vb): Some RemoveCounter effects can target any permanent
                    for &card_id in &self.battlefield.cards {
                        if let Ok(target_card) = self.cards.get(card_id) {
                            if target_card.is_creature() && is_legal_target(target_card, spell_owner, &spell_colors) {
                                valid_targets.push(card_id);
                            }
                        }
                    }
                }
                Effect::CreateDelayedTrigger { tracked_card, .. } if tracked_card.is_placeholder() => {
                    // CreateDelayedTrigger targets creatures (e.g., Fatal Fissure: "Choose target creature")
                    // The tracked_card field holds the target that will be watched for death
                    for &card_id in &self.battlefield.cards {
                        if let Ok(target_card) = self.cards.get(card_id) {
                            // Default to targeting any creature (Fatal Fissure targets any creature)
                            // Controller restriction from ValidTgts$ is checked at ability level
                            if target_card.is_creature() && is_legal_target(target_card, spell_owner, &spell_colors) {
                                valid_targets.push(card_id);
                            }
                        }
                    }
                }
                Effect::PutCounter { target, .. } if target.is_placeholder() => {
                    // PutCounter targets creatures (e.g., +1/+1 counter effects)
                    // TODO(mtg-n36vb): Some PutCounter effects can target any permanent
                    for &card_id in &self.battlefield.cards {
                        if let Ok(target_card) = self.cards.get(card_id) {
                            if target_card.is_creature() && is_legal_target(target_card, spell_owner, &spell_colors) {
                                valid_targets.push(card_id);
                            }
                        }
                    }
                }
                Effect::CopyPermanent {
                    target, restriction, ..
                } if target.is_placeholder() => {
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

                            if type_matches
                                && controller_matches
                                && is_legal_target(target_card, spell_owner, &spell_colors)
                            {
                                valid_targets.push(card_id);
                            }
                        }
                    }
                }
                Effect::Earthbend { target, .. } if target.is_placeholder() => {
                    // Earthbend targets lands you control
                    for &card_id in &self.battlefield.cards {
                        if let Ok(card) = self.cards.get(card_id) {
                            if card.is_land()
                                && card.controller == spell_owner
                                && is_legal_target(card, spell_owner, &spell_colors)
                            {
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
                            Effect::DestroyPermanent {
                                target, restriction, ..
                            } if target.is_placeholder() => {
                                // This mode destroys a permanent
                                for &card_id in &self.battlefield.cards {
                                    if let Ok(target_card) = self.cards.get(card_id) {
                                        if restriction.matches(target_card)
                                            && is_legal_target(target_card, spell_owner, &spell_colors)
                                        {
                                            valid_targets.push(card_id);
                                        }
                                    }
                                }
                            }
                            Effect::CopyPermanent {
                                target, restriction, ..
                            } if target.is_placeholder() => {
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
                                            && is_legal_target(target_card, spell_owner, &spell_colors)
                                        {
                                            valid_targets.push(card_id);
                                        }
                                    }
                                }
                            }
                            // Modal effects that don't need permanent/creature targets
                            // (target players, self, or have targets pre-specified)
                            Effect::DealDamage { .. }
                            | Effect::DealDamageXPaid { .. }
                            | Effect::DealDamageDivided { .. }
                            | Effect::DealDamageDynamic { .. }
                            | Effect::DealDamageToTriggeredPlayer { .. }
                            | Effect::EachDamage { .. }
                            | Effect::DrawCards { .. }
                            | Effect::DrawCardsXPaid { .. }
                            | Effect::DiscardCards { .. }
                            | Effect::DiscardCardsXPaid { .. }
                            | Effect::Loot { .. }
                            | Effect::GainLife { .. }
                            | Effect::GainLifeDynamic { .. }
                            | Effect::Mill { .. }
                            | Effect::DrainMana { .. }
                            | Effect::Scry { .. }
                            | Effect::Surveil { .. }
                            | Effect::AddMana { .. }
                            | Effect::Balance { .. }
                            | Effect::CreateToken { .. }
                            | Effect::CreateTokenWithStoredPt { .. }
                            | Effect::Dig { .. }
                            | Effect::SearchLibrary { .. }
                            | Effect::Firebend { .. }
                            | Effect::SetBasePowerToughness { .. }
                            | Effect::CounterSpell { .. }
                            | Effect::PumpCreature { .. }
                            | Effect::DebuffCreature { .. }
                            | Effect::PumpCreatureVariable { .. }
                            | Effect::TapPermanent { .. }
                            | Effect::UntapPermanent { .. }
                            | Effect::TapOrUntapPermanent { .. }
                            | Effect::ReturnPermanentToHand { .. }
                            | Effect::ExilePermanent { .. }
                            | Effect::Airbend { .. }
                            | Effect::Earthbend { .. }
                            | Effect::GrantCantBeBlocked { .. }
                            | Effect::Regenerate { .. }
                            | Effect::PreventDamage { .. }
                            | Effect::PreventDamageFromSource { .. }
                            | Effect::RemoveCounter { .. }
                            | Effect::PutCounter { .. }
                            | Effect::MultiplyCounter { .. }
                            | Effect::PutCounterAll { .. }
                            | Effect::ChangeZoneAll { .. }
                            | Effect::AttachEquipment { .. }
                            | Effect::ModalChoice { .. }
                            | Effect::PumpAllCreatures { .. }
                            | Effect::AnimateAll { .. }
                            | Effect::DestroyAll { .. }
                            | Effect::SacrificeAll { .. }
                            | Effect::DamageAll { .. }
                            | Effect::LoseLife { .. }
                            | Effect::ForceSacrifice { .. }
                            | Effect::SacrificeSelf { .. }
                            | Effect::TapAll { .. }
                            | Effect::UntapAll { .. }
                            | Effect::UntapOne { .. }
                            | Effect::SetLife { .. }
                            | Effect::CreateDelayedTrigger { .. }
                            | Effect::CopySpellAbility { .. }
                            | Effect::ImmediateTrigger { .. }
                            | Effect::ClearRemembered
                            | Effect::AddTurn { .. }
                            | Effect::AddPhase { .. }
                            | Effect::ChooseName { .. }
                            | Effect::ChooseColor { .. }
                            | Effect::Clone { .. }
                            | Effect::Proliferate
                            | Effect::Unimplemented { .. }
                            | Effect::NoOp { .. }
                            | Effect::ClassLevelUp { .. }
                            | Effect::UnlessCostWrapper { .. }
                            | Effect::GainControl { .. }
                            | Effect::Fight { .. }
                            | Effect::SelfExileFromStack { .. }
                            | Effect::MoveSelfBetweenZones { .. }
                            | Effect::ReturnCardsFromGraveyardToHand { .. }
                            | Effect::PutCardsFromHandOnTopOfLibrary { .. }
                            | Effect::RevealCardsFromHand { .. }
                            | Effect::ReturnGraveyardCardToHand { .. }
                            | Effect::ReturnGraveyardCardToZone { .. }
                            | Effect::PutCreatureFromHandOnBattlefield { .. }
                            | Effect::ReturnSelfAsEnchantment { .. }
                            | Effect::PreventAllCombatDamageThisTurn { .. }
                            | Effect::ExileIfWouldDieThisTurn { .. }
                            | Effect::ConditionalSelfCounter { .. }
                            | Effect::RearrangeTopOfLibrary { .. }
                            | Effect::SkipUntapStep { .. }
                            | Effect::CreateTokenDynamic { .. }
                            | Effect::CreateEmblem { .. }
                            | Effect::PlayFromGraveyard { .. }
                            | Effect::RepeatEach { .. }
                            | Effect::ExtraLandPlay { .. }
                            | Effect::TapPermanentsMatchingFilter { .. }
                            | Effect::ChooseAndRememberOneOfEach { .. }
                            | Effect::GrantCastWithFlash { .. } => {
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
                | Effect::DrawCardsXPaid { .. }
                | Effect::DiscardCards { .. }
                | Effect::DiscardCardsXPaid { .. }
                | Effect::Loot { .. }
                | Effect::GainLife { .. }
                | Effect::GainLifeDynamic { .. }
                | Effect::Mill { .. }
                | Effect::DrainMana { .. }
                | Effect::Scry { .. }
                | Effect::Surveil { .. }
                | Effect::AddMana { .. }
                | Effect::Balance { .. }
                | Effect::CreateToken { .. }
                | Effect::CreateTokenWithStoredPt { .. }
                | Effect::Dig { .. }
                | Effect::SearchLibrary { .. }
                | Effect::Firebend { .. }
                | Effect::SetBasePowerToughness { .. }
                | Effect::AddTurn { .. }
                | Effect::AddPhase { .. }
                | Effect::AttachEquipment { .. }
                | Effect::ForceSacrifice { .. }
                | Effect::SacrificeSelf { .. }
                | Effect::TapAll { .. }
                | Effect::UntapAll { .. }
                | Effect::UntapOne { .. }
                | Effect::SetLife { .. }
                | Effect::ChooseName { .. }
                | Effect::ChooseColor { .. }
                | Effect::Clone { .. }
                | Effect::Unimplemented { .. }
                | Effect::NoOp { .. }
                | Effect::ClassLevelUp { .. }
                | Effect::Proliferate
                // Phase-trigger damage: target player resolved at trigger time,
                // no cast-time targeting.
                | Effect::DealDamageToTriggeredPlayer { .. }
                | Effect::SelfExileFromStack { .. }
                | Effect::MoveSelfBetweenZones { .. }
                | Effect::ReturnCardsFromGraveyardToHand { .. }
                | Effect::PutCardsFromHandOnTopOfLibrary { .. }
                | Effect::RevealCardsFromHand { .. }
                | Effect::ReturnGraveyardCardToHand { .. }
                | Effect::ReturnGraveyardCardToZone { .. }
                | Effect::PutCreatureFromHandOnBattlefield { .. }
                | Effect::ReturnSelfAsEnchantment { .. }
                | Effect::PreventAllCombatDamageThisTurn { .. }
                | Effect::ConditionalSelfCounter { .. }
                | Effect::RearrangeTopOfLibrary { .. }
                | Effect::SkipUntapStep { .. }
                | Effect::CreateTokenDynamic { .. }
                | Effect::CreateEmblem { .. }
                // PlayFromGraveyard is triggered from a planeswalker activated ability;
                // targeting is handled through get_valid_targets_for_ability, not here.
                | Effect::PlayFromGraveyard { .. }
                // RepeatEach targets are consumed from chosen_targets at resolve_effect_target
                // time (Pattern A) or iterate over players (Pattern B) — no cast-time targeting.
                | Effect::RepeatEach { .. }
                // ExtraLandPlay grants permission to the spell controller; no cast-time target.
                | Effect::ExtraLandPlay { .. }
                // TapPermanentsMatchingFilter uses a filter, not a specific cast-time target.
                | Effect::TapPermanentsMatchingFilter { .. }
                // ChooseAndRememberOneOfEach reads from remembered_players; no cast-time target.
                | Effect::ChooseAndRememberOneOfEach { .. } => {
                    // These effects target players or have no targeting requirements
                    // AttachEquipment targeting is handled via Equip keyword abilities
                    // ChooseColor is a player choice effect (no permanent targets)
                    // Clone chooses which permanent to copy during resolution (ETB)
                    // Proliferate: player chooses permanents/players during resolution, no targeting
                    // SelfExileFromStack: operates on the resolving spell itself, no targets
                    // ReturnCardsFromGraveyardToHand: works on the caster's graveyard, no targeting
                    // PutCardsFromHandOnTopOfLibrary: controller picks cards during resolution, no targeting
                    // RevealCardsFromHand: controller reveals their own hand cards, no cast-time targeting
                    // ReturnGraveyardCardToHand: AI picks matching card, no cast-time targeting
                    // ReturnGraveyardCardToZone: AI picks matching card, no cast-time targeting
                    // PutCreatureFromHandOnBattlefield: AI picks creature, no cast-time targeting
                    // ReturnSelfAsEnchantment: death trigger self-return, no cast-time targeting
                    // PreventAllCombatDamageThisTurn: reuses last_resolved_target, no cast-time target
                    // PlayFromGraveyard: targeting via get_valid_targets_for_ability (activated ability path)
                    // RearrangeTopOfLibrary: controller looks at own library top, no cast-time targeting
                    // SkipUntapStep: opponent auto-targeted (resolved from ValidTgts$ during init), no
                    //   separate cast-time targeting step needed
                    // ExtraLandPlay: target is the spell controller, no permanent target needed
                }
                // Effects with already-specified targets (non-zero target field)
                // The handlers above only match when target.is_placeholder()
                Effect::DealDamage { .. }
                | Effect::DealDamageXPaid { .. }
                | Effect::DealDamageDivided { .. }
                | Effect::DealDamageDynamic { .. } => {
                    // Either TargetRef::Player (already specified) or TargetRef::Permanent (already specified)
                    // TargetRef::None case handled above
                }
                Effect::PreventDamage { .. } => {
                    // PreventDamage with Defined$ Self or Defined$ You - target already specified
                    // TargetRef::None case handled above
                }
                Effect::PreventDamageFromSource { .. } => {
                    // Circle of Protection is an activated ability, not a spell;
                    // its source is chosen via get_valid_targets_for_ability.
                }
                Effect::DestroyPermanent { .. }
                | Effect::GainControl { .. }
                | Effect::PumpCreature { .. }
                | Effect::DebuffCreature { .. }
                | Effect::PumpCreatureVariable { .. }
                | Effect::TapPermanent { .. }
                | Effect::UntapPermanent { .. }
                | Effect::TapOrUntapPermanent { .. }
                | Effect::CounterSpell { .. }
                | Effect::ReturnPermanentToHand { .. }
                | Effect::ExilePermanent { .. }
                // ExileIfWouldDieThisTurn rides on the parent DealDamage's
                // target (no independent targeting); nothing to enumerate here.
                | Effect::ExileIfWouldDieThisTurn { .. }
                | Effect::Airbend { .. }
                | Effect::GrantCantBeBlocked { .. }
                | Effect::Regenerate { .. }
                | Effect::RemoveCounter { .. }
                | Effect::PutCounter { .. }
                | Effect::MultiplyCounter { .. }
                | Effect::PutCounterAll { .. }
                | Effect::ChangeZoneAll { .. }
                | Effect::CopyPermanent { .. }
                | Effect::PumpAllCreatures { .. }
                | Effect::AnimateAll { .. }
                | Effect::DestroyAll { .. }
                | Effect::SacrificeAll { .. }
                | Effect::DamageAll { .. }
                | Effect::LoseLife { .. }
                | Effect::CreateDelayedTrigger { .. }
                | Effect::CopySpellAbility { .. }
                | Effect::ImmediateTrigger { .. }
                | Effect::ClearRemembered
                | Effect::EachDamage { .. }
                | Effect::Earthbend { .. }
                | Effect::Fight { .. } => {
                    // Target already specified (guard failed: target.as_u32() != 0)
                    // This means the effect has a concrete target already assigned
                    // PumpAllCreatures doesn't use explicit targets - it affects all matching creatures
                    // CreateDelayedTrigger with non-zero tracked_card already has target
                    // CopySpellAbility doesn't use explicit targets - copies triggering spell
                    // ImmediateTrigger/ClearRemembered don't need targets - work with remembered state
                    // EachDamage gets targets from parent ability's ValidTgts, resolved at spell resolution
                    // Fight with non-placeholder targets already has both fighters assigned
                }
                // UnlessCostWrapper delegates targeting to inner effect
                // TODO(mtg-n36vb): Handle inner effect targeting when implementing UnlessCost resolution
                Effect::UnlessCostWrapper { .. } => {
                    // For now, skip - inner effect targeting handled when we implement full UnlessCost
                }
                // GrantCastWithFlash operates on the controller (no cast-time target needed).
                Effect::GrantCastWithFlash { .. } => {}
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
                            if matches_type(target_card) && is_legal_target(target_card, spell_owner, &spell_colors) {
                                valid_targets.push(card_id);
                            }
                        }
                    }
                }
            }
        }

        // Sort for deterministic ordering (critical for snapshot/resume)
        valid_targets.sort();
        // Then offer opponents' player sentinels before the caster's own
        // (mtg-605). This runs AFTER the numeric sort (which would otherwise
        // put the low-id caster's sentinel first) and is itself deterministic —
        // keyed only on the spell owner — so snapshot/resume and network sync
        // are unaffected.
        crate::core::reorder_player_targets_opponents_first(&mut valid_targets, spell_owner);
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
            | Cost::DiscardHand
            | Cost::Waterbend { .. }
            | Cost::AddLoyalty { .. }
            | Cost::SubLoyalty { .. }
            | Cost::SubCounter { .. } => false,
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
        let source_colors = source_card.colors.clone();

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
                Effect::DestroyPermanent {
                    target, restriction, ..
                } if target.is_placeholder() => {
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

                            // Check spell-level type + controller restriction from ValidTgts
                            // (matches_with_controller honors YouCtrl/OppCtrl in addition to type/counter/power)
                            if !restriction.matches_with_controller(card, ability_controller, card.controller) {
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
                            if !is_legal_target(card, ability_controller, &source_colors) {
                                is_valid = false;
                            }

                            if is_valid {
                                valid_targets.push(card_id);
                            }
                        }
                    }
                }
                Effect::TapPermanent { target } if target.is_placeholder() => {
                    // Tap can target untapped permanents
                    for &card_id in &self.battlefield.cards {
                        if let Ok(card) = self.cards.get(card_id) {
                            let is_valid = !card.tapped && is_legal_target(card, ability_controller, &source_colors);

                            if is_valid {
                                valid_targets.push(card_id);
                            }
                        }
                    }
                }
                Effect::UntapPermanent { target } if target.is_placeholder() => {
                    // Untap can target tapped permanents
                    for &card_id in &self.battlefield.cards {
                        if let Ok(card) = self.cards.get(card_id) {
                            let is_valid = card.tapped && is_legal_target(card, ability_controller, &source_colors);

                            if is_valid {
                                valid_targets.push(card_id);
                            }
                        }
                    }
                }
                Effect::TapOrUntapPermanent { target } if target.is_placeholder() => {
                    // Tap or untap any permanent
                    for &card_id in &self.battlefield.cards {
                        if let Ok(card) = self.cards.get(card_id) {
                            if is_legal_target(card, ability_controller, &source_colors) {
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
                            let is_valid =
                                card.is_creature() && is_legal_target(card, ability_controller, &source_colors);

                            if is_valid {
                                valid_targets.push(card_id);
                            }
                        }
                    }
                }
                Effect::AttachEquipment { target_creature, .. } if target_creature.is_placeholder() => {
                    // Equip targets "creature you control" (CR 702.6a)
                    // Exclude the creature this Equipment is already attached to: re-equipping
                    // the same creature is a strictly wasteful no-op (detach + reattach burns
                    // mana for no game effect). This makes all controllers (random, heuristic,
                    // etc.) avoid that pitfall.
                    let already_attached_to = source_card.attached_to;
                    for &card_id in &self.battlefield.cards {
                        if let Ok(card) = self.cards.get(card_id) {
                            // Must be a creature
                            let mut is_valid = card.is_creature();

                            // Must be controlled by the ability's controller
                            if card.controller != ability_controller {
                                is_valid = false;
                            }

                            // Skip the creature already wearing this Equipment
                            if already_attached_to == Some(card_id) {
                                is_valid = false;
                            }

                            // Check shroud/hexproof (CR 702.18, 702.19)
                            // Note: Hexproof doesn't typically apply when we control both the Equipment
                            // and the target, but we check owner-based targeting for consistency
                            if !is_legal_target(card, ability_controller, &source_colors) {
                                is_valid = false;
                            }

                            if is_valid {
                                valid_targets.push(card_id);
                            }
                        }
                    }
                }
                Effect::Airbend { target } if target.is_placeholder() => {
                    // Airbend targets creatures (or other permanents based on ValidTgts)
                    // CR 701.65b: Airbend exiles a target permanent
                    for &card_id in &self.battlefield.cards {
                        if let Ok(card) = self.cards.get(card_id) {
                            // By default, Airbend targets creatures
                            // TODO(mtg-n36vb): Could be extended with ValidTgts parsing for nonland permanents
                            let mut is_valid = card.is_creature();

                            // Check shroud/hexproof
                            if !is_legal_target(card, ability_controller, &source_colors) {
                                is_valid = false;
                            }

                            if is_valid {
                                valid_targets.push(card_id);
                            }
                        }
                    }
                }
                Effect::GrantCantBeBlocked { target } if target.is_placeholder() => {
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
                            if !is_legal_target(card, ability_controller, &source_colors) {
                                is_valid = false;
                            }

                            if is_valid {
                                valid_targets.push(card_id);
                            }
                        }
                    }
                }
                Effect::GainControl {
                    target, restriction, ..
                } if target.is_placeholder() => {
                    // Activated GainControl (Aladdin `{1}{R}{R},{T}: gain control of
                    // target artifact`; Old Man of the Sea `{T}: gain control of
                    // target creature with power ≤ CARDNAME's power`). The spell
                    // path (get_valid_targets_for_spell) had a GainControl arm but
                    // the activated path did not, so these abilities enumerated ZERO
                    // targets and were never offered (mtg-713 B1). Honor the parsed
                    // ValidTgts$ restriction (type + controller) and the dynamic
                    // `powerLEX` threshold (X = this source's current power).
                    let source_power = i32::from(source_card.current_power());
                    for &card_id in &self.battlefield.cards {
                        if let Ok(card) = self.cards.get(card_id) {
                            if restriction.matches_with_source_power(card, source_power)
                                && restriction.matches_with_controller(card, ability_controller, card.controller)
                                && is_legal_target(card, ability_controller, &source_colors)
                            {
                                valid_targets.push(card_id);
                            }
                        }
                    }
                }
                Effect::PumpCreature { target, .. } if target.is_placeholder() => {
                    // Pump / Debuff targeting a creature from an *activated* ability
                    // (e.g. Mishra's Factory's `{T}: Target Assembly-Worker
                    // creature gets +1/+1`). The spell-cast path already enumerates
                    // these via effect_has_valid_targets; the activated-ability path
                    // previously fell through to the "target already specified"
                    // catch-all and offered NO targets, so any tap/mana-cost pump
                    // ability with a placeholder target was silently never offered
                    // at the action menu. Enumerate creatures honoring the ability's
                    // cached tapped/untapped restriction (mtg-522).
                    for &card_id in &self.battlefield.cards {
                        if let Ok(card) = self.cards.get(card_id) {
                            let mut is_valid =
                                card.is_creature() && is_legal_target(card, ability_controller, &source_colors);
                            if requires_tapped && !card.tapped {
                                is_valid = false;
                            }
                            if requires_untapped && card.tapped {
                                is_valid = false;
                            }
                            if is_valid {
                                valid_targets.push(card_id);
                            }
                        }
                    }
                }
                Effect::DebuffCreature { target, .. } if target.is_placeholder() => {
                    // Debuff (keyword removal) targeting a creature from an activated
                    // ability — same enumeration gap as PumpCreature above (mtg-522).
                    for &card_id in &self.battlefield.cards {
                        if let Ok(card) = self.cards.get(card_id) {
                            let mut is_valid =
                                card.is_creature() && is_legal_target(card, ability_controller, &source_colors);
                            if requires_tapped && !card.tapped {
                                is_valid = false;
                            }
                            if requires_untapped && card.tapped {
                                is_valid = false;
                            }
                            if is_valid {
                                valid_targets.push(card_id);
                            }
                        }
                    }
                }
                Effect::PumpCreatureVariable { target, .. } if target.is_placeholder() => {
                    // Variable pump (+X/+X) targeting a creature from an activated
                    // ability — same enumeration gap as PumpCreature above (mtg-522).
                    for &card_id in &self.battlefield.cards {
                        if let Ok(card) = self.cards.get(card_id) {
                            let mut is_valid =
                                card.is_creature() && is_legal_target(card, ability_controller, &source_colors);
                            if requires_tapped && !card.tapped {
                                is_valid = false;
                            }
                            if requires_untapped && card.tapped {
                                is_valid = false;
                            }
                            if is_valid {
                                valid_targets.push(card_id);
                            }
                        }
                    }
                }
                Effect::Earthbend { target, .. } if target.is_placeholder() => {
                    // Earthbend targets lands you control
                    // CR 701.65: Earthbend makes a land into a creature with counters
                    for &card_id in &self.battlefield.cards {
                        if let Ok(card) = self.cards.get(card_id) {
                            // Must be a land we control
                            let is_valid = card.is_land() && card.controller == ability_controller;

                            // Note: Lands typically can't have shroud/hexproof,
                            // but check anyway for completeness
                            if is_valid && is_legal_target(card, ability_controller, &source_colors) {
                                valid_targets.push(card_id);
                            }
                        }
                    }
                }
                Effect::PreventDamageFromSource { color, source, .. } if source.is_placeholder() => {
                    // Circle of Protection: choose a source of the shield's
                    // colour. Chooseable sources are matching-colour objects
                    // that could actually deal damage to the controller —
                    // creatures on the battlefield and spells on the stack
                    // (e.g. a red burn spell). We exclude the Circle's own
                    // enchantment (it is itself a coloured permanent for the
                    // self-coloured CoPs but deals no damage, so choosing it is
                    // never useful) and other non-damaging permanents, which
                    // keeps the auto-chooser pointed at real threats. The
                    // colour test uses public characteristics only.
                    for &card_id in &self.battlefield.cards {
                        if card_id == source_card_id {
                            continue;
                        }
                        if let Ok(card) = self.cards.get(card_id) {
                            if card.is_creature()
                                && card.is_color(*color)
                                && is_legal_target(card, ability_controller, &source_colors)
                            {
                                valid_targets.push(card_id);
                            }
                        }
                    }
                    for &card_id in &self.stack.cards {
                        if let Ok(card) = self.cards.get(card_id) {
                            if card.is_color(*color) {
                                valid_targets.push(card_id);
                            }
                        }
                    }
                }
                // ===== EXHAUSTIVE EFFECT HANDLING FOR ABILITY TARGETING =====
                // Effects that don't need targets or have targets pre-specified
                // SetLife that targets a player by name: Sorin Markov -3
                // ("Target opponent's life total becomes 10."). The effect is parsed
                // with player=placeholder; the ability description carries "target
                // opponent" or "target player", so we enumerate the appropriate
                // player set here using the ability's AbilityCache flags.
                Effect::SetLife { player, .. } if player.is_placeholder() && ability.cache.requires_target => {
                    if ability.cache.targets_opponent {
                        // Enumerate opponents only
                        for player in &self.players {
                            if player.id != ability_controller {
                                valid_targets.push(crate::core::player_as_target_sentinel(player.id));
                            }
                        }
                    } else if ability.cache.targets_player {
                        // Enumerate all players
                        for player in &self.players {
                            valid_targets.push(crate::core::player_as_target_sentinel(player.id));
                        }
                    }
                }

                Effect::DrawCards { .. }
                | Effect::DrawCardsXPaid { .. }
                | Effect::DiscardCards { .. }
                | Effect::DiscardCardsXPaid { .. }
                | Effect::Loot { .. }
                | Effect::GainLife { .. }
                | Effect::GainLifeDynamic { .. }
                | Effect::Mill { .. }
                | Effect::DrainMana { .. }
                | Effect::Scry { .. }
                | Effect::Surveil { .. }
                | Effect::AddMana { .. }
                | Effect::Balance { .. }
                | Effect::CreateToken { .. }
                | Effect::CreateTokenWithStoredPt { .. }
                | Effect::Dig { .. }
                | Effect::SearchLibrary { .. }
                | Effect::Firebend { .. }
                | Effect::SetBasePowerToughness { .. }
                | Effect::ModalChoice { .. }
                | Effect::CreateDelayedTrigger { .. }
                | Effect::CopySpellAbility { .. }
                | Effect::ImmediateTrigger { .. }
                | Effect::ClearRemembered
                | Effect::AddTurn { .. }
                | Effect::AddPhase { .. }
                | Effect::DestroyAll { .. }
                | Effect::PutCounterAll { .. }
                | Effect::UntapAll { .. }
                | Effect::UntapOne { .. }
                | Effect::Unimplemented { .. }
                | Effect::NoOp { .. }
                | Effect::ClassLevelUp { .. }
                | Effect::EachDamage { .. }
                | Effect::ForceSacrifice { .. }
                | Effect::SacrificeSelf { .. }
                | Effect::TapAll { .. }
                | Effect::SetLife { .. }
                | Effect::ChooseName { .. }
                | Effect::ChooseColor { .. }
                | Effect::Clone { .. }
                | Effect::Proliferate
                | Effect::DealDamageToTriggeredPlayer { .. }
                | Effect::SelfExileFromStack { .. }
                | Effect::MoveSelfBetweenZones { .. }
                | Effect::ReturnCardsFromGraveyardToHand { .. }
                | Effect::PutCardsFromHandOnTopOfLibrary { .. }
                | Effect::RevealCardsFromHand { .. }
                | Effect::ReturnGraveyardCardToHand { .. }
                | Effect::ReturnGraveyardCardToZone { .. }
                | Effect::PutCreatureFromHandOnBattlefield { .. }
                | Effect::ReturnSelfAsEnchantment { .. }
                | Effect::PreventAllCombatDamageThisTurn { .. }
                | Effect::ConditionalSelfCounter { .. }
                | Effect::RearrangeTopOfLibrary { .. }
                | Effect::SkipUntapStep { .. }
                | Effect::UnlessCostWrapper { .. }
                | Effect::CreateTokenDynamic { .. }
                | Effect::CreateEmblem { .. }
                // RepeatEach targets are consumed from chosen_targets at resolve_effect_target
                // time (Pattern A) or iterate over players (Pattern B) — no cast-time targeting.
                | Effect::RepeatEach { .. }
                | Effect::ExtraLandPlay { .. }
                // TapPermanentsMatchingFilter uses a filter, not a cast-time target.
                | Effect::TapPermanentsMatchingFilter { .. }
                // ChooseAndRememberOneOfEach reads from remembered_players; no cast-time target.
                | Effect::ChooseAndRememberOneOfEach { .. } => {
                    // These effects target players or have no targeting requirements
                    // CreateDelayedTrigger targets creatures - handled via ValidTgts$ Creature
                    // CopySpellAbility doesn't need explicit targets - copies triggering spell
                    // ImmediateTrigger/ClearRemembered work with remembered state, no targeting
                    // EachDamage targeting is handled via parent ability's ValidTgts$
                    // UnlessCostWrapper delegates targeting to inner effect
                    // Proliferate: player chooses during resolution, no targeting
                    // SelfExileFromStack: operates on the resolving spell itself, no targets
                    // ReturnCardsFromGraveyardToHand: uses remembered_cards count, no targeting
                    // PutCardsFromHandOnTopOfLibrary: controller picks cards during resolution, no targeting
                    // RevealCardsFromHand: controller reveals their own hand cards, no cast-time targeting
                    // ReturnGraveyardCardToHand: AI picks matching card, no cast-time targeting
                    // ReturnSelfAsEnchantment: death trigger self-return, no cast-time targeting
                    // ExtraLandPlay: target is the spell controller, no permanent target needed
                    // PreventAllCombatDamageThisTurn: reuses UntapPermanent's last_resolved_target
                    // RearrangeTopOfLibrary: controller looks at own library top, no targeting
                    // SkipUntapStep: opponent auto-targeted (ValidTgts$ resolved at init)
                    // RepeatEach: targets resolved from chosen_targets at execution time
                }
                Effect::PlayFromGraveyard {
                    target,
                    type_filter,
                    max_mana_value,
                    ..
                } if target.is_placeholder() => {
                    // Target: an instant or sorcery card in the controller's graveyard
                    // that matches the type_filter and (optionally) has CMC <= max_mana_value.
                    // Per Chandra's −2: ValidTgts$ Instant.YouCtrl+cmcLE3,Sorcery.YouCtrl+cmcLE3
                    if let Some(zones) = self.get_player_zones(ability_controller) {
                        let graveyard_cards: SmallVec<[CardId; 16]> =
                            zones.graveyard.cards.iter().copied().collect();
                        for card_id in graveyard_cards {
                            if let Ok(card) = self.cards.get(card_id) {
                                // Must be instant or sorcery (from type_filter, or default)
                                let type_ok = if type_filter.is_empty() {
                                    card.is_type(&crate::core::CardType::Instant)
                                        || card.is_type(&crate::core::CardType::Sorcery)
                                } else {
                                    type_filter.split(',').any(|t| {
                                        let t = t.trim().split('.').next().unwrap_or(t);
                                        match t {
                                            "Instant" => card.is_type(&crate::core::CardType::Instant),
                                            "Sorcery" => card.is_type(&crate::core::CardType::Sorcery),
                                            _ => false,
                                        }
                                    })
                                };
                                if !type_ok {
                                    continue;
                                }
                                // Check CMC restriction
                                if let Some(max_cmc) = max_mana_value {
                                    if card.mana_cost.cmc() > *max_cmc {
                                        continue;
                                    }
                                }
                                valid_targets.push(card_id);
                            }
                        }
                    }
                }
                // Effects with pre-specified targets (guard failed: target.as_u32() != 0)
                Effect::DealDamage { .. }
                | Effect::DealDamageXPaid { .. }
                | Effect::DealDamageDivided { .. }
                | Effect::DealDamageDynamic { .. } => {
                    // TargetRef::Player/Permanent - target already specified (or resolved below)
                }
                Effect::DestroyPermanent { .. }
                | Effect::PumpCreature { .. }
                | Effect::DebuffCreature { .. }
                | Effect::PumpCreatureVariable { .. }
                | Effect::TapPermanent { .. }
                | Effect::UntapPermanent { .. }
                | Effect::TapOrUntapPermanent { .. }
                | Effect::CounterSpell { .. }
                | Effect::ReturnPermanentToHand { .. }
                | Effect::ExilePermanent { .. }
                | Effect::ExileIfWouldDieThisTurn { .. }
                | Effect::Airbend { .. }
                | Effect::GrantCantBeBlocked { .. }
                | Effect::Regenerate { .. }
                | Effect::PreventDamage { .. }
                | Effect::PreventDamageFromSource { .. }
                | Effect::RemoveCounter { .. }
                | Effect::PutCounter { .. }
                | Effect::MultiplyCounter { .. }
                | Effect::ChangeZoneAll { .. }
                | Effect::CopyPermanent { .. }
                | Effect::AttachEquipment { .. }
                | Effect::PumpAllCreatures { .. }
                | Effect::AnimateAll { .. }
                | Effect::SacrificeAll { .. }
                | Effect::DamageAll { .. }
                | Effect::LoseLife { .. }
                | Effect::Earthbend { .. }
                | Effect::GainControl { .. }
                | Effect::Fight { .. }
                // PlayFromGraveyard targets a graveyard card; targeting is handled
                // through get_valid_targets_for_ability (activated ability path).
                // If it ever appears as a spell sub-effect, target is pre-specified.
                | Effect::PlayFromGraveyard { .. }
                // GrantCastWithFlash grants the controller flash-casting permission;
                // no cast-time target needed for activated abilities either.
                | Effect::GrantCastWithFlash { .. } => {
                    // Target already specified (guard failed: target.as_u32() != 0)
                    // PumpAllCreatures doesn't use explicit targets - it affects all matching creatures
                    // Earthbend target was handled above when target.is_placeholder()
                    // ChooseColor doesn't require any targets - it's a player choice effect
                    // PlayFromGraveyard: activated-ability targeting via get_valid_targets_for_ability
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
        let source_colors = spell_card.colors.clone();

        for effect in &spell_card.effects {
            if let Effect::ModalChoice { modes, .. } = effect {
                let mut result = Vec::with_capacity(modes.len());

                for (idx, mode) in modes.iter().enumerate() {
                    let has_targets = self.effect_has_valid_targets(&mode.effect, spell_owner, &source_colors);
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
    fn effect_has_valid_targets(
        &self,
        effect: &Effect,
        spell_owner: PlayerId,
        source_colors: &[crate::core::Color],
    ) -> bool {
        match effect {
            Effect::DestroyPermanent {
                target, restriction, ..
            } if target.is_placeholder() => {
                // Check if any permanent matches the restriction
                self.battlefield.cards.iter().any(|&card_id| {
                    if let Ok(card) = self.cards.get(card_id) {
                        restriction.matches(card) && is_legal_target(card, spell_owner, source_colors)
                    } else {
                        false
                    }
                })
            }
            Effect::RemoveCounter { target, .. } if target.is_placeholder() => {
                // RemoveCounter can target any creature (mode 2 of Heartless Act)
                self.battlefield.cards.iter().any(|&card_id| {
                    if let Ok(card) = self.cards.get(card_id) {
                        card.is_creature() && is_legal_target(card, spell_owner, source_colors)
                    } else {
                        false
                    }
                })
            }
            Effect::PreventDamageFromSource { color, source, .. } if source.is_placeholder() => {
                // Circle of Protection needs at least one matching-colour
                // damage source to choose: a creature on the battlefield or a
                // spell on the stack (mirrors get_valid_targets_for_ability).
                let creature_source = self.battlefield.cards.iter().any(|&card_id| {
                    self.cards
                        .get(card_id)
                        .map(|card| card.is_creature() && card.is_color(*color))
                        .unwrap_or(false)
                });
                let stack_source = self.stack.cards.iter().any(|&card_id| {
                    self.cards
                        .get(card_id)
                        .map(|card| card.is_color(*color))
                        .unwrap_or(false)
                });
                creature_source || stack_source
            }
            Effect::PumpCreature { target, .. } if target.is_placeholder() => {
                // Pump requires a creature target
                self.battlefield.cards.iter().any(|&card_id| {
                    if let Ok(card) = self.cards.get(card_id) {
                        card.is_creature() && is_legal_target(card, spell_owner, source_colors)
                    } else {
                        false
                    }
                })
            }
            Effect::DebuffCreature { target, .. } if target.is_placeholder() => {
                // Debuff requires a creature target
                self.battlefield.cards.iter().any(|&card_id| {
                    if let Ok(card) = self.cards.get(card_id) {
                        card.is_creature() && is_legal_target(card, spell_owner, source_colors)
                    } else {
                        false
                    }
                })
            }
            Effect::TapPermanent { target } if target.is_placeholder() => {
                // Tap requires an untapped permanent
                self.battlefield.cards.iter().any(|&card_id| {
                    if let Ok(card) = self.cards.get(card_id) {
                        !card.tapped && is_legal_target(card, spell_owner, source_colors)
                    } else {
                        false
                    }
                })
            }
            Effect::UntapPermanent { target } if target.is_placeholder() => {
                // Untap requires a tapped permanent
                self.battlefield.cards.iter().any(|&card_id| {
                    if let Ok(card) = self.cards.get(card_id) {
                        card.tapped && is_legal_target(card, spell_owner, source_colors)
                    } else {
                        false
                    }
                })
            }
            Effect::TapOrUntapPermanent { target } if target.is_placeholder() => {
                // Tap or untap can target any permanent
                !self.battlefield.cards.is_empty()
            }
            Effect::DealDamage {
                target: TargetRef::None,
                ..
            }
            | Effect::DealDamageXPaid {
                target: TargetRef::None,
                ..
            } => {
                // Damage can target creatures or players - always has targets if there's a creature
                // (players are always valid targets)
                true
            }
            Effect::ReturnPermanentToHand { target, restriction } if target.is_placeholder() => {
                // Bounce requires at least one permanent matching the ValidTgts$ filter.
                let restr = restriction.clone();
                self.battlefield.cards.iter().any(|&card_id| {
                    if let Ok(card) = self.cards.get(card_id) {
                        restr.matches(card) && is_legal_target(card, spell_owner, source_colors)
                    } else {
                        false
                    }
                })
            }
            Effect::ExilePermanent { target } if target.is_placeholder() => {
                // Exile requires a permanent target
                self.battlefield.cards.iter().any(|&card_id| {
                    if let Ok(card) = self.cards.get(card_id) {
                        is_legal_target(card, spell_owner, source_colors)
                    } else {
                        false
                    }
                })
            }
            Effect::CounterSpell { target, .. } if target.is_placeholder() => {
                // Counter requires a spell on the stack
                !self.stack.is_empty()
            }
            Effect::Earthbend { target, .. } if target.is_placeholder() => {
                // Earthbend targets lands you control
                self.battlefield.cards.iter().any(|&card_id| {
                    if let Ok(card) = self.cards.get(card_id) {
                        card.is_land() && card.controller == spell_owner
                    } else {
                        false
                    }
                })
            }
            // Effects that don't require targeting always "have targets"
            Effect::DrawCards { .. }
            | Effect::DrawCardsXPaid { .. }
            | Effect::DiscardCards { .. }
            | Effect::DiscardCardsXPaid { .. }
            | Effect::Loot { .. }
            | Effect::GainLife { .. }
            | Effect::GainLifeDynamic { .. }
            | Effect::Mill { .. }
            | Effect::DrainMana { .. }
            | Effect::Scry { .. }
            | Effect::Surveil { .. }
            | Effect::AddMana { .. }
            | Effect::Balance { .. }
            | Effect::CreateToken { .. }
            | Effect::CreateTokenWithStoredPt { .. }
            | Effect::Dig { .. }
            | Effect::SearchLibrary { .. }
            | Effect::Firebend { .. }
            | Effect::SetBasePowerToughness { .. }
            | Effect::AttachEquipment { .. }
            | Effect::ModalChoice { .. }
            | Effect::PumpAllCreatures { .. }
            | Effect::AnimateAll { .. }
            | Effect::CreateDelayedTrigger { .. }
            | Effect::CopySpellAbility { .. }
            | Effect::ImmediateTrigger { .. }
            | Effect::ClearRemembered
            | Effect::AddTurn { .. }
            | Effect::AddPhase { .. }
            | Effect::Unimplemented { .. }
            | Effect::NoOp { .. }
            | Effect::ClassLevelUp { .. }
            | Effect::EachDamage { .. }
            | Effect::UnlessCostWrapper { .. }
            | Effect::DestroyAll { .. }
            | Effect::SacrificeAll { .. }
            | Effect::DamageAll { .. }
            | Effect::LoseLife { .. }
            | Effect::ForceSacrifice { .. }
            | Effect::SacrificeSelf { .. }
            | Effect::TapAll { .. }
            | Effect::UntapAll { .. }
            | Effect::UntapOne { .. }
            | Effect::SetLife { .. }
            | Effect::GainControl { .. }
            | Effect::PutCounterAll { .. }
            | Effect::ChangeZoneAll { .. }
            | Effect::ChooseName { .. }
            | Effect::ChooseColor { .. }
            | Effect::Clone { .. }
            | Effect::Proliferate
            | Effect::DealDamageToTriggeredPlayer { .. }
            | Effect::SelfExileFromStack { .. }
            | Effect::MoveSelfBetweenZones { .. }
            | Effect::ReturnCardsFromGraveyardToHand { .. }
            | Effect::PutCardsFromHandOnTopOfLibrary { .. }
            | Effect::RevealCardsFromHand { .. }
            | Effect::ReturnGraveyardCardToHand { .. }
            | Effect::ReturnGraveyardCardToZone { .. }
            | Effect::PutCreatureFromHandOnBattlefield { .. }
            | Effect::ReturnSelfAsEnchantment { .. }
            | Effect::PreventAllCombatDamageThisTurn { .. }
            | Effect::ConditionalSelfCounter { .. }
            | Effect::RearrangeTopOfLibrary { .. }
            | Effect::SkipUntapStep { .. }
            | Effect::CreateTokenDynamic { .. }
            | Effect::CreateEmblem { .. }
            // ExileIfWouldDieThisTurn reuses the parent DealDamage's target, so
            // it never needs its own target check.
            | Effect::ExileIfWouldDieThisTurn { .. }
            | Effect::Fight { .. }
            // RepeatEach has no cast-time targets; they are resolved at execute time.
            | Effect::RepeatEach { .. }
            // ExtraLandPlay targets the spell controller — no permanent target needed.
            | Effect::ExtraLandPlay { .. }
            // TapPermanentsMatchingFilter uses a filter; no cast-time permanent target needed.
            | Effect::TapPermanentsMatchingFilter { .. }
            // ChooseAndRememberOneOfEach reads from remembered_players; no cast-time target.
            | Effect::ChooseAndRememberOneOfEach { .. }
            // GrantCastWithFlash grants the controller flash permission — no permanent target needed.
            | Effect::GrantCastWithFlash { .. } => true, // Filter-based / no-target effects

            // ===== EXHAUSTIVE EFFECT HANDLING =====
            // Effects with pre-specified targets (guard failed: target.as_u32() != 0)
            // These already have targets, so they "have valid targets"
            Effect::DealDamage { .. }
            | Effect::DealDamageXPaid { .. }
            | Effect::DealDamageDivided { .. }
            | Effect::DealDamageDynamic { .. } => true, // TargetRef::Player/Permanent already specified
            Effect::DestroyPermanent { .. }
            | Effect::ReturnPermanentToHand { .. }
            | Effect::PumpCreature { .. }
            | Effect::DebuffCreature { .. }
            | Effect::PumpCreatureVariable { .. }
            | Effect::TapPermanent { .. }
            | Effect::UntapPermanent { .. }
            | Effect::TapOrUntapPermanent { .. }
            | Effect::CounterSpell { .. }
            | Effect::ExilePermanent { .. }
            | Effect::Airbend { .. }
            | Effect::GrantCantBeBlocked { .. }
            | Effect::Regenerate { .. }
            | Effect::PreventDamage { .. }
            | Effect::PreventDamageFromSource { .. }
            | Effect::RemoveCounter { .. }
            | Effect::PutCounter { .. }
            | Effect::MultiplyCounter { .. }
            | Effect::Earthbend { .. }
            | Effect::CopyPermanent { .. }
            // PlayFromGraveyard is an activated-ability effect; valid-target check
            // is done in get_valid_targets_for_ability. When it appears on a spell
            // the target has already been pre-specified, so always return true here.
            | Effect::PlayFromGraveyard { .. } => {
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
    /// Apply selected mode indices to a modal spell, replacing the `ModalChoice` effect with
    /// the selected mode effects. Returns `(found_modal, total_mode_cost)` where
    /// `total_mode_cost` is the sum of `ModeCost$` values for all selected modes (used by the
    /// caller to record via `set_mode_cost_paid_logged` for undo-safe tracking).
    pub fn apply_selected_modes(
        &mut self,
        spell_card_id: CardId,
        selected_mode_indices: &[usize],
    ) -> Result<(bool, u8)> {
        let spell_card = self.cards.get_mut(spell_card_id)?;

        // Find the ModalChoice effect and get selected mode effects
        let mut new_effects = Vec::new();
        let mut found_modal = false;
        // Sum of ModeCost$ values for all selected modes (only one mode for tiered spells)
        let mut total_mode_cost: u8 = 0;

        for effect in spell_card.effects.drain(..) {
            if let Effect::ModalChoice { modes, .. } = effect {
                found_modal = true;
                // Add effects from selected modes in order
                for &mode_idx in selected_mode_indices {
                    if let Some(mode) = modes.get(mode_idx) {
                        new_effects.push((*mode.effect).clone());
                        total_mode_cost = total_mode_cost.saturating_add(mode.mode_cost);
                    }
                }
            } else {
                // Keep non-modal effects as-is
                new_effects.push(effect);
            }
        }

        spell_card.effects = new_effects;
        Ok((found_modal, total_mode_cost))
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
        sinkhole_card.definition.cache = crate::core::CardCache::new(&sinkhole_card.text, sinkhole_card.name.as_str());
        sinkhole_card.effects.push(Effect::DestroyPermanent {
            target: CardId::new(0),
            restriction: crate::core::TargetRestriction::any(),
            no_regenerate: false,
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
        terror_card.definition.cache = crate::core::CardCache::new(&terror_card.text, terror_card.name.as_str());
        terror_card.effects.push(Effect::DestroyPermanent {
            target: CardId::new(0),
            restriction: crate::core::TargetRestriction::any(),
            no_regenerate: false,
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
    /// This verifies the fix for mtg-239
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
            "Animate Dead should NOT target creatures on battlefield (this was the bug in mtg-239!)"
        );
    }

    /// Test that AbilityCache correctly parses "target opponent" vs "target player"
    #[test]
    fn test_ability_cache_targets_opponent_vs_player() {
        // Sorin Markov -3: targets only opponents
        let cache_sorin = crate::core::AbilityCache::new("Target opponent's life total becomes 10.");
        assert!(
            cache_sorin.targets_opponent,
            "Sorin -3 should have targets_opponent=true"
        );
        assert!(
            !cache_sorin.targets_player,
            "Sorin -3 should NOT have targets_player=true (exclusive)"
        );
        assert!(cache_sorin.requires_target, "Sorin -3 should require a target");

        // Ancestral Recall-like ability targeting any player
        let cache_player = crate::core::AbilityCache::new("Target player draws three cards.");
        assert!(
            !cache_player.targets_opponent,
            "Target-player ability should NOT have targets_opponent=true"
        );
        assert!(
            cache_player.targets_player,
            "Target-player ability should have targets_player=true"
        );
        assert!(
            cache_player.requires_target,
            "Target-player ability should require a target"
        );
    }

    /// Test that Sorin Markov's -3 ability enumerates opponents as valid targets (mtg-914 fix)
    ///
    /// The -3 ability is "Target opponent's life total becomes 10."
    /// Before the fix, SetLife was in the no-target catch-all of get_valid_targets_for_ability,
    /// so the ability was never shown (empty targets → can_activate=false).
    #[test]
    fn test_sorin_markov_minus3_targets_opponent_only() {
        use crate::core::{ActivatedAbility, CardType, Effect, PlayerId};
        use crate::game::state::GameState;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let player1 = PlayerId::new(0);
        let player2 = PlayerId::new(1);

        // Create a Sorin Markov planeswalker card controlled by player1
        let sorin_id = game.cards.next_id();
        let mut sorin_card = crate::core::Card::new(sorin_id, "Sorin Markov", player1);
        sorin_card.add_type(CardType::Planeswalker);
        sorin_card.controller = player1;

        // Add the -3 ability: SetLife targeting an opponent
        // player=PlayerId::new(0) is the placeholder; the description drives targeting
        let minus3_effect = Effect::SetLife {
            player: PlayerId::new(0), // placeholder
            amount: 10,
        };
        let minus3_ability = ActivatedAbility::new(
            Cost::SubLoyalty { amount: 3 },
            vec![minus3_effect],
            "Target opponent's life total becomes 10.".to_string(),
            false,
        );
        sorin_card.activated_abilities.push(minus3_ability);
        game.cards.insert(sorin_id, sorin_card);
        game.battlefield.cards.push(sorin_id);

        // Get valid targets for the -3 ability (index 0)
        let targets = game.get_valid_targets_for_ability(sorin_id, 0).unwrap();

        // Should include player2 (opponent) as a sentinel
        let p2_sentinel = crate::core::player_as_target_sentinel(player2);
        assert!(
            targets.contains(&p2_sentinel),
            "Sorin -3 should target the opponent (player2); got {:?}",
            targets
        );

        // Should NOT include player1 (the controller) — ability says "opponent"
        let p1_sentinel = crate::core::player_as_target_sentinel(player1);
        assert!(
            !targets.contains(&p1_sentinel),
            "Sorin -3 should NOT target player1 (the controller)"
        );
    }
}
