//! Game actions and mechanics

use crate::core::{CardId, CardType, Cost, Effect, Keyword, PlayerId, TargetRef, TriggerEvent};
use crate::game::GameState;
use crate::zones::Zone;
use crate::{MtgError, Result};
use smallvec::SmallVec;

/// Types of game actions
#[derive(Debug, Clone)]
pub enum GameAction {
    /// Play a land from hand
    PlayLand { player_id: PlayerId, card_id: CardId },

    /// Cast a spell from hand
    CastSpell {
        player_id: PlayerId,
        card_id: CardId,
        targets: Vec<CardId>,
    },

    /// Deal damage to a target
    DealDamage {
        source: CardId,
        target: CardId,
        amount: i32,
    },

    /// Tap a permanent for mana
    TapForMana { player_id: PlayerId, card_id: CardId },

    /// Declare attackers
    DeclareAttackers {
        player_id: PlayerId,
        attackers: Vec<CardId>,
    },

    /// Pass priority
    PassPriority { player_id: PlayerId },
}

impl GameState {
    /// Play a land from hand to battlefield
    pub fn play_land(&mut self, player_id: PlayerId, card_id: CardId) -> Result<()> {
        // Check if player can play a land
        let player = self.get_player(player_id)?;
        if !player.can_play_land() {
            return Err(MtgError::InvalidAction("Cannot play more lands this turn".to_string()));
        }

        // Check if card is a land and in hand
        let card = self.cards.get(card_id)?;
        if !card.is_land() {
            return Err(MtgError::InvalidAction("Card is not a land".to_string()));
        }

        // Check if in hand
        if let Some(zones) = self.get_player_zones(player_id) {
            if !zones.hand.contains(card_id) {
                return Err(MtgError::InvalidAction("Card not in hand".to_string()));
            }
        }

        // Move card to battlefield
        self.move_card(card_id, Zone::Hand, Zone::Battlefield, player_id)?;

        // Record the turn number when this land entered the battlefield
        if let Ok(card) = self.cards.get_mut(card_id) {
            card.turn_entered_battlefield = Some(self.turn.turn_number);
        }

        // Increment lands played
        let player = self.get_player_mut(player_id)?;
        player.play_land();

        Ok(())
    }

    /// Cast a spell (put it on the stack)
    ///
    /// This validates mana payment and deducts the cost from the player's mana pool.
    pub fn cast_spell(&mut self, player_id: PlayerId, card_id: CardId, _targets: Vec<CardId>) -> Result<()> {
        // Check if card is in hand
        if let Some(zones) = self.get_player_zones(player_id) {
            if !zones.hand.contains(card_id) {
                return Err(MtgError::InvalidAction("Card not in hand".to_string()));
            }
        }

        // Get the mana cost (need to do this before mutable borrow)
        let mana_cost = {
            let card = self.cards.get(card_id)?;
            card.mana_cost
        };

        // Pay the mana cost
        let player = self.get_player_mut(player_id)?;
        player.mana_pool.pay_cost(&mana_cost).map_err(MtgError::InvalidAction)?;

        // Move card to stack
        self.move_card(card_id, Zone::Hand, Zone::Stack, player_id)?;

        Ok(())
    }

    /// Resolve a spell from the stack
    ///
    /// ## Parameters
    /// - `card_id`: The spell card on the stack to resolve
    /// - `chosen_targets`: Targets selected by the controller during casting (optional)
    ///
    /// If targets are provided, they will be used to fill in placeholder targets in effects.
    /// Otherwise, effects must already have their targets specified.
    pub fn resolve_spell(&mut self, card_id: CardId, chosen_targets: &[CardId]) -> Result<()> {
        // Get card owner and effects
        let (card_owner, mut effects) = {
            let card = self.cards.get(card_id)?;
            // TODO: eliminate this clone and instead just take a reference. Why does it need to be mutable?
            (card.owner, card.effects.clone())
        };

        // Fill in targets for effects using the chosen targets
        // If no targets were chosen (empty slice), effects must already be fully specified
        let mut target_index = 0;
        for effect in &mut effects {
            match effect {
                Effect::DealDamage {
                    target: TargetRef::None,
                    amount,
                } if target_index < chosen_targets.len() => {
                    // Use the chosen target
                    *effect = Effect::DealDamage {
                        target: TargetRef::Permanent(chosen_targets[target_index]),
                        amount: *amount,
                    };
                    target_index += 1;
                }
                Effect::DestroyPermanent { target } if target.as_u32() == 0 && target_index < chosen_targets.len() => {
                    // Use the chosen target
                    *effect = Effect::DestroyPermanent {
                        target: chosen_targets[target_index],
                    };
                    target_index += 1;
                }
                Effect::PumpCreature {
                    target,
                    power_bonus,
                    toughness_bonus,
                } if target.as_u32() == 0 && target_index < chosen_targets.len() => {
                    // Use the chosen target
                    *effect = Effect::PumpCreature {
                        target: chosen_targets[target_index],
                        power_bonus: *power_bonus,
                        toughness_bonus: *toughness_bonus,
                    };
                    target_index += 1;
                }
                Effect::TapPermanent { target } if target.as_u32() == 0 && target_index < chosen_targets.len() => {
                    // Use the chosen target
                    *effect = Effect::TapPermanent {
                        target: chosen_targets[target_index],
                    };
                    target_index += 1;
                }
                Effect::UntapPermanent { target } if target.as_u32() == 0 && target_index < chosen_targets.len() => {
                    // Use the chosen target
                    *effect = Effect::UntapPermanent {
                        target: chosen_targets[target_index],
                    };
                    target_index += 1;
                }
                Effect::CounterSpell { target } if target.as_u32() == 0 && target_index < chosen_targets.len() => {
                    // Use the chosen target
                    *effect = Effect::CounterSpell {
                        target: chosen_targets[target_index],
                    };
                    target_index += 1;
                }
                Effect::ExilePermanent { target } if target.as_u32() == 0 && target_index < chosen_targets.len() => {
                    // Use the chosen target
                    *effect = Effect::ExilePermanent {
                        target: chosen_targets[target_index],
                    };
                    target_index += 1;
                }
                _ => {
                    // Effect doesn't need a target, or target is already specified
                }
            }
        }

        // Handle placeholder player IDs (0 means "controller")
        // This is still needed for effects that don't require targeting, like:
        // "Draw a card" or "You gain 3 life"
        for effect in &mut effects {
            match effect {
                Effect::DrawCards { player, count } if player.as_u32() == 0 => {
                    // Placeholder player ID 0 means "controller"
                    *effect = Effect::DrawCards {
                        player: card_owner,
                        count: *count,
                    };
                }
                Effect::GainLife { player, amount } if player.as_u32() == 0 => {
                    // Placeholder player ID 0 means "controller"
                    *effect = Effect::GainLife {
                        player: card_owner,
                        amount: *amount,
                    };
                }
                Effect::Mill { player, count } if player.as_u32() == 0 => {
                    // Placeholder player ID 0 means "controller"
                    *effect = Effect::Mill {
                        player: card_owner,
                        count: *count,
                    };
                }
                Effect::DealDamage {
                    target: TargetRef::None,
                    amount,
                } => {
                    // If no target was chosen, default to opponent for damage
                    // This handles untargeted damage like "deals 1 damage to each opponent"
                    if let Some(opponent_id) = self.players.iter().map(|p| p.id).find(|id| *id != card_owner) {
                        *effect = Effect::DealDamage {
                            target: TargetRef::Player(opponent_id),
                            amount: *amount,
                        };
                    }
                }
                _ => {}
            }
        }

        // Check if targets are still valid before executing effects
        // MTG Rules 608.2b: If all targets are illegal, the spell doesn't resolve
        //
        // For damage spells targeting permanents, verify targets are still on battlefield
        // (they might have been removed by another spell resolving first)
        let mut all_targets_illegal = false;
        if !chosen_targets.is_empty() {
            // Check if this spell deals damage to permanents
            let targets_permanents = effects.iter().any(|effect| {
                matches!(
                    effect,
                    Effect::DealDamage {
                        target: TargetRef::Permanent(_),
                        ..
                    }
                )
            });

            if targets_permanents {
                // Check if any permanent target is no longer on the battlefield
                let any_target_gone = chosen_targets
                    .iter()
                    .any(|&target_id| !self.battlefield.contains(target_id));
                if any_target_gone {
                    // Spell fizzles - permanent targets are no longer valid
                    all_targets_illegal = true;
                }
            }
        }

        // Execute effects only if targets are still valid
        if !all_targets_illegal {
            for effect in effects {
                self.execute_effect(&effect)?;
            }
        }

        // Determine destination based on card type
        let destination = {
            let card = self.cards.get(card_id)?;
            if card.is_type(&CardType::Instant) || card.is_type(&CardType::Sorcery) {
                Zone::Graveyard
            } else {
                Zone::Battlefield
            }
        };

        // Move card from stack to destination
        let owner = self.cards.get(card_id)?.owner;
        self.move_card(card_id, Zone::Stack, destination, owner)?;

        // If it entered the battlefield, record the turn number (for summoning sickness)
        if destination == Zone::Battlefield {
            if let Ok(card) = self.cards.get_mut(card_id) {
                card.turn_entered_battlefield = Some(self.turn.turn_number);
            }

            // Check for ETB triggers on all permanents (including the one that just entered)
            self.check_triggers(TriggerEvent::EntersBattlefield, card_id)?;
        }

        Ok(())
    }

    /// Get valid targets for a spell's effects
    ///
    /// This function filters game entities to find valid targets based on:
    /// - Effect type (damage, destroy, tap, etc.)
    /// - Targeting restrictions (hexproof, shroud, protection)
    /// - Controller ownership (can't target opponent's hexproof permanents)
    /// - Zone requirements (battlefield, stack, etc.)
    ///
    /// Returns a vector of valid target CardIds that can be chosen by the controller.
    /// For effects that target players, use TargetRef::Player instead.
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
                            if card.is_creature() && !card.has_shroud() {
                                // Hexproof only protects from opponent's spells
                                if card.owner == spell_owner || !card.has_hexproof() {
                                    valid_targets.push(card_id);
                                }
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
                            if !card.has_shroud() {
                                // Hexproof only protects from opponent's spells
                                if card.owner == spell_owner || !card.has_hexproof() {
                                    valid_targets.push(card_id);
                                }
                            }
                        }
                    }
                }
                Effect::PumpCreature { target, .. } if target.as_u32() == 0 => {
                    // Pump can target any creature
                    for &card_id in &self.battlefield.cards {
                        if let Ok(card) = self.cards.get(card_id) {
                            if card.is_creature() && !card.has_shroud() {
                                // Hexproof only protects from opponent's spells
                                if card.owner == spell_owner || !card.has_hexproof() {
                                    valid_targets.push(card_id);
                                }
                            }
                        }
                    }
                }
                Effect::TapPermanent { target } if target.as_u32() == 0 => {
                    // Tap can target untapped permanents
                    for &card_id in &self.battlefield.cards {
                        if let Ok(card) = self.cards.get(card_id) {
                            if !card.tapped && !card.has_shroud() {
                                // Hexproof only protects from opponent's spells
                                if card.owner == spell_owner || !card.has_hexproof() {
                                    valid_targets.push(card_id);
                                }
                            }
                        }
                    }
                }
                Effect::UntapPermanent { target } if target.as_u32() == 0 => {
                    // Untap can target tapped permanents
                    for &card_id in &self.battlefield.cards {
                        if let Ok(card) = self.cards.get(card_id) {
                            if card.tapped && !card.has_shroud() {
                                // Hexproof only protects from opponent's spells
                                if card.owner == spell_owner || !card.has_hexproof() {
                                    valid_targets.push(card_id);
                                }
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
                            if card.is_creature() && !card.has_shroud() {
                                // Hexproof only protects from opponent's spells
                                if card.owner == spell_owner || !card.has_hexproof() {
                                    valid_targets.push(card_id);
                                }
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
    /// A SmallVec of valid target CardIds
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
        let requires_tapped = ability.description.to_lowercase().contains("tapped");
        let requires_untapped = ability.description.to_lowercase().contains("untapped");
        let targets_creature = ability.description.to_lowercase().contains("creature");
        let targets_land = ability.description.to_lowercase().contains("land");

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

                            // Check shroud/hexproof
                            if card.has_shroud() {
                                is_valid = false;
                            }

                            // Hexproof only protects from opponent's abilities
                            if card.has_hexproof() && card.owner != ability_controller {
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
                            let mut is_valid = !card.tapped && !card.has_shroud();

                            // Check hexproof
                            if card.has_hexproof() && card.owner != ability_controller {
                                is_valid = false;
                            }

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
                            let mut is_valid = card.tapped && !card.has_shroud();

                            // Check hexproof
                            if card.has_hexproof() && card.owner != ability_controller {
                                is_valid = false;
                            }

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
                            let mut is_valid = card.is_creature() && !card.has_shroud();

                            // Check hexproof
                            if card.has_hexproof() && card.owner != ability_controller {
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

    /// Cast a spell following the full 8-step process (MTG Rules 601.2)
    ///
    /// This method implements the complete spell casting sequence:
    /// 1. Propose the spell (move to stack)
    /// 2. Make choices (modes, X values) - TODO
    /// 3. Choose targets
    /// 4. Divide effects - TODO
    /// 5. Determine total cost
    /// 6. Activate mana abilities (tap sources for mana)
    /// 7. Pay costs
    /// 8. Spell becomes cast (trigger abilities) - TODO
    ///
    /// ## Parameters
    /// - `player_id`: The player casting the spell
    /// - `card_id`: The spell card to cast
    /// - `choose_targets_fn`: Callback to choose targets (step 3)
    /// - `choose_mana_sources_fn`: Callback to choose what to tap for mana (step 6)
    ///
    /// ## Java Forge Equivalent
    /// This matches `ComputerUtil.handlePlayingSpellAbility()` which:
    /// 1. Moves spell to stack (line 99)
    /// 2. Handles targeting
    /// 3. Pays costs with `CostPayment.payComputerCosts()` (line 125)
    pub fn cast_spell_8_step<TargetFn, ManaFn>(
        &mut self,
        player_id: PlayerId,
        card_id: CardId,
        mut choose_targets_fn: TargetFn,
        mut choose_mana_sources_fn: ManaFn,
    ) -> Result<()>
    where
        TargetFn: FnMut(&GameState, CardId) -> Vec<CardId>,
        ManaFn: FnMut(&GameState, &crate::core::ManaCost) -> Vec<CardId>,
    {
        // Verify card is in hand
        if let Some(zones) = self.get_player_zones(player_id) {
            if !zones.hand.contains(card_id) {
                return Err(MtgError::InvalidAction("Card not in hand".to_string()));
            }
        }

        // Step 1: Propose the spell - move card to stack
        // This happens BEFORE paying costs (unlike our old implementation)
        self.move_card(card_id, Zone::Hand, Zone::Stack, player_id)?;

        // Step 2: Make choices (modes, X values)
        // TODO: Implement modal spell choices and X value selection

        // Step 3: Choose targets
        let _targets = choose_targets_fn(self, card_id);
        // TODO: Store targets on the spell for resolution
        // For now, we'll use them to update effects immediately (simplified)

        // Step 4: Divide effects
        // TODO: Implement dividing damage/counters among targets

        // Step 5: Determine total cost
        let mana_cost = {
            let card = self.cards.get(card_id)?;
            card.mana_cost
        };

        // Step 6: Activate mana abilities
        // This is where mana gets tapped - AFTER the spell is on the stack
        let sources_to_tap = choose_mana_sources_fn(self, &mana_cost);
        for &source_id in &sources_to_tap {
            self.tap_for_mana_for_cost(player_id, source_id, &mana_cost)?;
        }

        // Step 7: Pay costs
        let player = self.get_player_mut(player_id)?;
        if let Err(e) = player.mana_pool.pay_cost(&mana_cost) {
            // If we can't pay, we need to unwind:
            // 1. Move card back to hand from stack
            // 2. Untap all mana sources that were tapped
            // 3. Clear the mana pool

            // Move card back to hand
            self.move_card(card_id, Zone::Stack, Zone::Hand, player_id)?;

            // Untap all sources that were tapped
            for &source_id in &sources_to_tap {
                if let Ok(card) = self.cards.get_mut(source_id) {
                    card.untap();
                    // Log the untap for undo functionality
                    self.undo_log.log(crate::undo::GameAction::TapCard {
                        card_id: source_id,
                        tapped: false,
                    });
                }
            }

            // Clear the mana pool (remove any mana that was added)
            let player = self.get_player_mut(player_id)?;
            player.mana_pool.clear();

            return Err(MtgError::InvalidAction(format!("Failed to pay mana cost: {e}")));
        }

        // Step 8: Spell becomes cast
        // TODO: Trigger "whenever you cast a spell" abilities
        // For now, this is complete - spell is on stack and costs are paid

        Ok(())
    }

    /// Execute a single effect
    pub fn execute_effect(&mut self, effect: &Effect) -> Result<()> {
        match effect {
            Effect::DealDamage { target, amount } => match target {
                TargetRef::Player(player_id) => {
                    self.deal_damage(*player_id, *amount)?;
                }
                TargetRef::Permanent(card_id) => {
                    self.deal_damage_to_creature(*card_id, *amount)?;
                }
                TargetRef::None => {
                    return Err(MtgError::InvalidAction(
                        "DealDamage effect requires a target".to_string(),
                    ));
                }
            },
            Effect::DrawCards { player, count } => {
                for _ in 0..*count {
                    self.draw_card(*player)?;
                }
            }
            Effect::GainLife { player, amount } => {
                let p = self.get_player_mut(*player)?;
                p.gain_life(*amount);

                // Log the life gain
                self.undo_log.log(crate::undo::GameAction::ModifyLife {
                    player_id: *player,
                    delta: *amount,
                });
            }
            Effect::DestroyPermanent { target } => {
                // Skip if target is still placeholder (0) - no valid targets found
                if target.as_u32() == 0 {
                    // Spell fizzles - no valid targets
                    return Ok(());
                }
                // MTG Rules 702.12b: Permanents with indestructible can't be destroyed
                let (owner, has_indestructible) = {
                    let card = self.cards.get(*target)?;
                    (card.owner, card.has_indestructible())
                };
                if !has_indestructible {
                    self.move_card(*target, Zone::Battlefield, Zone::Graveyard, owner)?;
                }
            }
            Effect::TapPermanent { target } => {
                // Skip if target is still placeholder (0) - no valid targets found
                if target.as_u32() == 0 {
                    // Spell fizzles - no valid targets
                    return Ok(());
                }
                let card = self.cards.get_mut(*target)?;
                card.tap();

                // Log the tap
                self.undo_log.log(crate::undo::GameAction::TapCard {
                    card_id: *target,
                    tapped: true,
                });
            }
            Effect::UntapPermanent { target } => {
                let card = self.cards.get_mut(*target)?;
                card.untap();

                // Log the untap
                self.undo_log.log(crate::undo::GameAction::TapCard {
                    card_id: *target,
                    tapped: false,
                });
            }
            Effect::PumpCreature {
                target,
                power_bonus,
                toughness_bonus,
            } => {
                // Skip if target is still placeholder (0) - no valid targets found
                if target.as_u32() == 0 {
                    // Spell fizzles - no valid targets
                    return Ok(());
                }
                let card = self.cards.get_mut(*target)?;
                card.power_bonus += power_bonus;
                card.toughness_bonus += toughness_bonus;

                // Log the pump effect
                self.undo_log.log(crate::undo::GameAction::PumpCreature {
                    card_id: *target,
                    power_delta: *power_bonus,
                    toughness_delta: *toughness_bonus,
                });
            }
            Effect::Mill { player, count } => {
                // Mill cards from library to graveyard
                self.mill_cards(*player, *count)?;
            }
            Effect::CounterSpell { target } => {
                // Counter a spell on the stack
                self.counter_spell(*target)?;
            }
            Effect::AddMana { player, mana } => {
                // Add mana to player's mana pool
                let p = self.get_player_mut(*player)?;

                // Add each component of the mana cost to the pool
                for _ in 0..mana.white {
                    p.mana_pool.add_color(crate::core::Color::White);
                }
                for _ in 0..mana.blue {
                    p.mana_pool.add_color(crate::core::Color::Blue);
                }
                for _ in 0..mana.black {
                    p.mana_pool.add_color(crate::core::Color::Black);
                }
                for _ in 0..mana.red {
                    p.mana_pool.add_color(crate::core::Color::Red);
                }
                for _ in 0..mana.green {
                    p.mana_pool.add_color(crate::core::Color::Green);
                }
                for _ in 0..mana.colorless {
                    p.mana_pool.add_color(crate::core::Color::Colorless);
                }

                // Log the mana addition
                self.undo_log.log(crate::undo::GameAction::AddMana {
                    player_id: *player,
                    mana: *mana,
                });
            }
            Effect::PutCounter {
                target,
                counter_type,
                amount,
            } => {
                // Skip if target is still placeholder (0) - no valid targets found
                if target.as_u32() == 0 {
                    // Spell fizzles - no valid targets
                    return Ok(());
                }
                // Add counters using the GameState method (which logs for undo)
                self.add_counters(*target, *counter_type, *amount)?;
            }
            Effect::RemoveCounter {
                target,
                counter_type,
                amount,
            } => {
                // Skip if target is still placeholder (0) - no valid targets found
                if target.as_u32() == 0 {
                    // Spell fizzles - no valid targets
                    return Ok(());
                }
                // Remove counters using the GameState method (which logs for undo)
                self.remove_counters(*target, *counter_type, *amount)?;
            }
            Effect::ExilePermanent { target } => {
                // Skip if target is still placeholder (0) - no valid targets found
                if target.as_u32() == 0 {
                    // Spell fizzles - no valid targets
                    return Ok(());
                }
                // Exile the permanent by moving it from battlefield to exile
                let owner = self.cards.get(*target)?.owner;
                self.move_card(*target, Zone::Battlefield, Zone::Exile, owner)?;
            }
        }
        Ok(())
    }

    /// Check for triggered abilities and execute them
    ///
    /// This checks all permanents on the battlefield for triggers matching the given event.
    /// When triggers are found, their effects are executed immediately (for now).
    ///
    /// TODO: In full MTG rules, triggers should go on the stack and wait for priority,
    /// but for simplicity we're executing them immediately.
    pub fn check_triggers(&mut self, event: TriggerEvent, source_card_id: CardId) -> Result<()> {
        // Collect all triggered effects to execute (without holding a borrow on self.cards)
        let triggered_effects: Vec<(CardId, Vec<Effect>)> = self
            .battlefield
            .cards
            .iter()
            .filter_map(|&card_id| {
                if let Ok(card) = self.cards.get(card_id) {
                    // Find triggers matching this event
                    let matching_triggers: Vec<Effect> = card
                        .triggers
                        .iter()
                        .filter(|trigger| trigger.event == event)
                        .flat_map(|trigger| trigger.effects.clone())
                        .collect();

                    if !matching_triggers.is_empty() {
                        Some((card_id, matching_triggers))
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect();

        // Execute all triggered effects
        for (_trigger_source, effects) in triggered_effects {
            for mut effect in effects {
                // Fill in placeholder values in trigger effects
                // Similar to resolve_spell, we need to fill in targets
                match &mut effect {
                    Effect::DrawCards { player, .. } if player.as_u32() == 0 => {
                        // Placeholder player ID 0 means the controller of the trigger source
                        let controller = self.cards.get(source_card_id)?.controller;
                        if let Effect::DrawCards { player: _, count } = effect {
                            effect = Effect::DrawCards {
                                player: controller,
                                count,
                            };
                        }
                    }
                    Effect::DealDamage {
                        target: TargetRef::None,
                        amount,
                    } => {
                        // Find a valid target (opponent's creature)
                        let controller = self.cards.get(source_card_id)?.controller;
                        if let Some(target_id) = self
                            .battlefield
                            .cards
                            .iter()
                            .find(|&card_id| {
                                if let Ok(card) = self.cards.get(*card_id) {
                                    card.is_creature()
                                        && card.owner != controller
                                        && !card.has_hexproof()
                                        && !card.has_shroud()
                                } else {
                                    false
                                }
                            })
                            .copied()
                        {
                            effect = Effect::DealDamage {
                                target: TargetRef::Permanent(target_id),
                                amount: *amount,
                            };
                        }
                    }
                    Effect::GainLife { player, amount } if player.as_u32() == 0 => {
                        // Placeholder player ID 0 means the controller of the trigger source
                        let controller = self.cards.get(source_card_id)?.controller;
                        effect = Effect::GainLife {
                            player: controller,
                            amount: *amount,
                        };
                    }
                    Effect::DestroyPermanent { target } if target.as_u32() == 0 => {
                        // Find a valid target (opponent's creature)
                        let controller = self.cards.get(source_card_id)?.controller;
                        if let Some(target_id) = self
                            .battlefield
                            .cards
                            .iter()
                            .find(|&card_id| {
                                if let Ok(card) = self.cards.get(*card_id) {
                                    card.is_creature()
                                        && card.owner != controller
                                        && !card.has_hexproof()
                                        && !card.has_shroud()
                                } else {
                                    false
                                }
                            })
                            .copied()
                        {
                            effect = Effect::DestroyPermanent { target: target_id };
                        }
                    }
                    Effect::PumpCreature {
                        target,
                        power_bonus,
                        toughness_bonus,
                    } if target.as_u32() == 0 => {
                        // Find a valid target (any creature on battlefield)
                        if let Some(target_id) = self
                            .battlefield
                            .cards
                            .iter()
                            .find(|&card_id| {
                                if let Ok(card) = self.cards.get(*card_id) {
                                    card.is_creature() && !card.has_shroud()
                                } else {
                                    false
                                }
                            })
                            .copied()
                        {
                            effect = Effect::PumpCreature {
                                target: target_id,
                                power_bonus: *power_bonus,
                                toughness_bonus: *toughness_bonus,
                            };
                        }
                    }
                    _ => {}
                }

                self.execute_effect(&effect)?;
            }
        }

        Ok(())
    }

    /// Deal damage to a player target
    pub fn deal_damage(&mut self, target_id: PlayerId, amount: i32) -> Result<()> {
        // Check if target is a player
        if self.players.iter().any(|p| p.id == target_id) {
            let player = self.get_player_mut(target_id)?;
            player.lose_life(amount);

            // Log the life change
            self.undo_log.log(crate::undo::GameAction::ModifyLife {
                player_id: target_id,
                delta: -amount,
            });

            return Ok(());
        }

        Err(MtgError::InvalidAction("Invalid damage target".to_string()))
    }

    /// Deal damage to a creature
    pub fn deal_damage_to_creature(&mut self, target_id: CardId, amount: i32) -> Result<()> {
        // Get info about the creature first (without holding the borrow)
        let (is_creature, toughness, owner, has_indestructible) = {
            let card = self.cards.get(target_id)?;
            (
                card.is_creature(),
                card.current_toughness(),
                card.owner,
                card.has_indestructible(),
            )
        };

        if is_creature {
            // Mark damage (simplified - real MTG has damage tracking)
            // MTG Rules 702.12b: Permanents with indestructible aren't destroyed by lethal damage
            // For now, if damage >= toughness and creature doesn't have indestructible, creature dies
            if amount >= toughness as i32 && !has_indestructible {
                self.move_card(target_id, Zone::Battlefield, Zone::Graveyard, owner)?;
            }
            return Ok(());
        }

        Err(MtgError::InvalidAction("Invalid damage target".to_string()))
    }

    /// Tap a land for mana (without cost hint)
    pub fn tap_for_mana(&mut self, player_id: PlayerId, card_id: CardId) -> Result<()> {
        // Create an empty cost hint
        let empty_cost = crate::core::ManaCost::new();
        self.tap_for_mana_for_cost(player_id, card_id, &empty_cost)
    }

    /// Tap a land for mana with a cost hint to guide color production for any-color lands
    pub fn tap_for_mana_for_cost(
        &mut self,
        player_id: PlayerId,
        card_id: CardId,
        cost_hint: &crate::core::ManaCost,
    ) -> Result<()> {
        let card = self.cards.get_mut(card_id)?;

        // Check if card is a land and untapped
        if !card.is_land() {
            return Err(MtgError::InvalidAction("Card is not a land".to_string()));
        }

        if card.tapped {
            return Err(MtgError::InvalidAction("Land is already tapped".to_string()));
        }

        // Get land name before tapping (to avoid borrow conflicts)
        let land_name = card.name.to_lowercase();

        // Tap the land
        card.tap();

        // Log the tap
        self.undo_log
            .log(crate::undo::GameAction::TapCard { card_id, tapped: true });

        // Add mana to player's pool based on land type
        // For basic lands and simple cases, check name
        // For dual lands (e.g., Underground Sea = Island Swamp), we need smarter logic
        // First, check subtypes and any-color before we borrow player_mut
        let (
            has_swamp_subtype,
            has_mountain_subtype,
            has_island_subtype,
            has_forest_subtype,
            has_plains_subtype,
            is_any_color_land,
        ) = {
            let card = self.cards.get(card_id)?;
            (
                card.subtypes.iter().any(|s| s.as_str().eq_ignore_ascii_case("swamp")),
                card.subtypes
                    .iter()
                    .any(|s| s.as_str().eq_ignore_ascii_case("mountain")),
                card.subtypes.iter().any(|s| s.as_str().eq_ignore_ascii_case("island")),
                card.subtypes.iter().any(|s| s.as_str().eq_ignore_ascii_case("forest")),
                card.subtypes.iter().any(|s| s.as_str().eq_ignore_ascii_case("plains")),
                card.text.to_lowercase().contains("any color"),
            )
        };

        let player = self.get_player_mut(player_id)?;

        // Determine what colors this land can produce
        let mut available_colors = Vec::new();
        if has_plains_subtype || land_name.contains("plains") {
            available_colors.push(crate::core::Color::White);
        }
        if has_island_subtype || land_name.contains("island") {
            available_colors.push(crate::core::Color::Blue);
        }
        if has_swamp_subtype || land_name.contains("swamp") {
            available_colors.push(crate::core::Color::Black);
        }
        if has_mountain_subtype || land_name.contains("mountain") {
            available_colors.push(crate::core::Color::Red);
        }
        if has_forest_subtype || land_name.contains("forest") {
            available_colors.push(crate::core::Color::Green);
        }

        let color = if is_any_color_land || available_colors.len() > 1 {
            // Multi-color or any-color land: choose based on cost hint
            // Produce the first color needed by the cost that this land can produce
            if cost_hint.white > 0 && (is_any_color_land || available_colors.contains(&crate::core::Color::White)) {
                Some(crate::core::Color::White)
            } else if cost_hint.blue > 0 && (is_any_color_land || available_colors.contains(&crate::core::Color::Blue))
            {
                Some(crate::core::Color::Blue)
            } else if cost_hint.black > 0
                && (is_any_color_land || available_colors.contains(&crate::core::Color::Black))
            {
                Some(crate::core::Color::Black)
            } else if cost_hint.red > 0 && (is_any_color_land || available_colors.contains(&crate::core::Color::Red)) {
                Some(crate::core::Color::Red)
            } else if cost_hint.green > 0
                && (is_any_color_land || available_colors.contains(&crate::core::Color::Green))
            {
                Some(crate::core::Color::Green)
            } else {
                // Cost doesn't need a specific color - produce the first available color
                available_colors.first().copied().or(Some(crate::core::Color::White))
            }
        } else if available_colors.len() == 1 {
            // Single-color land
            available_colors.first().copied()
        } else {
            // Unknown land type
            None
        };

        if let Some(color) = color {
            player.mana_pool.add_color(color);

            // Log the mana addition
            let mut mana = crate::core::ManaCost::new();
            let color_symbol = match color {
                crate::core::Color::White => {
                    mana.white = 1;
                    "W"
                }
                crate::core::Color::Blue => {
                    mana.blue = 1;
                    "U"
                }
                crate::core::Color::Black => {
                    mana.black = 1;
                    "B"
                }
                crate::core::Color::Red => {
                    mana.red = 1;
                    "R"
                }
                crate::core::Color::Green => {
                    mana.green = 1;
                    "G"
                }
                crate::core::Color::Colorless => {
                    mana.colorless = 1;
                    "C"
                }
            };
            self.undo_log.log(crate::undo::GameAction::AddMana { player_id, mana });

            // Log visible message for mana tapping
            if self.logger.verbosity() >= crate::game::VerbosityLevel::Normal {
                let card_name = self.cards.get(card_id).map(|c| c.name.as_str()).unwrap_or("Unknown");
                println!("  Tap {} for {{{}}}", card_name, color_symbol);
            }
        }

        Ok(())
    }

    /// Declare a creature as an attacker
    pub fn declare_attacker(&mut self, player_id: PlayerId, card_id: CardId) -> Result<()> {
        // Validate creature can attack
        let card = self.cards.get(card_id)?;

        // Must be a creature
        if !card.is_creature() {
            return Err(MtgError::InvalidAction("Only creatures can attack".to_string()));
        }

        // Must be controlled by the attacking player
        if card.controller != player_id {
            return Err(MtgError::InvalidAction(
                "Can't attack with opponent's creatures".to_string(),
            ));
        }

        // Must be on battlefield
        if !self.battlefield.contains(card_id) {
            return Err(MtgError::InvalidAction(
                "Creature must be on battlefield to attack".to_string(),
            ));
        }

        // Must not be tapped
        if card.tapped {
            return Err(MtgError::InvalidAction(
                "Creature is tapped and can't attack".to_string(),
            ));
        }

        // Check for defender keyword
        // Creatures with defender can't attack
        if card.has_defender() {
            return Err(MtgError::InvalidAction(
                "Creature with defender can't attack".to_string(),
            ));
        }

        // Check for summoning sickness
        // Creatures can't attack the turn they entered the battlefield unless they have haste
        if let Some(entered_turn) = card.turn_entered_battlefield {
            if entered_turn == self.turn.turn_number && !card.has_keyword(&Keyword::Haste) {
                return Err(MtgError::InvalidAction(
                    "Creature has summoning sickness and can't attack this turn".to_string(),
                ));
            }
        }

        // Get defending player (for 2-player, it's the other player)
        let defending_player = self
            .players
            .iter()
            .find(|p| p.id != player_id)
            .map(|p| p.id)
            .ok_or_else(|| MtgError::InvalidAction("No opponent found".to_string()))?;

        // Declare attacker in combat state
        self.combat.declare_attacker(card_id, defending_player);

        // Tap the creature (unless it has vigilance)
        let has_vigilance = self.cards.get(card_id)?.has_keyword(&Keyword::Vigilance);
        if !has_vigilance {
            let card = self.cards.get_mut(card_id)?;
            card.tap();

            // Log the action
            self.undo_log
                .log(crate::undo::GameAction::TapCard { card_id, tapped: true });
        }

        Ok(())
    }

    /// Declare a creature as a blocker
    pub fn declare_blocker(&mut self, player_id: PlayerId, blocker_id: CardId, attackers: Vec<CardId>) -> Result<()> {
        // Validate blocker can block
        let blocker = self.cards.get(blocker_id)?;

        // Must be a creature
        if !blocker.is_creature() {
            return Err(MtgError::InvalidAction("Only creatures can block".to_string()));
        }

        // Must be controlled by the defending player
        if blocker.controller != player_id {
            return Err(MtgError::InvalidAction(
                "Can't block with opponent's creatures".to_string(),
            ));
        }

        // Must be on battlefield
        if !self.battlefield.contains(blocker_id) {
            return Err(MtgError::InvalidAction(
                "Creature must be on battlefield to block".to_string(),
            ));
        }

        // Must not be tapped
        if blocker.tapped {
            return Err(MtgError::InvalidAction(
                "Creature is tapped and can't block".to_string(),
            ));
        }

        // Validate all attackers are actually attacking
        for &attacker in &attackers {
            if !self.combat.is_attacking(attacker) {
                return Err(MtgError::InvalidAction(format!("Card {attacker:?} is not attacking")));
            }
        }

        // Check Flying/Reach restrictions (MTG rule 702.9)
        // A creature with Flying can only be blocked by creatures with Flying or Reach
        let blocker_has_flying = blocker.has_keyword(&Keyword::Flying);
        let blocker_has_reach = blocker.has_keyword(&Keyword::Reach);

        for &attacker_id in &attackers {
            let attacker = self.cards.get(attacker_id)?;
            let attacker_has_flying = attacker.has_keyword(&Keyword::Flying);

            if attacker_has_flying && !blocker_has_flying && !blocker_has_reach {
                return Err(MtgError::InvalidAction(
                    "Creature cannot block attacker with flying (needs flying or reach)".to_string(),
                ));
            }

            // Note: Menace validation (MTG rule 702.111b) would require checking that creatures
            // with Menace have 0 or 2+ blockers, but this can only be validated after all
            // blockers are declared. Controllers should be smart enough not to block a Menace
            // creature with exactly 1 blocker. Incremental validation during blocker declaration
            // would reject the first blocker, which is incorrect.
        }

        // MTG rule: normally a creature can only block one attacker
        // (unless it has an ability that allows it to block multiple)
        if attackers.len() > 1 {
            // TODO: Check for abilities that allow blocking multiple
            return Err(MtgError::InvalidAction(
                "Creature can only block one attacker".to_string(),
            ));
        }

        // Declare blocker
        let mut attackers_vec = smallvec::SmallVec::new();
        for &attacker in &attackers {
            attackers_vec.push(attacker);
        }
        self.combat.declare_blocker(blocker_id, attackers_vec);

        Ok(())
    }

    /// Assign and deal combat damage
    ///
    /// This method handles the combat damage step. For each attacker:
    /// - If unblocked, damage goes to defending player
    /// - If blocked by multiple creatures, attacker's controller chooses damage assignment order
    /// - Damage is assigned in order, with lethal damage assigned to each blocker before the next
    ///
    /// MTG Rules 510.1: Combat damage is assigned and dealt simultaneously.
    /// MTG Rules 510.4: If any creature has first strike or double strike, there are two
    /// combat damage steps. Creatures with first strike or double strike deal damage in the
    /// first step, and creatures without first strike (plus double strike creatures) deal
    /// damage in the second step.
    ///
    /// # Arguments
    /// * `first_strike_step` - True for first strike damage step, false for normal damage step
    pub fn assign_combat_damage(
        &mut self,
        attacker_controller: &mut dyn crate::game::controller::PlayerController,
        blocker_controller: &mut dyn crate::game::controller::PlayerController,
        first_strike_step: bool,
    ) -> Result<()> {
        use crate::game::controller::GameStateView;
        use std::collections::HashMap;

        // First pass: collect all damage assignment orders for attackers with multiple blockers
        let mut damage_orders: HashMap<CardId, SmallVec<[CardId; 4]>> = HashMap::new();

        // Use iterator to avoid Vec allocation for attackers
        for attacker_id in self.combat.attackers_iter() {
            if self.combat.is_blocked(attacker_id) {
                let blockers = self.combat.get_blockers(attacker_id);

                // If multiple blockers, ask attacker's controller for damage assignment order
                if blockers.len() > 1 {
                    let attacker = self.cards.get(attacker_id)?;
                    let attacker_owner = attacker.owner;
                    let view = GameStateView::new(self, attacker_owner);

                    let ordered_blockers = if attacker_owner == attacker_controller.player_id() {
                        attacker_controller.choose_damage_assignment_order(&view, attacker_id, &blockers)
                    } else {
                        blocker_controller.choose_damage_assignment_order(&view, attacker_id, &blockers)
                    };

                    damage_orders.insert(attacker_id, ordered_blockers);
                }
            }
        }

        // Second pass: assign all damage
        let mut damage_to_creatures: HashMap<CardId, i32> = HashMap::new();
        let mut damage_to_players: HashMap<PlayerId, i32> = HashMap::new();
        // Track damage dealt by each creature for lifelink (creature_id -> total damage dealt)
        let mut damage_dealt_by_creature: HashMap<CardId, i32> = HashMap::new();
        // Track creatures dealt deathtouch damage (for state-based destruction)
        let mut deathtouch_damaged_creatures: std::collections::HashSet<CardId> = std::collections::HashSet::new();

        // Use iterator again for second pass (zero allocation)
        for attacker_id in self.combat.attackers_iter() {
            // Skip creatures that are no longer on the battlefield
            // (e.g., died in first strike damage step)
            if !self.battlefield.contains(attacker_id) {
                continue;
            }

            let attacker = self.cards.get(attacker_id)?;

            // Check if this creature deals damage in this step
            // First strike step: only first strike or double strike creatures
            // Normal step: only creatures without first strike, plus double strike creatures
            let deals_damage_this_step = if first_strike_step {
                attacker.has_first_strike() || attacker.has_double_strike()
            } else {
                attacker.has_normal_strike()
            };

            if !deals_damage_this_step {
                continue; // This creature doesn't deal damage in this step
            }

            let mut remaining_power = attacker.current_power();

            if remaining_power <= 0 {
                continue; // 0 or negative power deals no damage
            }

            // Check if attacker is blocked
            if self.combat.is_blocked(attacker_id) {
                // Attacker deals damage to blockers
                let blockers = self.combat.get_blockers(attacker_id);

                // Use the pre-determined order if we have one, otherwise use default order
                let ordered_blockers = damage_orders.get(&attacker_id).cloned().unwrap_or(blockers);

                // Assign damage in order
                // MTG Rules 510.1c:
                // - If exactly one creature is blocking:
                //   * WITHOUT trample: assign ALL damage to that blocker
                //   * WITH trample: assign at least lethal, rest can trample over
                // - If multiple creatures are blocking: assign at least lethal to each
                //   before assigning to the next (can assign more)
                // Note: Current implementation doesn't track damage, so lethal = toughness
                let has_trample = attacker.has_trample();
                for blocker_id in &ordered_blockers {
                    if remaining_power <= 0 {
                        break;
                    }

                    let blocker = self.cards.get(*blocker_id)?;
                    let blocker_toughness = blocker.current_toughness();

                    // Lethal damage is the creature's toughness
                    // MTG Rules 702.2c: If attacker has deathtouch, any nonzero damage is lethal
                    // (In full MTG, this would be toughness minus damage already marked)
                    let has_deathtouch = attacker.has_deathtouch();
                    let lethal_damage = if has_deathtouch && blocker_toughness > 0 {
                        1 // Any nonzero damage from deathtouch is lethal
                    } else {
                        blocker_toughness
                    };

                    let damage_to_assign = if ordered_blockers.len() == 1 && !has_trample {
                        // MTG Rules 510.1c: With exactly one blocker and NO trample,
                        // assign ALL damage to it (even if more than lethal)
                        remaining_power
                    } else {
                        // MTG Rules 510.1c: With trample OR multiple blockers,
                        // assign at least lethal to each before moving to next.
                        // For simplicity, we assign exactly lethal.
                        remaining_power.min(lethal_damage)
                    };

                    if damage_to_assign > 0 {
                        *damage_to_creatures.entry(*blocker_id).or_insert(0) += damage_to_assign as i32;
                        // Track damage for lifelink
                        *damage_dealt_by_creature.entry(attacker_id).or_insert(0) += damage_to_assign as i32;
                        // Track deathtouch damage (MTG Rules 702.2b)
                        if has_deathtouch {
                            deathtouch_damaged_creatures.insert(*blocker_id);
                        }
                        remaining_power -= damage_to_assign;
                    }
                }

                // Trample: If attacker has trample and there's remaining damage after
                // assigning lethal to all blockers, assign remaining to defending player
                // MTG Rules 702.19
                if attacker.has_trample() && remaining_power > 0 {
                    if let Some(defending_player) = self.combat.get_defending_player(attacker_id) {
                        *damage_to_players.entry(defending_player).or_insert(0) += remaining_power as i32;
                        // Track damage for lifelink
                        *damage_dealt_by_creature.entry(attacker_id).or_insert(0) += remaining_power as i32;
                    }
                }

                // All blockers deal their damage back to attacker (simultaneously)
                // But only if they deal damage in this step (same rules as attackers)
                for blocker_id in &ordered_blockers {
                    // Skip blockers that are no longer on the battlefield
                    if !self.battlefield.contains(*blocker_id) {
                        continue;
                    }

                    let blocker = self.cards.get(*blocker_id)?;

                    // Check if blocker deals damage in this step
                    let blocker_deals_damage = if first_strike_step {
                        blocker.has_first_strike() || blocker.has_double_strike()
                    } else {
                        blocker.has_normal_strike()
                    };

                    if !blocker_deals_damage {
                        continue;
                    }

                    let blocker_power = blocker.current_power();
                    if blocker_power > 0 {
                        *damage_to_creatures.entry(attacker_id).or_insert(0) += blocker_power as i32;
                        // Track damage for lifelink
                        *damage_dealt_by_creature.entry(*blocker_id).or_insert(0) += blocker_power as i32;
                        // Track deathtouch damage from blocker (MTG Rules 702.2b)
                        if blocker.has_deathtouch() {
                            deathtouch_damaged_creatures.insert(attacker_id);
                        }
                    }
                }
            } else {
                // Unblocked attacker deals damage to defending player
                if let Some(defending_player) = self.combat.get_defending_player(attacker_id) {
                    *damage_to_players.entry(defending_player).or_insert(0) += remaining_power as i32;
                    // Track damage for lifelink
                    *damage_dealt_by_creature.entry(attacker_id).or_insert(0) += remaining_power as i32;
                }
            }
        }

        // Apply lifelink BEFORE dealing damage (since creatures might die)
        // MTG Rules 702.15: Damage dealt by a source with lifelink also causes
        // its controller to gain that much life
        for (creature_id, total_damage) in &damage_dealt_by_creature {
            if let Ok(creature) = self.cards.get(*creature_id) {
                if creature.has_lifelink() {
                    let controller = creature.controller;
                    if let Ok(player) = self.get_player_mut(controller) {
                        player.gain_life(*total_damage);
                    }
                }
            }
        }

        // Deal all damage simultaneously
        for (creature_id, damage) in damage_to_creatures {
            self.deal_damage_to_creature(creature_id, damage)?;
        }

        for (player_id, damage) in damage_to_players {
            self.deal_damage(player_id, damage)?;
        }

        // Apply deathtouch state-based action (MTG Rules 702.2b)
        // Any creature with toughness > 0 that was dealt damage by a deathtouch source is destroyed
        // MTG Rules 702.12b: Permanents with indestructible can't be destroyed
        for creature_id in deathtouch_damaged_creatures {
            // Check if creature is still on battlefield (might have died from normal damage)
            if self.battlefield.contains(creature_id) {
                if let Ok(creature) = self.cards.get(creature_id) {
                    // Only destroy if it has toughness > 0 and doesn't have indestructible
                    if creature.is_creature() && creature.current_toughness() > 0 && !creature.has_indestructible() {
                        let owner = creature.owner;
                        self.move_card(creature_id, Zone::Battlefield, Zone::Graveyard, owner)?;
                    }
                }
            }
        }

        Ok(())
    }

    /// Pay the cost for an activated ability
    ///
    /// This method pays costs in the correct order:
    /// 1. Tap costs (must happen before zone changes)
    /// 2. Mana costs (pay from mana pool)
    /// 3. Other costs (sacrifice, discard, etc.) - TODO
    ///
    /// Returns Ok(()) if costs were successfully paid, Err otherwise.
    ///
    /// Note: This is a simplified implementation. Full implementation would:
    /// - Support cost refund if payment fails midway
    /// - Handle cost ordering more comprehensively
    /// - Support all cost types (sacrifice, discard, pay life, etc.)
    pub fn pay_ability_cost(&mut self, player_id: PlayerId, card_id: CardId, cost: &crate::core::Cost) -> Result<()> {
        use crate::core::Cost;

        match cost {
            Cost::Tap => {
                // Tap the permanent
                let card = self.cards.get_mut(card_id)?;
                if card.tapped {
                    return Err(MtgError::InvalidAction("Permanent is already tapped".to_string()));
                }
                card.tap();
                Ok(())
            }

            Cost::Mana(mana_cost) => {
                // Pay mana from pool
                let player = self.get_player_mut(player_id)?;
                if !player.mana_pool.can_pay(mana_cost) {
                    return Err(MtgError::InvalidAction("Cannot pay mana cost".to_string()));
                }
                player.mana_pool.pay_cost(mana_cost).map_err(MtgError::InvalidAction)?;
                Ok(())
            }

            Cost::TapAndMana(mana_cost) => {
                // Pay both tap and mana
                // Tap first (must happen before zone changes)
                let card = self.cards.get_mut(card_id)?;
                if card.tapped {
                    return Err(MtgError::InvalidAction("Permanent is already tapped".to_string()));
                }
                card.tap();

                // Then pay mana
                let player = self.get_player_mut(player_id)?;
                if !player.mana_pool.can_pay(mana_cost) {
                    // TODO: Should refund the tap here
                    return Err(MtgError::InvalidAction("Cannot pay mana cost".to_string()));
                }
                player.mana_pool.pay_cost(mana_cost).map_err(MtgError::InvalidAction)?;
                Ok(())
            }

            Cost::PayLife { amount } => {
                // Pay life
                let player = self.get_player_mut(player_id)?;
                if player.life < *amount {
                    return Err(MtgError::InvalidAction("Not enough life".to_string()));
                }
                player.life -= amount;
                Ok(())
            }

            Cost::Untap => {
                // Untap the permanent
                let card = self.cards.get_mut(card_id)?;
                if !card.tapped {
                    return Err(MtgError::InvalidAction("Permanent is not tapped".to_string()));
                }
                card.untap();
                Ok(())
            }

            Cost::SacrificePattern { count, card_type } => {
                // Find permanents matching the pattern and sacrifice them
                // For now, automatically choose without asking the controller
                // TODO: Let controller choose which permanents to sacrifice

                let mut to_sacrifice = Vec::new();

                // Special case: CARDNAME means the card with this ability
                if card_type == "CARDNAME" {
                    to_sacrifice.push(card_id);
                } else {
                    // Find permanents on battlefield matching the type
                    // Collect IDs first to avoid borrowing issues
                    let battlefield_cards = self.battlefield.cards.to_vec();

                    for permanent_id in battlefield_cards {
                        if to_sacrifice.len() >= *count as usize {
                            break;
                        }

                        let card = self.cards.get(permanent_id)?;

                        // Check ownership
                        if card.owner != player_id {
                            continue;
                        }

                        // Check if it matches the pattern
                        let matches = if card_type == "Land" {
                            card.is_land()
                        } else if card_type.starts_with("Creature") {
                            if card_type == "Creature.Other" {
                                // Other means not the card with the ability
                                card.is_creature() && permanent_id != card_id
                            } else {
                                card.is_creature()
                            }
                        } else if card_type == "Artifact" {
                            card.is_artifact()
                        } else {
                            // Generic type match - check if any type contains the string
                            card.types.iter().any(|t| format!("{t:?}").contains(card_type))
                        };

                        if matches {
                            to_sacrifice.push(permanent_id);
                        }
                    }
                }

                // Check if we found enough permanents to sacrifice
                if to_sacrifice.len() < *count as usize {
                    return Err(MtgError::InvalidAction(format!(
                        "Not enough permanents of type {} to sacrifice (need {}, found {})",
                        card_type,
                        count,
                        to_sacrifice.len()
                    )));
                }

                // Sacrifice the permanents (move to graveyard)
                for sac_id in to_sacrifice.iter().take(*count as usize) {
                    let owner = self.cards.get(*sac_id)?.owner;
                    self.move_card(*sac_id, Zone::Battlefield, Zone::Graveyard, owner)?;
                }

                Ok(())
            }

            Cost::Sacrifice { card_id: sac_id } => {
                // Sacrifice a specific permanent (move to graveyard)
                let owner = self.cards.get(*sac_id)?.owner;
                self.move_card(*sac_id, Zone::Battlefield, Zone::Graveyard, owner)
            }

            Cost::Discard { card_id: _ } => {
                // TODO: Implement discard cost
                Err(MtgError::InvalidAction(format!(
                    "Cost type {cost:?} not yet implemented"
                )))
            }

            Cost::Composite(costs) => {
                // Pay each cost in order
                for sub_cost in costs {
                    self.pay_ability_cost(player_id, card_id, sub_cost)?;
                }
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{Card, ManaCost};
    use crate::game::ZeroController;
    use crate::loader::CardDatabase;
    use std::path::PathBuf;

    /// Helper to load a card from the cardsfolder for tests
    fn load_test_card(game: &mut GameState, card_name: &str, owner_id: PlayerId) -> Result<CardId> {
        let card_id = game.next_entity_id();

        // Load card definition from cardsfolder
        let cardsfolder = PathBuf::from("./cardsfolder");
        let db = CardDatabase::new(cardsfolder);

        let card_def = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async { db.get_card(card_name).await })?
            .ok_or_else(|| MtgError::InvalidCardFormat(format!("Card not found: {card_name}")))?;

        // Create card instance from definition
        let card = card_def.instantiate(card_id, owner_id);
        game.cards.insert(card_id, card);

        Ok(card_id)
    }

    #[test]
    fn test_play_land() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);

        let p1_id = game.players.first().unwrap().id;

        // Create a mountain card
        let card_id = game.next_entity_id();
        let mut card = Card::new(card_id, "Mountain".to_string(), p1_id);
        card.types.push(CardType::Land);
        game.cards.insert(card_id, card);

        // Add to hand
        if let Some(zones) = game.get_player_zones_mut(p1_id) {
            zones.hand.add(card_id);
        }

        // Play the land
        assert!(game.play_land(p1_id, card_id).is_ok());

        // Check it's on battlefield
        assert!(game.battlefield.contains(card_id));

        // Check player used their land drop
        let player = game.get_player(p1_id).unwrap();
        assert!(!player.can_play_land());
    }

    #[test]
    fn test_tap_for_mana() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);

        let p1_id = game.players.first().unwrap().id;

        // Create a mountain on battlefield
        let card_id = game.next_entity_id();
        let mut card = Card::new(card_id, "Mountain".to_string(), p1_id);
        card.types.push(CardType::Land);
        game.cards.insert(card_id, card);
        game.battlefield.add(card_id);

        // Tap for mana
        assert!(game.tap_for_mana(p1_id, card_id).is_ok());

        // Check mana was added
        let player = game.get_player(p1_id).unwrap();
        assert_eq!(player.mana_pool.red, 1);

        // Check land is tapped
        let card = game.cards.get(card_id).unwrap();
        assert!(card.tapped);
    }

    #[test]
    fn test_deal_damage_to_player() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);

        let p1_id = game.players.first().unwrap().id;

        // Deal 3 damage
        assert!(game.deal_damage(p1_id, 3).is_ok());

        let player = game.get_player(p1_id).unwrap();
        assert_eq!(player.life, 17);
    }

    #[test]
    fn test_move_card_battlefield_to_graveyard() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);

        let p1_id = game.players.first().unwrap().id;

        // Create a creature on battlefield
        let card_id = game.next_entity_id();
        let card = Card::new(card_id, "Test Card".to_string(), p1_id);
        game.cards.insert(card_id, card);
        game.battlefield.add(card_id);

        // Test move_card directly
        let result = game.move_card(card_id, Zone::Battlefield, Zone::Graveyard, p1_id);
        if let Err(e) = &result {
            panic!("move_card failed: {e:?}");
        }

        // Check it moved
        assert!(!game.battlefield.contains(card_id), "Card still on battlefield");
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert!(zones.graveyard.contains(card_id), "Card not in graveyard");
        }
    }

    #[test]
    fn test_deal_damage_to_creature() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);

        let p1_id = game.players.first().unwrap().id;

        // Create a 2/2 creature on battlefield
        let card_id = game.next_card_id();
        let mut card = Card::new(card_id, "Grizzly Bears".to_string(), p1_id);
        card.types.push(CardType::Creature);
        card.power = Some(2);
        card.toughness = Some(2);
        game.cards.insert(card_id, card);
        game.battlefield.add(card_id);

        // Deal 2 damage (should kill it)
        let result = game.deal_damage_to_creature(card_id, 2);
        assert!(result.is_ok(), "deal_damage_to_creature failed: {result:?}");

        // Check it's in graveyard
        assert!(!game.battlefield.contains(card_id), "Card still on battlefield");
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert!(zones.graveyard.contains(card_id), "Card not in graveyard");
        }
    }

    #[test]
    fn test_cast_spell_with_mana_payment() {
        use crate::core::{Color, ManaCost};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players.first().unwrap().id;

        // Create a Lightning Bolt in hand (cost: R)
        let bolt_id = game.next_card_id();
        let mut bolt = Card::new(bolt_id, "Lightning Bolt".to_string(), p1_id);
        bolt.types.push(CardType::Instant);
        bolt.mana_cost = ManaCost::from_string("R");
        game.cards.insert(bolt_id, bolt);

        // Add to hand
        if let Some(zones) = game.get_player_zones_mut(p1_id) {
            zones.hand.add(bolt_id);
        }

        // Try to cast without mana - should fail
        let result = game.cast_spell(p1_id, bolt_id, vec![]);
        assert!(result.is_err());

        // Add mana to pool
        let player = game.get_player_mut(p1_id).unwrap();
        player.mana_pool.add_color(Color::Red);

        // Now cast should succeed
        let result = game.cast_spell(p1_id, bolt_id, vec![]);
        assert!(result.is_ok(), "cast_spell failed: {result:?}");

        // Check mana was deducted
        let player = game.get_player(p1_id).unwrap();
        assert_eq!(player.mana_pool.red, 0);

        // Check card is on stack
        assert!(game.stack.contains(bolt_id));
    }

    #[test]
    fn test_cast_spell_with_generic_mana() {
        use crate::core::{Color, ManaCost};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players.first().unwrap().id;

        // Create a spell with cost 2R
        let spell_id = game.next_card_id();
        let mut spell = Card::new(spell_id, "Lava Spike".to_string(), p1_id);
        spell.types.push(CardType::Sorcery);
        spell.mana_cost = ManaCost::from_string("2R");
        game.cards.insert(spell_id, spell);

        // Add to hand
        if let Some(zones) = game.get_player_zones_mut(p1_id) {
            zones.hand.add(spell_id);
        }

        // Add mana: 2R + 1U = 4 mana total
        let player = game.get_player_mut(p1_id).unwrap();
        player.mana_pool.add_color(Color::Red);
        player.mana_pool.add_color(Color::Red);
        player.mana_pool.add_color(Color::Blue);

        // Cast spell - should use 1R for R, and 2R for generic 2
        let result = game.cast_spell(p1_id, spell_id, vec![]);
        assert!(result.is_ok(), "cast_spell failed: {result:?}");

        // Check mana was deducted properly (should have 1 blue left)
        let player = game.get_player(p1_id).unwrap();
        assert_eq!(player.mana_pool.red, 0);
        assert_eq!(player.mana_pool.blue, 0); // Blue was used for generic cost
        assert_eq!(player.mana_pool.total(), 0);

        // Check card is on stack
        assert!(game.stack.contains(spell_id));
    }

    #[test]
    fn test_execute_damage_effect_to_player() {
        use crate::core::{Effect, TargetRef};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let players: Vec<_> = game.players.iter().map(|p| p.id).collect();
        let p2_id = players[1];

        let effect = Effect::DealDamage {
            target: TargetRef::Player(p2_id),
            amount: 3,
        };

        assert!(game.execute_effect(&effect).is_ok());

        let p2 = game.get_player(p2_id).unwrap();
        assert_eq!(p2.life, 17);
    }

    #[test]
    fn test_execute_draw_effect() {
        use crate::core::Effect;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players.first().unwrap().id;

        // Add cards to library
        for i in 0..5 {
            let card_id = game.next_card_id();
            let card = Card::new(card_id, format!("Card {i}"), p1_id);
            game.cards.insert(card_id, card);
            if let Some(zones) = game.get_player_zones_mut(p1_id) {
                zones.library.add(card_id);
            }
        }

        let effect = Effect::DrawCards {
            player: p1_id,
            count: 2,
        };

        assert!(game.execute_effect(&effect).is_ok());

        // Check cards were drawn
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert_eq!(zones.hand.cards.len(), 2);
            assert_eq!(zones.library.cards.len(), 3);
        }
    }

    #[test]
    fn test_resolve_spell_with_effects() {
        use crate::core::{Effect, ManaCost, TargetRef};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let players: Vec<_> = game.players.iter().map(|p| p.id).collect();
        let p1_id = players[0];
        let p2_id = players[1];

        // Create Lightning Bolt with damage effect
        let bolt_id = game.next_card_id();
        let mut bolt = Card::new(bolt_id, "Lightning Bolt".to_string(), p1_id);
        bolt.types.push(CardType::Instant);
        bolt.mana_cost = ManaCost::from_string("R");
        bolt.effects.push(Effect::DealDamage {
            target: TargetRef::Player(p2_id),
            amount: 3,
        });
        game.cards.insert(bolt_id, bolt);

        // Put it on the stack (simulating cast)
        game.stack.add(bolt_id);

        // Resolve the spell
        assert!(game.resolve_spell(bolt_id, &[]).is_ok());

        // Check damage was dealt
        let p2 = game.get_player(p2_id).unwrap();
        assert_eq!(p2.life, 17);

        // Check spell went to graveyard
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert!(zones.graveyard.contains(bolt_id));
        }
    }

    #[test]
    fn test_resolve_draw_spell() {
        use crate::core::{Effect, ManaCost};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let players: Vec<_> = game.players.iter().map(|p| p.id).collect();
        let p1_id = players[0];

        // Add cards to P1's library
        for i in 0..5 {
            let card_id = game.next_card_id();
            let card = Card::new(card_id, format!("Card {i}"), p1_id);
            game.cards.insert(card_id, card);
            if let Some(zones) = game.get_player_zones_mut(p1_id) {
                zones.library.add(card_id);
            }
        }

        // Create a Draw spell (like Divination)
        let draw_spell_id = game.next_card_id();
        let mut draw_spell = Card::new(draw_spell_id, "Divination".to_string(), p1_id);
        draw_spell.types.push(CardType::Sorcery);
        draw_spell.mana_cost = ManaCost::from_string("2U");
        // Use placeholder player ID 0 which will be replaced with card owner
        draw_spell.effects.push(Effect::DrawCards {
            player: PlayerId::new(0),
            count: 2,
        });
        game.cards.insert(draw_spell_id, draw_spell);

        // Put it on the stack (simulating cast)
        game.stack.add(draw_spell_id);

        // Check initial state
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert_eq!(zones.hand.cards.len(), 0, "Should start with 0 cards in hand");
            assert_eq!(zones.library.cards.len(), 5, "Should have 5 cards in library");
        }

        // Resolve the spell
        assert!(
            game.resolve_spell(draw_spell_id, &[]).is_ok(),
            "Failed to resolve draw spell"
        );

        // Check cards were drawn
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert_eq!(zones.hand.cards.len(), 2, "Should have drawn 2 cards");
            assert_eq!(zones.library.cards.len(), 3, "Should have 3 cards left in library");
        }

        // Check spell went to graveyard
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert!(
                zones.graveyard.contains(draw_spell_id),
                "Draw spell should be in graveyard"
            );
        }
    }

    #[test]
    fn test_resolve_destroy_spell() {
        use crate::core::{Effect, ManaCost};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let players: Vec<_> = game.players.iter().map(|p| p.id).collect();
        let p1_id = players[0];
        let p2_id = players[1];

        // Create a creature for P2 (the target)
        let target_creature_id = game.next_card_id();
        let mut target = Card::new(target_creature_id, "Grizzly Bears".to_string(), p2_id);
        target.types.push(CardType::Creature);
        target.power = Some(2);
        target.toughness = Some(2);
        target.controller = p2_id;
        game.cards.insert(target_creature_id, target);
        game.battlefield.add(target_creature_id);

        // Create a Destroy spell (like Terror)
        let destroy_spell_id = game.next_card_id();
        let mut destroy_spell = Card::new(destroy_spell_id, "Terror".to_string(), p1_id);
        destroy_spell.types.push(CardType::Instant);
        destroy_spell.mana_cost = ManaCost::from_string("1B");
        // Use placeholder card ID 0 which will be replaced with an opponent's creature
        destroy_spell
            .effects
            .push(Effect::DestroyPermanent { target: CardId::new(0) });
        game.cards.insert(destroy_spell_id, destroy_spell);

        // Put it on the stack (simulating cast)
        game.stack.add(destroy_spell_id);

        // Check initial state
        assert!(
            game.battlefield.contains(target_creature_id),
            "Target creature should be on battlefield"
        );

        // Resolve the spell with the target creature
        assert!(
            game.resolve_spell(destroy_spell_id, &[target_creature_id]).is_ok(),
            "Failed to resolve destroy spell"
        );

        // Check target creature was destroyed (moved to graveyard)
        assert!(
            !game.battlefield.contains(target_creature_id),
            "Target creature should not be on battlefield"
        );

        if let Some(zones) = game.get_player_zones(p2_id) {
            assert!(
                zones.graveyard.contains(target_creature_id),
                "Target creature should be in graveyard"
            );
        }

        // Check spell went to graveyard
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert!(
                zones.graveyard.contains(destroy_spell_id),
                "Destroy spell should be in graveyard"
            );
        }
    }

    #[test]
    fn test_resolve_gainlife_spell() {
        use crate::core::{Effect, ManaCost};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let players: Vec<_> = game.players.iter().map(|p| p.id).collect();
        let p1_id = players[0];

        // Create a GainLife spell (like Angel's Mercy)
        let gainlife_spell_id = game.next_card_id();
        let mut gainlife_spell = Card::new(gainlife_spell_id, "Angel's Mercy".to_string(), p1_id);
        gainlife_spell.types.push(CardType::Instant);
        gainlife_spell.mana_cost = ManaCost::from_string("2WW");
        // Use placeholder player ID 0 which will be replaced with card controller
        gainlife_spell.effects.push(Effect::GainLife {
            player: PlayerId::new(0),
            amount: 7,
        });
        game.cards.insert(gainlife_spell_id, gainlife_spell);

        // Put it on the stack (simulating cast)
        game.stack.add(gainlife_spell_id);

        // Check initial life total
        let p1_before = game.get_player(p1_id).unwrap();
        assert_eq!(p1_before.life, 20, "Should start with 20 life");

        // Resolve the spell
        assert!(
            game.resolve_spell(gainlife_spell_id, &[]).is_ok(),
            "Failed to resolve gain life spell"
        );

        // Check life was gained
        let p1_after = game.get_player(p1_id).unwrap();
        assert_eq!(p1_after.life, 27, "Should have gained 7 life (20 + 7)");

        // Check spell went to graveyard
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert!(
                zones.graveyard.contains(gainlife_spell_id),
                "GainLife spell should be in graveyard"
            );
        }
    }

    #[test]
    fn test_declare_attacker() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let players: Vec<_> = game.players.iter().map(|p| p.id).collect();
        let p1_id = players[0];

        // Create a creature
        let creature_id = game.next_card_id();
        let mut creature = Card::new(creature_id, "Grizzly Bears".to_string(), p1_id);
        creature.types.push(CardType::Creature);
        creature.power = Some(2);
        creature.toughness = Some(2);
        creature.controller = p1_id;
        game.cards.insert(creature_id, creature);

        // Put creature on battlefield
        game.battlefield.add(creature_id);

        // Declare attacker
        let result = game.declare_attacker(p1_id, creature_id);
        assert!(result.is_ok(), "Failed to declare attacker: {result:?}");

        // Check creature is attacking
        assert!(game.combat.is_attacking(creature_id));

        // Check creature is tapped
        let creature = game.cards.get(creature_id).unwrap();
        assert!(creature.tapped);
    }

    #[test]
    fn test_declare_blocker() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let players: Vec<_> = game.players.iter().map(|p| p.id).collect();
        let p1_id = players[0];
        let p2_id = players[1];

        // Create an attacker
        let attacker_id = game.next_card_id();
        let mut attacker = Card::new(attacker_id, "Goblin".to_string(), p1_id);
        attacker.types.push(CardType::Creature);
        attacker.power = Some(2);
        attacker.toughness = Some(1);
        attacker.controller = p1_id;
        game.cards.insert(attacker_id, attacker);
        game.battlefield.add(attacker_id);

        // Declare as attacker
        game.combat.declare_attacker(attacker_id, p2_id);

        // Create a blocker
        let blocker_id = game.next_card_id();
        let mut blocker = Card::new(blocker_id, "Wall".to_string(), p2_id);
        blocker.types.push(CardType::Creature);
        blocker.power = Some(0);
        blocker.toughness = Some(3);
        blocker.controller = p2_id;
        game.cards.insert(blocker_id, blocker);
        game.battlefield.add(blocker_id);

        // Declare blocker
        let result = game.declare_blocker(p2_id, blocker_id, vec![attacker_id]);
        assert!(result.is_ok(), "Failed to declare blocker: {result:?}");

        // Check blocker is blocking
        assert!(game.combat.is_blocking(blocker_id));
        assert!(game.combat.is_blocked(attacker_id));

        let blockers = game.combat.get_blockers(attacker_id);
        assert_eq!(blockers.len(), 1);
        assert!(blockers.contains(&blocker_id));
    }

    #[test]
    fn test_combat_damage_unblocked() {
        use crate::game::zero_controller::ZeroController;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let players: Vec<_> = game.players.iter().map(|p| p.id).collect();
        let p1_id = players[0];
        let p2_id = players[1];

        // Create an attacker
        let attacker_id = game.next_card_id();
        let mut attacker = Card::new(attacker_id, "Dragon".to_string(), p1_id);
        attacker.types.push(CardType::Creature);
        attacker.power = Some(5);
        attacker.toughness = Some(5);
        attacker.controller = p1_id;
        game.cards.insert(attacker_id, attacker);
        game.battlefield.add(attacker_id);

        // Declare as attacker (unblocked)
        game.combat.declare_attacker(attacker_id, p2_id);

        // Create controllers
        let mut controller1 = ZeroController::new(p1_id);
        let mut controller2 = ZeroController::new(p2_id);

        // Assign combat damage
        let result = game.assign_combat_damage(&mut controller1, &mut controller2, false);
        assert!(result.is_ok(), "Failed to assign combat damage: {result:?}");

        // Check defending player took damage
        let p2 = game.get_player(p2_id).unwrap();
        assert_eq!(p2.life, 15); // 20 - 5 = 15
    }

    #[test]
    fn test_combat_damage_blocked() {
        use crate::game::zero_controller::ZeroController;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let players: Vec<_> = game.players.iter().map(|p| p.id).collect();
        let p1_id = players[0];
        let p2_id = players[1];

        // Create an attacker (3/3)
        let attacker_id = game.next_card_id();
        let mut attacker = Card::new(attacker_id, "Bear".to_string(), p1_id);
        attacker.types.push(CardType::Creature);
        attacker.power = Some(3);
        attacker.toughness = Some(3);
        attacker.controller = p1_id;
        game.cards.insert(attacker_id, attacker);
        game.battlefield.add(attacker_id);

        // Create a blocker (2/2)
        let blocker_id = game.next_card_id();
        let mut blocker = Card::new(blocker_id, "Wolf".to_string(), p2_id);
        blocker.types.push(CardType::Creature);
        blocker.power = Some(2);
        blocker.toughness = Some(2);
        blocker.controller = p2_id;
        game.cards.insert(blocker_id, blocker);
        game.battlefield.add(blocker_id);

        // Declare attacker and blocker
        game.combat.declare_attacker(attacker_id, p2_id);
        let blocker_vec = smallvec::SmallVec::from_vec(vec![attacker_id]);
        game.combat.declare_blocker(blocker_id, blocker_vec);

        // Create controllers
        let mut controller1 = ZeroController::new(p1_id);
        let mut controller2 = ZeroController::new(p2_id);

        // Assign combat damage
        let result = game.assign_combat_damage(&mut controller1, &mut controller2, false);
        assert!(result.is_ok(), "Failed to assign combat damage: {result:?}");

        // Check defending player took no damage (blocked)
        let p2 = game.get_player(p2_id).unwrap();
        assert_eq!(p2.life, 20);

        // Check blocker died (took 2 damage, toughness 2)
        if let Some(zones) = game.get_player_zones(p2_id) {
            assert!(zones.graveyard.contains(blocker_id));
        }

        // Check attacker took 2 damage but has toughness 3, so it survives
        assert!(game.battlefield.contains(attacker_id));
    }

    #[test]
    fn test_combat_damage_multiple_blockers() {
        use crate::game::zero_controller::ZeroController;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let players: Vec<_> = game.players.iter().map(|p| p.id).collect();
        let p1_id = players[0];
        let p2_id = players[1];

        // Create a powerful attacker (5/5)
        let attacker_id = game.next_card_id();
        let mut attacker = Card::new(attacker_id, "Dragon".to_string(), p1_id);
        attacker.types.push(CardType::Creature);
        attacker.power = Some(5);
        attacker.toughness = Some(5);
        attacker.controller = p1_id;
        game.cards.insert(attacker_id, attacker);
        game.battlefield.add(attacker_id);

        // Create first blocker (2/2)
        let blocker1_id = game.next_card_id();
        let mut blocker1 = Card::new(blocker1_id, "Bear".to_string(), p2_id);
        blocker1.types.push(CardType::Creature);
        blocker1.power = Some(2);
        blocker1.toughness = Some(2);
        blocker1.controller = p2_id;
        game.cards.insert(blocker1_id, blocker1);
        game.battlefield.add(blocker1_id);

        // Create second blocker (3/3)
        let blocker2_id = game.next_card_id();
        let mut blocker2 = Card::new(blocker2_id, "Wolf".to_string(), p2_id);
        blocker2.types.push(CardType::Creature);
        blocker2.power = Some(3);
        blocker2.toughness = Some(3);
        blocker2.controller = p2_id;
        game.cards.insert(blocker2_id, blocker2);
        game.battlefield.add(blocker2_id);

        // Declare attacker and both blockers
        game.combat.declare_attacker(attacker_id, p2_id);
        let blocker_vec = smallvec::SmallVec::from_vec(vec![attacker_id]);
        game.combat.declare_blocker(blocker1_id, blocker_vec.clone());
        game.combat.declare_blocker(blocker2_id, blocker_vec);

        // Create controllers
        let mut controller1 = ZeroController::new(p1_id);
        let mut controller2 = ZeroController::new(p2_id);

        // Assign combat damage
        // ZeroController will keep the order as-is
        // Dragon (5 power) assigns: 2 to first blocker (lethal), 3 to second blocker (lethal)
        // Both blockers (2+3=5 power) deal 5 damage back to Dragon (lethal)
        let result = game.assign_combat_damage(&mut controller1, &mut controller2, false);
        assert!(result.is_ok(), "Failed to assign combat damage: {result:?}");

        // Check defending player took no damage (blocked)
        let p2 = game.get_player(p2_id).unwrap();
        assert_eq!(p2.life, 20);

        // Check both blockers died
        if let Some(zones) = game.get_player_zones(p2_id) {
            assert!(
                zones.graveyard.contains(blocker1_id),
                "First blocker should be in graveyard"
            );
            assert!(
                zones.graveyard.contains(blocker2_id),
                "Second blocker should be in graveyard"
            );
        }

        // Check attacker died (took 5 damage, toughness 5)
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert!(zones.graveyard.contains(attacker_id), "Attacker should be in graveyard");
        }
    }

    #[test]
    fn test_summoning_sickness_blocks_attack() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Create a creature and put it on battlefield
        let creature_id = game.next_entity_id();
        let mut creature = Card::new(creature_id, "Grizzly Bears".to_string(), p1_id);
        creature.types.push(CardType::Creature);
        creature.power = Some(2);
        creature.toughness = Some(2);
        creature.controller = p1_id;
        game.cards.insert(creature_id, creature);
        game.battlefield.add(creature_id);

        // Mark it as entering this turn (summoning sickness)
        if let Ok(card) = game.cards.get_mut(creature_id) {
            card.turn_entered_battlefield = Some(game.turn.turn_number);
        }

        // Try to declare it as an attacker - should fail due to summoning sickness
        let result = game.declare_attacker(p1_id, creature_id);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("summoning sickness"));
    }

    #[test]
    fn test_summoning_sickness_allows_attack_next_turn() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Create a creature and put it on battlefield
        let creature_id = game.next_entity_id();
        let mut creature = Card::new(creature_id, "Grizzly Bears".to_string(), p1_id);
        creature.types.push(CardType::Creature);
        creature.power = Some(2);
        creature.toughness = Some(2);
        creature.controller = p1_id;
        game.cards.insert(creature_id, creature);
        game.battlefield.add(creature_id);

        // Mark it as entering on a previous turn
        if let Ok(card) = game.cards.get_mut(creature_id) {
            card.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        }

        // Declare it as an attacker - should succeed
        let result = game.declare_attacker(p1_id, creature_id);
        assert!(result.is_ok());
        assert!(game.combat.is_attacking(creature_id));
    }

    #[test]
    fn test_haste_bypasses_summoning_sickness() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Create a creature with haste
        let creature_id = game.next_entity_id();
        let mut creature = Card::new(creature_id, "Lightning Elemental".to_string(), p1_id);
        creature.types.push(CardType::Creature);
        creature.power = Some(4);
        creature.toughness = Some(1);
        creature.controller = p1_id;
        creature.keywords.push(Keyword::Haste);
        game.cards.insert(creature_id, creature);
        game.battlefield.add(creature_id);

        // Mark it as entering this turn
        if let Ok(card) = game.cards.get_mut(creature_id) {
            card.turn_entered_battlefield = Some(game.turn.turn_number);
        }

        // Declare it as an attacker - should succeed because of haste
        let result = game.declare_attacker(p1_id, creature_id);
        assert!(result.is_ok());
        assert!(game.combat.is_attacking(creature_id));
    }

    #[test]
    fn test_defender_blocks_attack() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Create a creature with defender
        let creature_id = game.next_entity_id();
        let mut creature = Card::new(creature_id, "Wall of Stone".to_string(), p1_id);
        creature.types.push(CardType::Creature);
        creature.power = Some(0);
        creature.toughness = Some(8);
        creature.controller = p1_id;
        creature.keywords.push(Keyword::Defender);
        game.cards.insert(creature_id, creature);
        game.battlefield.add(creature_id);

        // Mark it as entering on a previous turn (no summoning sickness)
        if let Ok(card) = game.cards.get_mut(creature_id) {
            card.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        }

        // Try to declare it as an attacker - should fail because of defender
        let result = game.declare_attacker(p1_id, creature_id);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("defender can't attack"));
        assert!(!game.combat.is_attacking(creature_id));
    }

    #[test]
    fn test_vigilance_creature_stays_untapped() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Load Serra Angel (4/4 with Flying and Vigilance)
        let creature_id = load_test_card(&mut game, "Serra Angel", p1_id).expect("Failed to load Serra Angel");

        if let Ok(creature) = game.cards.get_mut(creature_id) {
            creature.controller = p1_id;
        }
        game.battlefield.add(creature_id);

        // Mark it as entering on a previous turn (no summoning sickness)
        if let Ok(card) = game.cards.get_mut(creature_id) {
            card.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        }

        // Declare it as an attacker
        let result = game.declare_attacker(p1_id, creature_id);
        assert!(result.is_ok());
        assert!(game.combat.is_attacking(creature_id));

        // Check that creature is still untapped (vigilance effect)
        let card = game.cards.get(creature_id).unwrap();
        assert!(
            !card.tapped,
            "Creature with vigilance should not be tapped after attacking"
        );
    }

    #[test]
    fn test_non_vigilance_creature_gets_tapped() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Create a creature WITHOUT vigilance
        let creature_id = game.next_entity_id();
        let mut creature = Card::new(creature_id, "Grizzly Bears".to_string(), p1_id);
        creature.types.push(CardType::Creature);
        creature.power = Some(2);
        creature.toughness = Some(2);
        creature.controller = p1_id;
        game.cards.insert(creature_id, creature);
        game.battlefield.add(creature_id);

        // Mark it as entering on a previous turn (no summoning sickness)
        if let Ok(card) = game.cards.get_mut(creature_id) {
            card.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        }

        // Declare it as an attacker
        let result = game.declare_attacker(p1_id, creature_id);
        assert!(result.is_ok());
        assert!(game.combat.is_attacking(creature_id));

        // Check that creature is tapped (normal attack behavior)
        let card = game.cards.get(creature_id).unwrap();
        assert!(
            card.tapped,
            "Creature without vigilance should be tapped after attacking"
        );
    }

    #[test]
    fn test_flying_creature_blocked_by_flying() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1: Load Storm Crow (1/2 with Flying) as attacker
        let attacker_id = load_test_card(&mut game, "Storm Crow", p1_id).expect("Failed to load Storm Crow");

        if let Ok(attacker) = game.cards.get_mut(attacker_id) {
            attacker.controller = p1_id;
            attacker.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        }
        game.battlefield.add(attacker_id);

        // P2: Load Segovian Angel (1/1 with Flying and Vigilance) as blocker
        let blocker_id = load_test_card(&mut game, "Segovian Angel", p2_id).expect("Failed to load Segovian Angel");

        if let Ok(blocker) = game.cards.get_mut(blocker_id) {
            blocker.controller = p2_id;
        }
        game.battlefield.add(blocker_id);

        // Declare attacker
        game.declare_attacker(p1_id, attacker_id).unwrap();

        // Blocker with flying should be able to block attacker with flying
        let result = game.declare_blocker(p2_id, blocker_id, vec![attacker_id]);
        assert!(
            result.is_ok(),
            "Creature with flying should be able to block creature with flying"
        );
    }

    #[test]
    fn test_flying_creature_blocked_by_reach() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1: Load Storm Crow (1/2 with Flying) as attacker
        let attacker_id = load_test_card(&mut game, "Storm Crow", p1_id).expect("Failed to load Storm Crow");

        if let Ok(attacker) = game.cards.get_mut(attacker_id) {
            attacker.controller = p1_id;
            attacker.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        }
        game.battlefield.add(attacker_id);

        // P2: Load Giant Spider (2/4 with Reach) as blocker
        let blocker_id = load_test_card(&mut game, "Giant Spider", p2_id).expect("Failed to load Giant Spider");

        if let Ok(blocker) = game.cards.get_mut(blocker_id) {
            blocker.controller = p2_id;
        }
        game.battlefield.add(blocker_id);

        // Declare attacker
        game.declare_attacker(p1_id, attacker_id).unwrap();

        // Blocker with reach should be able to block attacker with flying
        let result = game.declare_blocker(p2_id, blocker_id, vec![attacker_id]);
        assert!(
            result.is_ok(),
            "Creature with reach should be able to block creature with flying"
        );
    }

    #[test]
    fn test_flying_creature_cannot_be_blocked_by_non_flying() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1: Create a creature with Flying (attacker)
        let attacker_id = game.next_entity_id();
        let mut attacker = Card::new(attacker_id, "Storm Crow".to_string(), p1_id);
        attacker.types.push(CardType::Creature);
        attacker.power = Some(1);
        attacker.toughness = Some(2);
        attacker.controller = p1_id;
        attacker.keywords.push(Keyword::Flying);
        attacker.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        game.cards.insert(attacker_id, attacker);
        game.battlefield.add(attacker_id);

        // P2: Create a creature without Flying or Reach (blocker)
        let blocker_id = game.next_entity_id();
        let mut blocker = Card::new(blocker_id, "Grizzly Bears".to_string(), p2_id);
        blocker.types.push(CardType::Creature);
        blocker.power = Some(2);
        blocker.toughness = Some(2);
        blocker.controller = p2_id;
        game.cards.insert(blocker_id, blocker);
        game.battlefield.add(blocker_id);

        // Declare attacker
        game.declare_attacker(p1_id, attacker_id).unwrap();

        // Blocker without flying or reach should NOT be able to block attacker with flying
        let result = game.declare_blocker(p2_id, blocker_id, vec![attacker_id]);
        assert!(
            result.is_err(),
            "Creature without flying or reach should not be able to block creature with flying"
        );
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("cannot block attacker with flying"));
    }

    #[test]
    fn test_non_flying_creature_blocked_by_any() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1: Create a creature without Flying (attacker)
        let attacker_id = game.next_entity_id();
        let mut attacker = Card::new(attacker_id, "Grizzly Bears".to_string(), p1_id);
        attacker.types.push(CardType::Creature);
        attacker.power = Some(2);
        attacker.toughness = Some(2);
        attacker.controller = p1_id;
        attacker.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        game.cards.insert(attacker_id, attacker);
        game.battlefield.add(attacker_id);

        // P2: Create a creature without Flying or Reach (blocker)
        let blocker_id = game.next_entity_id();
        let mut blocker = Card::new(blocker_id, "Hill Giant".to_string(), p2_id);
        blocker.types.push(CardType::Creature);
        blocker.power = Some(3);
        blocker.toughness = Some(3);
        blocker.controller = p2_id;
        game.cards.insert(blocker_id, blocker);
        game.battlefield.add(blocker_id);

        // Declare attacker
        game.declare_attacker(p1_id, attacker_id).unwrap();

        // Any creature should be able to block a non-flying creature
        let result = game.declare_blocker(p2_id, blocker_id, vec![attacker_id]);
        assert!(
            result.is_ok(),
            "Any creature should be able to block a non-flying creature"
        );
    }

    #[test]
    fn test_flying_and_reach_blocker_can_block_flying() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1: Create a creature with Flying (attacker)
        let attacker_id = game.next_entity_id();
        let mut attacker = Card::new(attacker_id, "Storm Crow".to_string(), p1_id);
        attacker.types.push(CardType::Creature);
        attacker.power = Some(1);
        attacker.toughness = Some(2);
        attacker.controller = p1_id;
        attacker.keywords.push(Keyword::Flying);
        attacker.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        game.cards.insert(attacker_id, attacker);
        game.battlefield.add(attacker_id);

        // P2: Create a creature with both Flying AND Reach (blocker)
        let blocker_id = game.next_entity_id();
        let mut blocker = Card::new(blocker_id, "Mystic Drake".to_string(), p2_id);
        blocker.types.push(CardType::Creature);
        blocker.power = Some(2);
        blocker.toughness = Some(3);
        blocker.controller = p2_id;
        blocker.keywords.push(Keyword::Flying);
        blocker.keywords.push(Keyword::Reach);
        game.cards.insert(blocker_id, blocker);
        game.battlefield.add(blocker_id);

        // Declare attacker
        game.declare_attacker(p1_id, attacker_id).unwrap();

        // Blocker with both flying and reach should be able to block attacker with flying
        let result = game.declare_blocker(p2_id, blocker_id, vec![attacker_id]);
        assert!(
            result.is_ok(),
            "Creature with flying and reach should be able to block creature with flying"
        );
    }

    #[test]
    fn test_first_strike_creature_kills_before_taking_damage() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1: Load Advance Scout (1/1 with First Strike) as attacker
        let attacker_id = load_test_card(&mut game, "Advance Scout", p1_id).expect("Failed to load Advance Scout");

        // Set attacker power/toughness to 2/2 so test works as before
        if let Ok(attacker) = game.cards.get_mut(attacker_id) {
            attacker.power = Some(2);
            attacker.toughness = Some(2);
            attacker.controller = p1_id;
            attacker.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        }
        game.battlefield.add(attacker_id);

        // P2: Load Grizzly Bears (2/2 vanilla) as blocker
        let blocker_id = load_test_card(&mut game, "Grizzly Bears", p2_id).expect("Failed to load Grizzly Bears");

        if let Ok(blocker) = game.cards.get_mut(blocker_id) {
            blocker.controller = p2_id;
        }
        game.battlefield.add(blocker_id);

        // Declare combat
        game.combat.declare_attacker(attacker_id, p2_id);
        let attacker_vec = smallvec::SmallVec::from_vec(vec![attacker_id]);
        game.combat.declare_blocker(blocker_id, attacker_vec);

        // Create controllers
        let mut controller1 = ZeroController::new(p1_id);
        let mut controller2 = ZeroController::new(p2_id);

        // First strike damage step: attacker deals 2 damage, blocker takes none
        let result = game.assign_combat_damage(&mut controller1, &mut controller2, true);
        assert!(result.is_ok(), "Failed to assign first strike damage: {result:?}");

        // Blocker should be dead (took 2 damage, toughness 2)
        if let Some(zones) = game.get_player_zones(p2_id) {
            assert!(
                zones.graveyard.contains(blocker_id),
                "Blocker should be in graveyard after first strike damage"
            );
        }

        // Normal damage step: only attacker can deal damage (blocker is dead)
        let result = game.assign_combat_damage(&mut controller1, &mut controller2, false);
        assert!(result.is_ok(), "Failed to assign normal damage: {result:?}");

        // Attacker should still be alive (never took damage)
        assert!(game.battlefield.contains(attacker_id), "Attacker should still be alive");

        // Check attacker is undamaged
        if let Ok(attacker) = game.cards.get(attacker_id) {
            assert_eq!(attacker.current_toughness(), 2, "Attacker should be undamaged");
        }
    }

    #[test]
    fn test_double_strike_creature_deals_damage_twice() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1: Load Adorned Pouncer (1/1 with Double Strike) as attacker
        let attacker_id = load_test_card(&mut game, "Adorned Pouncer", p1_id).expect("Failed to load Adorned Pouncer");

        // Set power/toughness to 3/3 so test works as before
        if let Ok(attacker) = game.cards.get_mut(attacker_id) {
            attacker.power = Some(3);
            attacker.toughness = Some(3);
            attacker.controller = p1_id;
            attacker.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        }
        game.battlefield.add(attacker_id);

        // Declare unblocked attacker
        game.combat.declare_attacker(attacker_id, p2_id);

        // Create controllers
        let mut controller1 = ZeroController::new(p1_id);
        let mut controller2 = ZeroController::new(p2_id);

        // First strike damage step: attacker deals 3 damage to player
        let result = game.assign_combat_damage(&mut controller1, &mut controller2, true);
        assert!(result.is_ok(), "Failed to assign first strike damage: {result:?}");

        // Check player took 3 damage
        let p2 = game.get_player(p2_id).unwrap();
        assert_eq!(p2.life, 17, "Player should have taken 3 damage in first strike step");

        // Normal damage step: attacker deals another 3 damage to player
        let result = game.assign_combat_damage(&mut controller1, &mut controller2, false);
        assert!(result.is_ok(), "Failed to assign normal damage: {result:?}");

        // Check player took another 3 damage (total 6)
        let p2 = game.get_player(p2_id).unwrap();
        assert_eq!(
            p2.life, 14,
            "Player should have taken 6 total damage from double strike"
        );
    }

    #[test]
    fn test_double_strike_vs_first_strike() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1: Create a 2/2 creature with Double Strike (attacker)
        let attacker_id = game.next_entity_id();
        let mut attacker = Card::new(attacker_id, "Double Strike Knight".to_string(), p1_id);
        attacker.types.push(CardType::Creature);
        attacker.power = Some(2);
        attacker.toughness = Some(2);
        attacker.controller = p1_id;
        attacker.keywords.push(Keyword::DoubleStrike);
        attacker.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        game.cards.insert(attacker_id, attacker);
        game.battlefield.add(attacker_id);

        // P2: Create a 2/2 creature with First Strike (blocker)
        let blocker_id = game.next_entity_id();
        let mut blocker = Card::new(blocker_id, "First Strike Soldier".to_string(), p2_id);
        blocker.types.push(CardType::Creature);
        blocker.power = Some(2);
        blocker.toughness = Some(2);
        blocker.controller = p2_id;
        blocker.keywords.push(Keyword::FirstStrike);
        game.cards.insert(blocker_id, blocker);
        game.battlefield.add(blocker_id);

        // Declare combat
        game.combat.declare_attacker(attacker_id, p2_id);
        let attacker_vec = smallvec::SmallVec::from_vec(vec![attacker_id]);
        game.combat.declare_blocker(blocker_id, attacker_vec);

        // Create controllers
        let mut controller1 = ZeroController::new(p1_id);
        let mut controller2 = ZeroController::new(p2_id);

        // First strike damage step: both creatures deal damage simultaneously
        let result = game.assign_combat_damage(&mut controller1, &mut controller2, true);
        assert!(result.is_ok(), "Failed to assign first strike damage: {result:?}");

        // Both creatures should be dead (both took 2 damage, both have 2 toughness)
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert!(
                zones.graveyard.contains(attacker_id),
                "Double strike attacker should be in graveyard"
            );
        }
        if let Some(zones) = game.get_player_zones(p2_id) {
            assert!(
                zones.graveyard.contains(blocker_id),
                "First strike blocker should be in graveyard"
            );
        }

        // Normal damage step: no creatures left to deal damage
        let result = game.assign_combat_damage(&mut controller1, &mut controller2, false);
        assert!(result.is_ok(), "Failed to assign normal damage: {result:?}");

        // Both creatures should still be in graveyards
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert!(zones.graveyard.contains(attacker_id));
        }
        if let Some(zones) = game.get_player_zones(p2_id) {
            assert!(zones.graveyard.contains(blocker_id));
        }
    }

    #[test]
    fn test_resolve_pump_spell() {
        use crate::core::{Effect, ManaCost};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let players: Vec<_> = game.players.iter().map(|p| p.id).collect();
        let p1_id = players[0];

        // Create a 2/2 creature on battlefield
        let creature_id = game.next_card_id();
        let mut creature = Card::new(creature_id, "Grizzly Bears".to_string(), p1_id);
        creature.types.push(CardType::Creature);
        creature.power = Some(2);
        creature.toughness = Some(2);
        creature.controller = p1_id;
        game.cards.insert(creature_id, creature);
        game.battlefield.add(creature_id);

        // Check initial stats
        let creature_before = game.cards.get(creature_id).unwrap();
        assert_eq!(creature_before.current_power(), 2);
        assert_eq!(creature_before.current_toughness(), 2);

        // Create Giant Growth (pump +3/+3)
        let pump_spell_id = game.next_card_id();
        let mut pump_spell = Card::new(pump_spell_id, "Giant Growth".to_string(), p1_id);
        pump_spell.types.push(CardType::Instant);
        pump_spell.mana_cost = ManaCost::from_string("G");
        // Target the creature we created
        pump_spell.effects.push(Effect::PumpCreature {
            target: creature_id,
            power_bonus: 3,
            toughness_bonus: 3,
        });
        game.cards.insert(pump_spell_id, pump_spell);

        // Put spell on stack (simulating cast)
        game.stack.add(pump_spell_id);

        // Resolve the spell
        assert!(
            game.resolve_spell(pump_spell_id, &[]).is_ok(),
            "Failed to resolve pump spell"
        );

        // Check creature got the bonus
        let creature_after = game.cards.get(creature_id).unwrap();
        assert_eq!(
            creature_after.current_power(),
            5,
            "Creature should have +3 power bonus (2 + 3)"
        );
        assert_eq!(
            creature_after.current_toughness(),
            5,
            "Creature should have +3 toughness bonus (2 + 3)"
        );

        // Check spell went to graveyard
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert!(
                zones.graveyard.contains(pump_spell_id),
                "Pump spell should be in graveyard"
            );
        }
    }

    #[test]
    fn test_pump_effect_cleanup_at_end_of_turn() {
        use crate::core::CardType;
        use crate::game::Step;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Create a 2/2 creature on battlefield
        let creature_id = game.next_card_id();
        let mut creature = Card::new(creature_id, "Grizzly Bears".to_string(), p1_id);
        creature.types.push(CardType::Creature);
        creature.power = Some(2);
        creature.toughness = Some(2);
        creature.controller = p1_id;
        game.cards.insert(creature_id, creature);
        game.battlefield.add(creature_id);

        // Apply pump effect manually
        if let Ok(card) = game.cards.get_mut(creature_id) {
            card.power_bonus = 3;
            card.toughness_bonus = 3;
        }

        // Verify pumped stats
        let creature_pumped = game.cards.get(creature_id).unwrap();
        assert_eq!(creature_pumped.current_power(), 5);
        assert_eq!(creature_pumped.current_toughness(), 5);

        // Advance to End step
        game.turn.current_step = Step::End;

        // Advance to Cleanup step (should trigger cleanup)
        assert!(game.advance_step().is_ok());
        assert_eq!(game.turn.current_step, Step::Cleanup);

        // Check that bonuses were cleared
        let creature_after = game.cards.get(creature_id).unwrap();
        assert_eq!(
            creature_after.current_power(),
            2,
            "Power bonus should be cleared at cleanup"
        );
        assert_eq!(
            creature_after.current_toughness(),
            2,
            "Toughness bonus should be cleared at cleanup"
        );
    }

    #[test]
    fn test_normal_creature_vs_first_strike() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1: Create a 3/3 creature without first strike (attacker)
        let attacker_id = game.next_entity_id();
        let mut attacker = Card::new(attacker_id, "Hill Giant".to_string(), p1_id);
        attacker.types.push(CardType::Creature);
        attacker.power = Some(3);
        attacker.toughness = Some(3);
        attacker.controller = p1_id;
        attacker.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        game.cards.insert(attacker_id, attacker);
        game.battlefield.add(attacker_id);

        // P2: Create a 2/2 creature with First Strike (blocker)
        let blocker_id = game.next_entity_id();
        let mut blocker = Card::new(blocker_id, "First Strike Knight".to_string(), p2_id);
        blocker.types.push(CardType::Creature);
        blocker.power = Some(2);
        blocker.toughness = Some(2);
        blocker.controller = p2_id;
        blocker.keywords.push(Keyword::FirstStrike);
        game.cards.insert(blocker_id, blocker);
        game.battlefield.add(blocker_id);

        // Declare combat
        game.combat.declare_attacker(attacker_id, p2_id);
        let attacker_vec = smallvec::SmallVec::from_vec(vec![attacker_id]);
        game.combat.declare_blocker(blocker_id, attacker_vec);

        // Create controllers
        let mut controller1 = ZeroController::new(p1_id);
        let mut controller2 = ZeroController::new(p2_id);

        // First strike damage step: only blocker deals damage
        let result = game.assign_combat_damage(&mut controller1, &mut controller2, true);
        assert!(result.is_ok(), "Failed to assign first strike damage: {result:?}");

        // Attacker should have taken 2 damage but still be alive (3 toughness)
        assert!(
            game.battlefield.contains(attacker_id),
            "Attacker should still be alive after first strike"
        );

        // Blocker should still be alive (hasn't taken damage yet)
        assert!(game.battlefield.contains(blocker_id), "Blocker should still be alive");

        // Normal damage step: attacker deals damage, killing blocker
        let result = game.assign_combat_damage(&mut controller1, &mut controller2, false);
        assert!(result.is_ok(), "Failed to assign normal damage: {result:?}");

        // Blocker should be dead (took 3 damage, toughness 2)
        if let Some(zones) = game.get_player_zones(p2_id) {
            assert!(
                zones.graveyard.contains(blocker_id),
                "Blocker should be in graveyard after normal damage"
            );
        }

        // Attacker should still be alive (took only 2 damage, has 3 toughness)
        assert!(game.battlefield.contains(attacker_id), "Attacker should still be alive");
    }

    #[test]
    fn test_resolve_tap_spell() {
        use crate::core::{Effect, ManaCost};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let players: Vec<_> = game.players.iter().map(|p| p.id).collect();
        let p1_id = players[0];
        let p2_id = players[1];

        // Create an untapped creature for P2
        let creature_id = game.next_card_id();
        let mut creature = Card::new(creature_id, "Grizzly Bears".to_string(), p2_id);
        creature.types.push(CardType::Creature);
        creature.power = Some(2);
        creature.toughness = Some(2);
        creature.controller = p2_id;
        game.cards.insert(creature_id, creature);
        game.battlefield.add(creature_id);

        // Check initial state
        let creature_before = game.cards.get(creature_id).unwrap();
        assert!(!creature_before.tapped, "Creature should start untapped");

        // Create a Tap spell
        let tap_spell_id = game.next_card_id();
        let mut tap_spell = Card::new(tap_spell_id, "Frost Breath".to_string(), p1_id);
        tap_spell.types.push(CardType::Instant);
        tap_spell.mana_cost = ManaCost::from_string("2U");
        // Target the specific creature
        tap_spell.effects.push(Effect::TapPermanent { target: creature_id });
        game.cards.insert(tap_spell_id, tap_spell);

        // Put spell on stack (simulating cast)
        game.stack.add(tap_spell_id);

        // Resolve the spell
        assert!(
            game.resolve_spell(tap_spell_id, &[]).is_ok(),
            "Failed to resolve tap spell"
        );

        // Check creature is tapped
        let creature_after = game.cards.get(creature_id).unwrap();
        assert!(creature_after.tapped, "Creature should be tapped after spell");

        // Check spell went to graveyard
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert!(
                zones.graveyard.contains(tap_spell_id),
                "Tap spell should be in graveyard"
            );
        }
    }

    #[test]
    fn test_resolve_untap_spell() {
        use crate::core::{Effect, ManaCost};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let players: Vec<_> = game.players.iter().map(|p| p.id).collect();
        let p1_id = players[0];

        // Create a tapped land for P1
        let land_id = game.next_card_id();
        let mut land = Card::new(land_id, "Forest".to_string(), p1_id);
        land.types.push(CardType::Land);
        land.controller = p1_id;
        land.tapped = true; // Start tapped
        game.cards.insert(land_id, land);
        game.battlefield.add(land_id);

        // Check initial state
        let land_before = game.cards.get(land_id).unwrap();
        assert!(land_before.tapped, "Land should start tapped");

        // Create an Untap spell
        let untap_spell_id = game.next_card_id();
        let mut untap_spell = Card::new(untap_spell_id, "Untap".to_string(), p1_id);
        untap_spell.types.push(CardType::Instant);
        untap_spell.mana_cost = ManaCost::from_string("U");
        // Target the specific land
        untap_spell.effects.push(Effect::UntapPermanent { target: land_id });
        game.cards.insert(untap_spell_id, untap_spell);

        // Put spell on stack (simulating cast)
        game.stack.add(untap_spell_id);

        // Resolve the spell
        assert!(
            game.resolve_spell(untap_spell_id, &[]).is_ok(),
            "Failed to resolve untap spell"
        );

        // Check land is untapped
        let land_after = game.cards.get(land_id).unwrap();
        assert!(!land_after.tapped, "Land should be untapped after spell");

        // Check spell went to graveyard
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert!(
                zones.graveyard.contains(untap_spell_id),
                "Untap spell should be in graveyard"
            );
        }
    }

    #[test]
    fn test_trample_excess_damage_to_player() {
        use crate::game::random_controller::RandomController;
        use crate::game::zero_controller::ZeroController;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1: Create a 5/5 creature with Trample (attacker)
        let attacker_id = game.next_entity_id();
        let mut attacker = Card::new(attacker_id, "Craw Wurm".to_string(), p1_id);
        attacker.types.push(CardType::Creature);
        attacker.power = Some(5);
        attacker.toughness = Some(5);
        attacker.controller = p1_id;
        attacker.keywords.push(Keyword::Trample);
        attacker.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        game.cards.insert(attacker_id, attacker);
        game.battlefield.add(attacker_id);

        // P2: Create a 2/2 creature (blocker)
        let blocker_id = game.next_entity_id();
        let mut blocker = Card::new(blocker_id, "Grizzly Bears".to_string(), p2_id);
        blocker.types.push(CardType::Creature);
        blocker.power = Some(2);
        blocker.toughness = Some(2);
        blocker.controller = p2_id;
        game.cards.insert(blocker_id, blocker);
        game.battlefield.add(blocker_id);

        // Declare combat
        game.combat.declare_attacker(attacker_id, p2_id);
        let attacker_vec = smallvec::SmallVec::from_vec(vec![attacker_id]);
        game.combat.declare_blocker(blocker_id, attacker_vec);

        // Record P2's life before combat
        let p2_life_before = game.players[1].life;

        // Assign combat damage
        let mut controller1 = RandomController::with_seed(p1_id, 42);
        let mut controller2 = ZeroController::new(p2_id);
        let result = game.assign_combat_damage(&mut controller1, &mut controller2, false);
        assert!(result.is_ok(), "Failed to assign combat damage: {result:?}");

        // Blocker should be dead (took 5 damage, toughness 2)
        if let Some(zones) = game.get_player_zones(p2_id) {
            assert!(zones.graveyard.contains(blocker_id), "Blocker should be in graveyard");
        }

        // P2 should have taken 3 trample damage (5 power - 2 to kill blocker)
        let p2_life_after = game.players[1].life;
        assert_eq!(
            p2_life_after,
            p2_life_before - 3,
            "P2 should have taken 3 trample damage"
        );
    }

    #[test]
    fn test_trample_exact_lethal_no_excess() {
        use crate::game::random_controller::RandomController;
        use crate::game::zero_controller::ZeroController;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1: Create a 3/3 creature with Trample (attacker)
        let attacker_id = game.next_entity_id();
        let mut attacker = Card::new(attacker_id, "Trained Armodon".to_string(), p1_id);
        attacker.types.push(CardType::Creature);
        attacker.power = Some(3);
        attacker.toughness = Some(3);
        attacker.controller = p1_id;
        attacker.keywords.push(Keyword::Trample);
        attacker.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        game.cards.insert(attacker_id, attacker);
        game.battlefield.add(attacker_id);

        // P2: Create a 3/3 creature (blocker)
        let blocker_id = game.next_entity_id();
        let mut blocker = Card::new(blocker_id, "Hill Giant".to_string(), p2_id);
        blocker.types.push(CardType::Creature);
        blocker.power = Some(3);
        blocker.toughness = Some(3);
        blocker.controller = p2_id;
        game.cards.insert(blocker_id, blocker);
        game.battlefield.add(blocker_id);

        // Declare combat
        game.combat.declare_attacker(attacker_id, p2_id);
        let attacker_vec = smallvec::SmallVec::from_vec(vec![attacker_id]);
        game.combat.declare_blocker(blocker_id, attacker_vec);

        // Record P2's life before combat
        let p2_life_before = game.players[1].life;

        // Assign combat damage
        let mut controller1 = RandomController::with_seed(p1_id, 42);
        let mut controller2 = ZeroController::new(p2_id);
        let result = game.assign_combat_damage(&mut controller1, &mut controller2, false);
        assert!(result.is_ok(), "Failed to assign combat damage: {result:?}");

        // Blocker should be dead (took 3 damage, toughness 3)
        if let Some(zones) = game.get_player_zones(p2_id) {
            assert!(zones.graveyard.contains(blocker_id), "Blocker should be in graveyard");
        }

        // P2 should NOT have taken any damage (exact lethal, no excess)
        let p2_life_after = game.players[1].life;
        assert_eq!(
            p2_life_after, p2_life_before,
            "P2 should not have taken damage (exact lethal, no excess)"
        );
    }

    #[test]
    fn test_non_trample_blocked_no_player_damage() {
        use crate::game::random_controller::RandomController;
        use crate::game::zero_controller::ZeroController;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1: Create a 5/5 creature WITHOUT Trample (attacker)
        let attacker_id = game.next_entity_id();
        let mut attacker = Card::new(attacker_id, "Serra Angel".to_string(), p1_id);
        attacker.types.push(CardType::Creature);
        attacker.power = Some(5);
        attacker.toughness = Some(5);
        attacker.controller = p1_id;
        // NO Trample keyword
        attacker.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        game.cards.insert(attacker_id, attacker);
        game.battlefield.add(attacker_id);

        // P2: Create a 1/1 creature (blocker)
        let blocker_id = game.next_entity_id();
        let mut blocker = Card::new(blocker_id, "Llanowar Elves".to_string(), p2_id);
        blocker.types.push(CardType::Creature);
        blocker.power = Some(1);
        blocker.toughness = Some(1);
        blocker.controller = p2_id;
        game.cards.insert(blocker_id, blocker);
        game.battlefield.add(blocker_id);

        // Declare combat
        game.combat.declare_attacker(attacker_id, p2_id);
        let attacker_vec = smallvec::SmallVec::from_vec(vec![attacker_id]);
        game.combat.declare_blocker(blocker_id, attacker_vec);

        // Record P2's life before combat
        let p2_life_before = game.players[1].life;

        // Assign combat damage
        let mut controller1 = RandomController::with_seed(p1_id, 42);
        let mut controller2 = ZeroController::new(p2_id);
        let result = game.assign_combat_damage(&mut controller1, &mut controller2, false);
        assert!(result.is_ok(), "Failed to assign combat damage: {result:?}");

        // Blocker should be dead
        if let Some(zones) = game.get_player_zones(p2_id) {
            assert!(zones.graveyard.contains(blocker_id), "Blocker should be in graveyard");
        }

        // P2 should NOT have taken any damage (no trample, so excess is lost)
        let p2_life_after = game.players[1].life;
        assert_eq!(
            p2_life_after, p2_life_before,
            "P2 should not have taken damage without trample"
        );
    }

    #[test]
    fn test_trample_multiple_blockers() {
        use crate::game::random_controller::RandomController;
        use crate::game::zero_controller::ZeroController;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1: Create a 7/7 creature with Trample (attacker)
        let attacker_id = game.next_entity_id();
        let mut attacker = Card::new(attacker_id, "Enormous Baloth".to_string(), p1_id);
        attacker.types.push(CardType::Creature);
        attacker.power = Some(7);
        attacker.toughness = Some(7);
        attacker.controller = p1_id;
        attacker.keywords.push(Keyword::Trample);
        attacker.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        game.cards.insert(attacker_id, attacker);
        game.battlefield.add(attacker_id);

        // P2: Create two blockers (2/2 and 3/3)
        let blocker1_id = game.next_entity_id();
        let mut blocker1 = Card::new(blocker1_id, "Grizzly Bears".to_string(), p2_id);
        blocker1.types.push(CardType::Creature);
        blocker1.power = Some(2);
        blocker1.toughness = Some(2);
        blocker1.controller = p2_id;
        game.cards.insert(blocker1_id, blocker1);
        game.battlefield.add(blocker1_id);

        let blocker2_id = game.next_entity_id();
        let mut blocker2 = Card::new(blocker2_id, "Hill Giant".to_string(), p2_id);
        blocker2.types.push(CardType::Creature);
        blocker2.power = Some(3);
        blocker2.toughness = Some(3);
        blocker2.controller = p2_id;
        game.cards.insert(blocker2_id, blocker2);
        game.battlefield.add(blocker2_id);

        // Declare combat
        game.combat.declare_attacker(attacker_id, p2_id);
        let attacker_vec = smallvec::SmallVec::from_vec(vec![attacker_id]);
        game.combat.declare_blocker(blocker1_id, attacker_vec.clone());
        game.combat.declare_blocker(blocker2_id, attacker_vec);

        // Record P2's life before combat
        let p2_life_before = game.players[1].life;

        // Assign combat damage
        let mut controller1 = RandomController::with_seed(p1_id, 42);
        let mut controller2 = ZeroController::new(p2_id);
        let result = game.assign_combat_damage(&mut controller1, &mut controller2, false);
        assert!(result.is_ok(), "Failed to assign combat damage: {result:?}");

        // Both blockers should be dead
        if let Some(zones) = game.get_player_zones(p2_id) {
            assert!(
                zones.graveyard.contains(blocker1_id),
                "Blocker 1 should be in graveyard"
            );
            assert!(
                zones.graveyard.contains(blocker2_id),
                "Blocker 2 should be in graveyard"
            );
        }

        // P2 should have taken 2 trample damage (7 power - 2 - 3 = 2)
        let p2_life_after = game.players[1].life;
        assert_eq!(
            p2_life_after,
            p2_life_before - 2,
            "P2 should have taken 2 trample damage"
        );
    }

    #[test]
    fn test_lifelink_attacker_blocked() {
        use crate::game::random_controller::RandomController;
        use crate::game::zero_controller::ZeroController;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1: Create a 3/3 creature with Lifelink (attacker)
        let attacker_id = game.next_entity_id();
        let mut attacker = Card::new(attacker_id, "Healer's Hawk".to_string(), p1_id);
        attacker.types.push(CardType::Creature);
        attacker.power = Some(3);
        attacker.toughness = Some(3);
        attacker.controller = p1_id;
        attacker.keywords.push(Keyword::Lifelink);
        attacker.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        game.cards.insert(attacker_id, attacker);
        game.battlefield.add(attacker_id);

        // P2: Create a 2/2 creature (blocker)
        let blocker_id = game.next_entity_id();
        let mut blocker = Card::new(blocker_id, "Grizzly Bears".to_string(), p2_id);
        blocker.types.push(CardType::Creature);
        blocker.power = Some(2);
        blocker.toughness = Some(2);
        blocker.controller = p2_id;
        game.cards.insert(blocker_id, blocker);
        game.battlefield.add(blocker_id);

        // Declare combat
        game.combat.declare_attacker(attacker_id, p2_id);
        let attacker_vec = smallvec::SmallVec::from_vec(vec![attacker_id]);
        game.combat.declare_blocker(blocker_id, attacker_vec);

        // Record P1's life before combat
        let p1_life_before = game.players[0].life;

        // Assign combat damage
        let mut controller1 = RandomController::with_seed(p1_id, 42);
        let mut controller2 = ZeroController::new(p2_id);
        let result = game.assign_combat_damage(&mut controller1, &mut controller2, false);
        assert!(result.is_ok(), "Failed to assign combat damage: {result:?}");

        // P1 should have gained 3 life from lifelink (3 damage dealt to blocker)
        let p1_life_after = game.players[0].life;
        assert_eq!(
            p1_life_after,
            p1_life_before + 3,
            "P1 should have gained 3 life from lifelink"
        );

        // Blocker should be dead
        if let Some(zones) = game.get_player_zones(p2_id) {
            assert!(zones.graveyard.contains(blocker_id), "Blocker should be in graveyard");
        }
    }

    #[test]
    fn test_lifelink_attacker_unblocked() {
        use crate::game::random_controller::RandomController;
        use crate::game::zero_controller::ZeroController;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1: Create a 4/4 creature with Lifelink (attacker)
        let attacker_id = game.next_entity_id();
        let mut attacker = Card::new(attacker_id, "Ajani's Pridemate".to_string(), p1_id);
        attacker.types.push(CardType::Creature);
        attacker.power = Some(4);
        attacker.toughness = Some(4);
        attacker.controller = p1_id;
        attacker.keywords.push(Keyword::Lifelink);
        attacker.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        game.cards.insert(attacker_id, attacker);
        game.battlefield.add(attacker_id);

        // Declare combat (no blockers)
        game.combat.declare_attacker(attacker_id, p2_id);

        // Record life before combat
        let p1_life_before = game.players[0].life;
        let p2_life_before = game.players[1].life;

        // Assign combat damage
        let mut controller1 = RandomController::with_seed(p1_id, 42);
        let mut controller2 = ZeroController::new(p2_id);
        let result = game.assign_combat_damage(&mut controller1, &mut controller2, false);
        assert!(result.is_ok(), "Failed to assign combat damage: {result:?}");

        // P1 should have gained 4 life from lifelink (4 damage dealt to player)
        let p1_life_after = game.players[0].life;
        assert_eq!(
            p1_life_after,
            p1_life_before + 4,
            "P1 should have gained 4 life from lifelink"
        );

        // P2 should have taken 4 damage
        let p2_life_after = game.players[1].life;
        assert_eq!(p2_life_after, p2_life_before - 4, "P2 should have taken 4 damage");
    }

    #[test]
    fn test_lifelink_blocker() {
        use crate::game::random_controller::RandomController;
        use crate::game::zero_controller::ZeroController;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1: Create a 3/3 creature (attacker)
        let attacker_id = game.next_entity_id();
        let mut attacker = Card::new(attacker_id, "Hill Giant".to_string(), p1_id);
        attacker.types.push(CardType::Creature);
        attacker.power = Some(3);
        attacker.toughness = Some(3);
        attacker.controller = p1_id;
        attacker.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        game.cards.insert(attacker_id, attacker);
        game.battlefield.add(attacker_id);

        // P2: Create a 2/2 creature with Lifelink (blocker)
        let blocker_id = game.next_entity_id();
        let mut blocker = Card::new(blocker_id, "Vampire Cutthroat".to_string(), p2_id);
        blocker.types.push(CardType::Creature);
        blocker.power = Some(2);
        blocker.toughness = Some(2);
        blocker.controller = p2_id;
        blocker.keywords.push(Keyword::Lifelink);
        game.cards.insert(blocker_id, blocker);
        game.battlefield.add(blocker_id);

        // Declare combat
        game.combat.declare_attacker(attacker_id, p2_id);
        let attacker_vec = smallvec::SmallVec::from_vec(vec![attacker_id]);
        game.combat.declare_blocker(blocker_id, attacker_vec);

        // Record P2's life before combat
        let p2_life_before = game.players[1].life;

        // Assign combat damage
        let mut controller1 = RandomController::with_seed(p1_id, 42);
        let mut controller2 = ZeroController::new(p2_id);
        let result = game.assign_combat_damage(&mut controller1, &mut controller2, false);
        assert!(result.is_ok(), "Failed to assign combat damage: {result:?}");

        // P2 should have gained 2 life from lifelink (blocker dealt 2 damage)
        let p2_life_after = game.players[1].life;
        assert_eq!(
            p2_life_after,
            p2_life_before + 2,
            "P2 should have gained 2 life from lifelink blocker"
        );

        // Blocker should be dead (took 3 damage, has 2 toughness)
        // Attacker should survive (took 2 damage, has 3 toughness)
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert!(
                !zones.graveyard.contains(attacker_id),
                "Attacker should still be alive (took 2 damage, has 3 toughness)"
            );
            assert!(
                game.battlefield.contains(attacker_id),
                "Attacker should still be on battlefield"
            );
        }
        if let Some(zones) = game.get_player_zones(p2_id) {
            assert!(
                zones.graveyard.contains(blocker_id),
                "Blocker should be in graveyard (took 3 damage, has 2 toughness)"
            );
        }
    }

    #[test]
    fn test_lifelink_with_trample() {
        use crate::game::random_controller::RandomController;
        use crate::game::zero_controller::ZeroController;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1: Create a 5/5 creature with Lifelink AND Trample (attacker)
        let attacker_id = game.next_entity_id();
        let mut attacker = Card::new(attacker_id, "Baneslayer Angel".to_string(), p1_id);
        attacker.types.push(CardType::Creature);
        attacker.power = Some(5);
        attacker.toughness = Some(5);
        attacker.controller = p1_id;
        attacker.keywords.push(Keyword::Lifelink);
        attacker.keywords.push(Keyword::Trample);
        attacker.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        game.cards.insert(attacker_id, attacker);
        game.battlefield.add(attacker_id);

        // P2: Create a 2/2 creature (blocker)
        let blocker_id = game.next_entity_id();
        let mut blocker = Card::new(blocker_id, "Grizzly Bears".to_string(), p2_id);
        blocker.types.push(CardType::Creature);
        blocker.power = Some(2);
        blocker.toughness = Some(2);
        blocker.controller = p2_id;
        game.cards.insert(blocker_id, blocker);
        game.battlefield.add(blocker_id);

        // Declare combat
        game.combat.declare_attacker(attacker_id, p2_id);
        let attacker_vec = smallvec::SmallVec::from_vec(vec![attacker_id]);
        game.combat.declare_blocker(blocker_id, attacker_vec);

        // Record life before combat
        let p1_life_before = game.players[0].life;
        let p2_life_before = game.players[1].life;

        // Assign combat damage
        let mut controller1 = RandomController::with_seed(p1_id, 42);
        let mut controller2 = ZeroController::new(p2_id);
        let result = game.assign_combat_damage(&mut controller1, &mut controller2, false);
        assert!(result.is_ok(), "Failed to assign combat damage: {result:?}");

        // P1 should have gained 5 life (2 to blocker + 3 trample to player = 5 total damage)
        let p1_life_after = game.players[0].life;
        assert_eq!(
            p1_life_after,
            p1_life_before + 5,
            "P1 should have gained 5 life from lifelink (all damage dealt)"
        );

        // P2 should have taken 3 trample damage
        let p2_life_after = game.players[1].life;
        assert_eq!(
            p2_life_after,
            p2_life_before - 3,
            "P2 should have taken 3 trample damage"
        );

        // Blocker should be dead
        if let Some(zones) = game.get_player_zones(p2_id) {
            assert!(zones.graveyard.contains(blocker_id), "Blocker should be in graveyard");
        }
    }

    #[test]
    fn test_deathtouch_attacker_kills_large_blocker() {
        use crate::game::random_controller::RandomController;
        use crate::game::zero_controller::ZeroController;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1: Create a 1/1 creature with Deathtouch (attacker)
        let attacker_id = game.next_entity_id();
        let mut attacker = Card::new(attacker_id, "Deadly Recluse".to_string(), p1_id);
        attacker.types.push(CardType::Creature);
        attacker.power = Some(1);
        attacker.toughness = Some(1);
        attacker.controller = p1_id;
        attacker.keywords.push(Keyword::Deathtouch);
        attacker.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        game.cards.insert(attacker_id, attacker);
        game.battlefield.add(attacker_id);

        // P2: Create a 5/5 creature (blocker)
        let blocker_id = game.next_entity_id();
        let mut blocker = Card::new(blocker_id, "Serra Angel".to_string(), p2_id);
        blocker.types.push(CardType::Creature);
        blocker.power = Some(5);
        blocker.toughness = Some(5);
        blocker.controller = p2_id;
        game.cards.insert(blocker_id, blocker);
        game.battlefield.add(blocker_id);

        // Declare combat
        game.combat.declare_attacker(attacker_id, p2_id);
        let attacker_vec = smallvec::SmallVec::from_vec(vec![attacker_id]);
        game.combat.declare_blocker(blocker_id, attacker_vec);

        // Assign combat damage
        let mut controller1 = RandomController::with_seed(p1_id, 42);
        let mut controller2 = ZeroController::new(p2_id);
        let result = game.assign_combat_damage(&mut controller1, &mut controller2, false);
        assert!(result.is_ok(), "Failed to assign combat damage: {result:?}");

        // Blocker should be dead (deathtouch from 1 damage)
        // Attacker should be dead (5 damage from blocker)
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert!(
                zones.graveyard.contains(attacker_id),
                "Attacker should be in graveyard (took 5 damage)"
            );
        }
        if let Some(zones) = game.get_player_zones(p2_id) {
            assert!(
                zones.graveyard.contains(blocker_id),
                "Blocker should be in graveyard (dealt deathtouch damage)"
            );
        }
    }

    #[test]
    fn test_deathtouch_blocker_kills_large_attacker() {
        use crate::game::random_controller::RandomController;
        use crate::game::zero_controller::ZeroController;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1: Create a 5/5 creature (attacker)
        let attacker_id = game.next_entity_id();
        let mut attacker = Card::new(attacker_id, "Serra Angel".to_string(), p1_id);
        attacker.types.push(CardType::Creature);
        attacker.power = Some(5);
        attacker.toughness = Some(5);
        attacker.controller = p1_id;
        attacker.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        game.cards.insert(attacker_id, attacker);
        game.battlefield.add(attacker_id);

        // P2: Create a 1/1 creature with Deathtouch (blocker)
        let blocker_id = game.next_entity_id();
        let mut blocker = Card::new(blocker_id, "Typhoid Rats".to_string(), p2_id);
        blocker.types.push(CardType::Creature);
        blocker.power = Some(1);
        blocker.toughness = Some(1);
        blocker.controller = p2_id;
        blocker.keywords.push(Keyword::Deathtouch);
        game.cards.insert(blocker_id, blocker);
        game.battlefield.add(blocker_id);

        // Declare combat
        game.combat.declare_attacker(attacker_id, p2_id);
        let attacker_vec = smallvec::SmallVec::from_vec(vec![attacker_id]);
        game.combat.declare_blocker(blocker_id, attacker_vec);

        // Assign combat damage
        let mut controller1 = RandomController::with_seed(p1_id, 42);
        let mut controller2 = ZeroController::new(p2_id);
        let result = game.assign_combat_damage(&mut controller1, &mut controller2, false);
        assert!(result.is_ok(), "Failed to assign combat damage: {result:?}");

        // Attacker should be dead (deathtouch from 1 damage)
        // Blocker should be dead (5 damage from attacker)
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert!(
                zones.graveyard.contains(attacker_id),
                "Attacker should be in graveyard (dealt deathtouch damage)"
            );
        }
        if let Some(zones) = game.get_player_zones(p2_id) {
            assert!(
                zones.graveyard.contains(blocker_id),
                "Blocker should be in graveyard (took 5 damage)"
            );
        }
    }

    #[test]
    fn test_deathtouch_with_trample_minimal_damage() {
        use crate::game::random_controller::RandomController;
        use crate::game::zero_controller::ZeroController;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1: Create a 5/5 creature with Deathtouch AND Trample (attacker)
        let attacker_id = game.next_entity_id();
        let mut attacker = Card::new(attacker_id, "Chevill, Bane of Monsters".to_string(), p1_id);
        attacker.types.push(CardType::Creature);
        attacker.power = Some(5);
        attacker.toughness = Some(5);
        attacker.controller = p1_id;
        attacker.keywords.push(Keyword::Deathtouch);
        attacker.keywords.push(Keyword::Trample);
        attacker.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        game.cards.insert(attacker_id, attacker);
        game.battlefield.add(attacker_id);

        // P2: Create a 3/3 creature (blocker)
        let blocker_id = game.next_entity_id();
        let mut blocker = Card::new(blocker_id, "Hill Giant".to_string(), p2_id);
        blocker.types.push(CardType::Creature);
        blocker.power = Some(3);
        blocker.toughness = Some(3);
        blocker.controller = p2_id;
        game.cards.insert(blocker_id, blocker);
        game.battlefield.add(blocker_id);

        // Declare combat
        game.combat.declare_attacker(attacker_id, p2_id);
        let attacker_vec = smallvec::SmallVec::from_vec(vec![attacker_id]);
        game.combat.declare_blocker(blocker_id, attacker_vec);

        // Record P2's life before combat
        let p2_life_before = game.players[1].life;

        // Assign combat damage
        let mut controller1 = RandomController::with_seed(p1_id, 42);
        let mut controller2 = ZeroController::new(p2_id);
        let result = game.assign_combat_damage(&mut controller1, &mut controller2, false);
        assert!(result.is_ok(), "Failed to assign combat damage: {result:?}");

        // MTG Rules 702.2c: With deathtouch + trample, only 1 damage is lethal
        // So 1 damage to blocker (kills it), 4 damage tramples over to player
        let p2_life_after = game.players[1].life;
        assert_eq!(
            p2_life_after,
            p2_life_before - 4,
            "P2 should have taken 4 trample damage (5 power - 1 lethal to blocker)"
        );

        // Blocker should be dead (deathtouch)
        // Attacker should survive (took 3 damage, has 5 toughness)
        assert!(
            game.battlefield.contains(attacker_id),
            "Attacker should survive (took 3 damage, has 5 toughness)"
        );
        if let Some(zones) = game.get_player_zones(p2_id) {
            assert!(
                zones.graveyard.contains(blocker_id),
                "Blocker should be in graveyard (dealt deathtouch damage)"
            );
        }
    }

    #[test]
    fn test_deathtouch_with_multiple_blockers() {
        use crate::game::random_controller::RandomController;
        use crate::game::zero_controller::ZeroController;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1: Create a 3/3 creature with Deathtouch (attacker)
        let attacker_id = game.next_entity_id();
        let mut attacker = Card::new(attacker_id, "Gifted Aetherborn".to_string(), p1_id);
        attacker.types.push(CardType::Creature);
        attacker.power = Some(3);
        attacker.toughness = Some(3);
        attacker.controller = p1_id;
        attacker.keywords.push(Keyword::Deathtouch);
        attacker.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        game.cards.insert(attacker_id, attacker);
        game.battlefield.add(attacker_id);

        // P2: Create two blockers (both 5/5)
        let blocker1_id = game.next_entity_id();
        let mut blocker1 = Card::new(blocker1_id, "Serra Angel".to_string(), p2_id);
        blocker1.types.push(CardType::Creature);
        blocker1.power = Some(5);
        blocker1.toughness = Some(5);
        blocker1.controller = p2_id;
        game.cards.insert(blocker1_id, blocker1);
        game.battlefield.add(blocker1_id);

        let blocker2_id = game.next_entity_id();
        let mut blocker2 = Card::new(blocker2_id, "Air Elemental".to_string(), p2_id);
        blocker2.types.push(CardType::Creature);
        blocker2.power = Some(5);
        blocker2.toughness = Some(5);
        blocker2.controller = p2_id;
        game.cards.insert(blocker2_id, blocker2);
        game.battlefield.add(blocker2_id);

        // Declare combat with both blockers
        game.combat.declare_attacker(attacker_id, p2_id);
        let attacker_vec = smallvec::SmallVec::from_vec(vec![attacker_id]);
        game.combat.declare_blocker(blocker1_id, attacker_vec.clone());
        game.combat.declare_blocker(blocker2_id, attacker_vec);

        // Assign combat damage (damage order determined internally)
        let mut controller1 = RandomController::with_seed(p1_id, 42);
        let mut controller2 = ZeroController::new(p2_id);
        let result = game.assign_combat_damage(&mut controller1, &mut controller2, false);
        assert!(result.is_ok(), "Failed to assign combat damage: {result:?}");

        // With deathtouch, 1 damage is lethal to each blocker
        // 3/3 attacker: 1 damage to first blocker, 1 damage to second blocker, 1 damage wasted
        // Both blockers should be dead, attacker should be dead (took 10 damage total)
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert!(
                zones.graveyard.contains(attacker_id),
                "Attacker should be in graveyard (took 10 damage from two 5/5 blockers)"
            );
        }
        if let Some(zones) = game.get_player_zones(p2_id) {
            assert!(
                zones.graveyard.contains(blocker1_id),
                "First blocker should be in graveyard (dealt deathtouch damage)"
            );
            assert!(
                zones.graveyard.contains(blocker2_id),
                "Second blocker should be in graveyard (dealt deathtouch damage)"
            );
        }
    }

    // Note: Menace validation test removed because incremental validation during
    // blocker declaration would incorrectly reject the first blocker. Menace validation
    // should happen after all blockers are declared. The following tests verify that
    // Menace works correctly when multiple blockers are declared or no blockers are declared.

    #[test]
    fn test_menace_can_be_blocked_by_two_creatures() {
        use crate::game::random_controller::RandomController;
        use crate::game::zero_controller::ZeroController;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1: Create a 3/3 creature with Menace (attacker)
        let attacker_id = game.next_entity_id();
        let mut attacker = Card::new(attacker_id, "Mardu Skullhunter".to_string(), p1_id);
        attacker.types.push(CardType::Creature);
        attacker.power = Some(3);
        attacker.toughness = Some(3);
        attacker.controller = p1_id;
        attacker.keywords.push(Keyword::Menace);
        attacker.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        game.cards.insert(attacker_id, attacker);
        game.battlefield.add(attacker_id);

        // P2: Create two blockers
        let blocker1_id = game.next_entity_id();
        let mut blocker1 = Card::new(blocker1_id, "Grizzly Bears".to_string(), p2_id);
        blocker1.types.push(CardType::Creature);
        blocker1.power = Some(2);
        blocker1.toughness = Some(2);
        blocker1.controller = p2_id;
        game.cards.insert(blocker1_id, blocker1);
        game.battlefield.add(blocker1_id);

        let blocker2_id = game.next_entity_id();
        let mut blocker2 = Card::new(blocker2_id, "Elite Vanguard".to_string(), p2_id);
        blocker2.types.push(CardType::Creature);
        blocker2.power = Some(2);
        blocker2.toughness = Some(1);
        blocker2.controller = p2_id;
        game.cards.insert(blocker2_id, blocker2);
        game.battlefield.add(blocker2_id);

        // Declare attacker
        game.combat.declare_attacker(attacker_id, p2_id);

        // Block with two creatures - should succeed
        let result1 = game.declare_blocker(p2_id, blocker1_id, vec![attacker_id]);
        assert!(result1.is_ok(), "First blocker should succeed: {result1:?}");

        let result2 = game.declare_blocker(p2_id, blocker2_id, vec![attacker_id]);
        assert!(result2.is_ok(), "Second blocker should succeed: {result2:?}");

        // Verify combat resolves correctly
        let mut controller1 = RandomController::with_seed(p1_id, 42);
        let mut controller2 = ZeroController::new(p2_id);
        let result = game.assign_combat_damage(&mut controller1, &mut controller2, false);
        assert!(result.is_ok(), "Combat damage should resolve: {result:?}");

        // Both blockers should be dead (took 3 damage total, both have <= 2 toughness)
        // Attacker should be dead (took 4 damage total from 2+2, has 3 toughness)
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert!(zones.graveyard.contains(attacker_id), "Attacker should be dead");
        }
        if let Some(zones) = game.get_player_zones(p2_id) {
            assert!(zones.graveyard.contains(blocker1_id), "First blocker should be dead");
            assert!(zones.graveyard.contains(blocker2_id), "Second blocker should be dead");
        }
    }

    #[test]
    fn test_menace_can_be_unblocked() {
        use crate::game::random_controller::RandomController;
        use crate::game::zero_controller::ZeroController;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1: Create a 3/3 creature with Menace (attacker)
        let attacker_id = game.next_entity_id();
        let mut attacker = Card::new(attacker_id, "Goblin Heelcutter".to_string(), p1_id);
        attacker.types.push(CardType::Creature);
        attacker.power = Some(3);
        attacker.toughness = Some(3);
        attacker.controller = p1_id;
        attacker.keywords.push(Keyword::Menace);
        attacker.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        game.cards.insert(attacker_id, attacker);
        game.battlefield.add(attacker_id);

        // Declare attacker (no blockers)
        game.combat.declare_attacker(attacker_id, p2_id);

        // Record life before combat
        let p2_life_before = game.players[1].life;

        // Assign combat damage
        let mut controller1 = RandomController::with_seed(p1_id, 42);
        let mut controller2 = ZeroController::new(p2_id);
        let result = game.assign_combat_damage(&mut controller1, &mut controller2, false);
        assert!(result.is_ok(), "Combat damage should resolve: {result:?}");

        // P2 should have taken 3 damage
        let p2_life_after = game.players[1].life;
        assert_eq!(
            p2_life_after,
            p2_life_before - 3,
            "P2 should have taken 3 damage from unblocked menace creature"
        );
    }

    #[test]
    fn test_menace_with_three_blockers() {
        use crate::game::random_controller::RandomController;
        use crate::game::zero_controller::ZeroController;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1: Create a 5/5 creature with Menace (attacker)
        let attacker_id = game.next_entity_id();
        let mut attacker = Card::new(attacker_id, "Charging Monstrosaur".to_string(), p1_id);
        attacker.types.push(CardType::Creature);
        attacker.power = Some(5);
        attacker.toughness = Some(5);
        attacker.controller = p1_id;
        attacker.keywords.push(Keyword::Menace);
        attacker.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        game.cards.insert(attacker_id, attacker);
        game.battlefield.add(attacker_id);

        // P2: Create three blockers (1/1 each)
        let blocker1_id = game.next_entity_id();
        let mut blocker1 = Card::new(blocker1_id, "Soldier Token 1".to_string(), p2_id);
        blocker1.types.push(CardType::Creature);
        blocker1.power = Some(1);
        blocker1.toughness = Some(1);
        blocker1.controller = p2_id;
        game.cards.insert(blocker1_id, blocker1);
        game.battlefield.add(blocker1_id);

        let blocker2_id = game.next_entity_id();
        let mut blocker2 = Card::new(blocker2_id, "Soldier Token 2".to_string(), p2_id);
        blocker2.types.push(CardType::Creature);
        blocker2.power = Some(1);
        blocker2.toughness = Some(1);
        blocker2.controller = p2_id;
        game.cards.insert(blocker2_id, blocker2);
        game.battlefield.add(blocker2_id);

        let blocker3_id = game.next_entity_id();
        let mut blocker3 = Card::new(blocker3_id, "Soldier Token 3".to_string(), p2_id);
        blocker3.types.push(CardType::Creature);
        blocker3.power = Some(1);
        blocker3.toughness = Some(1);
        blocker3.controller = p2_id;
        game.cards.insert(blocker3_id, blocker3);
        game.battlefield.add(blocker3_id);

        // Declare attacker
        game.combat.declare_attacker(attacker_id, p2_id);

        // Block with three creatures - should succeed (more than 2 is fine)
        let result1 = game.declare_blocker(p2_id, blocker1_id, vec![attacker_id]);
        assert!(result1.is_ok(), "First blocker should succeed");

        let result2 = game.declare_blocker(p2_id, blocker2_id, vec![attacker_id]);
        assert!(result2.is_ok(), "Second blocker should succeed");

        let result3 = game.declare_blocker(p2_id, blocker3_id, vec![attacker_id]);
        assert!(result3.is_ok(), "Third blocker should succeed");

        // Verify combat resolves correctly
        let mut controller1 = RandomController::with_seed(p1_id, 42);
        let mut controller2 = ZeroController::new(p2_id);
        let result = game.assign_combat_damage(&mut controller1, &mut controller2, false);
        assert!(result.is_ok(), "Combat damage should resolve: {result:?}");

        // All three blockers should be dead (each took 1 toughness worth of damage)
        // Attacker should survive (took 3 damage, has 5 toughness)
        assert!(
            game.battlefield.contains(attacker_id),
            "Attacker should survive (took 3 damage, has 5 toughness)"
        );
        if let Some(zones) = game.get_player_zones(p2_id) {
            assert!(zones.graveyard.contains(blocker1_id), "First blocker should be dead");
            assert!(zones.graveyard.contains(blocker2_id), "Second blocker should be dead");
            assert!(zones.graveyard.contains(blocker3_id), "Third blocker should be dead");
        }
    }

    #[test]
    fn test_hexproof_blocks_destroy_spell() {
        // Test that destroy spells cannot target hexproof creatures controlled by opponent
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P2: Create a hexproof creature
        let hexproof_creature_id = game.next_entity_id();
        let mut hexproof_creature = Card::new(hexproof_creature_id, "Slippery Bogle".to_string(), p2_id);
        hexproof_creature.types.push(CardType::Creature);
        hexproof_creature.power = Some(1);
        hexproof_creature.toughness = Some(1);
        hexproof_creature.keywords.push(Keyword::Hexproof);
        game.cards.insert(hexproof_creature_id, hexproof_creature);
        game.battlefield.add(hexproof_creature_id);

        // P2: Create a normal creature
        let normal_creature_id = game.next_entity_id();
        let mut normal_creature = Card::new(normal_creature_id, "Grizzly Bears".to_string(), p2_id);
        normal_creature.types.push(CardType::Creature);
        normal_creature.power = Some(2);
        normal_creature.toughness = Some(2);
        game.cards.insert(normal_creature_id, normal_creature);
        game.battlefield.add(normal_creature_id);

        // P1: Cast a destroy spell (Terror) - should target normal creature, not hexproof one
        let destroy_spell_id = game.next_entity_id();
        let mut destroy_spell = Card::new(destroy_spell_id, "Terror".to_string(), p1_id);
        destroy_spell.types.push(CardType::Instant);
        destroy_spell.mana_cost = ManaCost::from_string("1B");
        // Use placeholder card ID 0 which will be replaced with a targetable opponent's creature
        destroy_spell
            .effects
            .push(Effect::DestroyPermanent { target: CardId::new(0) });
        game.cards.insert(destroy_spell_id, destroy_spell);

        // Put it on the stack (simulating cast)
        game.stack.add(destroy_spell_id);

        // Resolve the spell - explicitly target the normal creature
        // (controller would have chosen normal_creature_id, not hexproof one)
        let result = game.resolve_spell(destroy_spell_id, &[normal_creature_id]);
        assert!(result.is_ok(), "Destroy spell should resolve successfully");

        // Check that the hexproof creature is still alive
        assert!(
            game.battlefield.contains(hexproof_creature_id),
            "Hexproof creature should still be on battlefield"
        );

        // Check that the normal creature was destroyed
        if let Some(zones) = game.get_player_zones(p2_id) {
            assert!(
                zones.graveyard.contains(normal_creature_id),
                "Normal creature should be in graveyard"
            );
        }
    }

    #[test]
    fn test_hexproof_blocks_tap_spell() {
        // Test that tap spells cannot target hexproof creatures controlled by opponent
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P2: Create a hexproof creature
        let hexproof_creature_id = game.next_entity_id();
        let mut hexproof_creature = Card::new(hexproof_creature_id, "Slippery Bogle".to_string(), p2_id);
        hexproof_creature.types.push(CardType::Creature);
        hexproof_creature.power = Some(1);
        hexproof_creature.toughness = Some(1);
        hexproof_creature.keywords.push(Keyword::Hexproof);
        game.cards.insert(hexproof_creature_id, hexproof_creature);
        game.battlefield.add(hexproof_creature_id);

        // P2: Create a normal creature
        let normal_creature_id = game.next_entity_id();
        let mut normal_creature = Card::new(normal_creature_id, "Grizzly Bears".to_string(), p2_id);
        normal_creature.types.push(CardType::Creature);
        normal_creature.power = Some(2);
        normal_creature.toughness = Some(2);
        game.cards.insert(normal_creature_id, normal_creature);
        game.battlefield.add(normal_creature_id);

        // P1: Cast a tap spell - should target normal creature, not hexproof one
        let tap_spell_id = game.next_entity_id();
        let mut tap_spell = Card::new(tap_spell_id, "Frost Breath".to_string(), p1_id);
        tap_spell.types.push(CardType::Instant);
        tap_spell.mana_cost = ManaCost::from_string("2U");
        // Use placeholder card ID 0 which will be replaced with a targetable opponent's creature
        tap_spell.effects.push(Effect::TapPermanent { target: CardId::new(0) });
        game.cards.insert(tap_spell_id, tap_spell);

        // Put spell on stack (simulating cast)
        game.stack.add(tap_spell_id);

        // Resolve the spell - explicitly target the normal creature
        // (controller would have chosen normal_creature_id, not hexproof one)
        let result = game.resolve_spell(tap_spell_id, &[normal_creature_id]);
        assert!(result.is_ok(), "Tap spell should resolve successfully");

        // Check that the hexproof creature is not tapped
        let hexproof_card = game.cards.get(hexproof_creature_id).unwrap();
        assert!(!hexproof_card.tapped, "Hexproof creature should not be tapped");

        // Check that the normal creature was tapped
        let normal_card = game.cards.get(normal_creature_id).unwrap();
        assert!(normal_card.tapped, "Normal creature should be tapped");
    }

    #[test]
    fn test_hexproof_allows_own_spells() {
        // Test that hexproof creatures CAN be targeted by their controller's spells
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let _p2_id = game.players[1].id;

        // P1: Create a hexproof creature
        let hexproof_creature_id = game.next_entity_id();
        let mut hexproof_creature = Card::new(hexproof_creature_id, "Slippery Bogle".to_string(), p1_id);
        hexproof_creature.types.push(CardType::Creature);
        hexproof_creature.power = Some(1);
        hexproof_creature.toughness = Some(1);
        hexproof_creature.keywords.push(Keyword::Hexproof);
        game.cards.insert(hexproof_creature_id, hexproof_creature);
        game.battlefield.add(hexproof_creature_id);

        // P1: Cast Giant Growth on their own hexproof creature - should work!
        let pump_spell_id = game.next_entity_id();
        let mut pump_spell = Card::new(pump_spell_id, "Giant Growth".to_string(), p1_id);
        pump_spell.types.push(CardType::Instant);
        pump_spell.mana_cost = ManaCost::from_string("G");
        // Use placeholder card ID 0 which will be replaced with a targetable creature
        pump_spell.effects.push(Effect::PumpCreature {
            target: CardId::new(0),
            power_bonus: 3,
            toughness_bonus: 3,
        });
        game.cards.insert(pump_spell_id, pump_spell);

        // Put spell on stack (simulating cast)
        game.stack.add(pump_spell_id);

        // Resolve the spell
        let result = game.resolve_spell(pump_spell_id, &[hexproof_creature_id]);
        assert!(
            result.is_ok(),
            "Pump spell on own hexproof creature should resolve successfully"
        );

        // Check that the hexproof creature got the pump
        let creature = game.cards.get(hexproof_creature_id).unwrap();
        assert_eq!(
            creature.current_power(),
            4,
            "Hexproof creature should have boosted power (1+3)"
        );
        assert_eq!(
            creature.current_toughness(),
            4,
            "Hexproof creature should have boosted toughness (1+3)"
        );
    }

    #[test]
    fn test_hexproof_no_valid_targets() {
        // Test that spells fail to find targets if only hexproof creatures exist
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P2: Create only a hexproof creature (no valid targets for opponent)
        let hexproof_creature_id = game.next_entity_id();
        let mut hexproof_creature = Card::new(hexproof_creature_id, "Slippery Bogle".to_string(), p2_id);
        hexproof_creature.types.push(CardType::Creature);
        hexproof_creature.power = Some(1);
        hexproof_creature.toughness = Some(1);
        hexproof_creature.keywords.push(Keyword::Hexproof);
        game.cards.insert(hexproof_creature_id, hexproof_creature);
        game.battlefield.add(hexproof_creature_id);

        // P1: Try to cast a destroy spell - should fail to find valid target
        let destroy_spell_id = game.next_entity_id();
        let mut destroy_spell = Card::new(destroy_spell_id, "Terror".to_string(), p1_id);
        destroy_spell.types.push(CardType::Instant);
        destroy_spell.mana_cost = ManaCost::from_string("1B");
        // Use placeholder card ID 0 which will fail to be replaced with a target
        destroy_spell
            .effects
            .push(Effect::DestroyPermanent { target: CardId::new(0) });
        game.cards.insert(destroy_spell_id, destroy_spell);

        // Put it on the stack (simulating cast)
        game.stack.add(destroy_spell_id);

        // Resolve the spell - should succeed but do nothing (no valid targets)
        let result = game.resolve_spell(destroy_spell_id, &[]);
        assert!(result.is_ok(), "Spell with no valid targets should still resolve");

        // Check that the hexproof creature is still alive
        assert!(
            game.battlefield.contains(hexproof_creature_id),
            "Hexproof creature should still be on battlefield"
        );
    }

    #[test]
    fn test_indestructible_survives_lethal_damage() {
        // Test that indestructible creatures survive lethal damage
        use crate::game::zero_controller::ZeroController;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1: Create a 2/2 indestructible creature
        let indestructible_id = game.next_entity_id();
        let mut indestructible = Card::new(indestructible_id, "Darksteel Myr".to_string(), p1_id);
        indestructible.types.push(CardType::Creature);
        indestructible.power = Some(2);
        indestructible.toughness = Some(2);
        indestructible.keywords.push(Keyword::Indestructible);
        indestructible.controller = p1_id;
        indestructible.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        game.cards.insert(indestructible_id, indestructible);
        game.battlefield.add(indestructible_id);

        // P2: Create a 5/5 creature (blocker)
        let blocker_id = game.next_entity_id();
        let mut blocker = Card::new(blocker_id, "Hill Giant".to_string(), p2_id);
        blocker.types.push(CardType::Creature);
        blocker.power = Some(5);
        blocker.toughness = Some(5);
        blocker.controller = p2_id;
        blocker.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        game.cards.insert(blocker_id, blocker);
        game.battlefield.add(blocker_id);

        // P1 attacks with indestructible creature
        let mut controller1 = ZeroController::new(p1_id);
        let mut controller2 = ZeroController::new(p2_id);

        game.combat.declare_attacker(indestructible_id, p2_id);

        // P2 blocks with 5/5 creature
        let result = game.declare_blocker(p2_id, blocker_id, vec![indestructible_id]);
        assert!(result.is_ok(), "Failed to declare blocker: {result:?}");

        // Assign combat damage
        // Indestructible 2/2 deals 2 damage to blocker
        // Blocker 5/5 deals 5 damage to indestructible (more than lethal, but indestructible survives)
        let result = game.assign_combat_damage(&mut controller1, &mut controller2, false);
        assert!(result.is_ok(), "Failed to assign combat damage: {result:?}");

        // Indestructible creature should survive (took 5 damage but has indestructible)
        assert!(
            game.battlefield.contains(indestructible_id),
            "Indestructible creature should survive lethal damage"
        );

        // Blocker should survive (took 2 damage, has 5 toughness)
        assert!(game.battlefield.contains(blocker_id), "Blocker should survive 2 damage");
    }

    #[test]
    fn test_indestructible_immune_to_destroy_effects() {
        // Test that indestructible creatures can't be destroyed by Terror/Murder
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P2: Create an indestructible creature
        let indestructible_id = game.next_entity_id();
        let mut indestructible = Card::new(indestructible_id, "Darksteel Myr".to_string(), p2_id);
        indestructible.types.push(CardType::Creature);
        indestructible.power = Some(0);
        indestructible.toughness = Some(1);
        indestructible.keywords.push(Keyword::Indestructible);
        game.cards.insert(indestructible_id, indestructible);
        game.battlefield.add(indestructible_id);

        // P1: Cast Terror targeting the indestructible creature
        let destroy_spell_id = game.next_entity_id();
        let mut destroy_spell = Card::new(destroy_spell_id, "Terror".to_string(), p1_id);
        destroy_spell.types.push(CardType::Instant);
        destroy_spell.mana_cost = ManaCost::from_string("1B");
        // Explicitly target the indestructible creature
        destroy_spell.effects.push(Effect::DestroyPermanent {
            target: indestructible_id,
        });
        game.cards.insert(destroy_spell_id, destroy_spell);

        // Put it on the stack
        game.stack.add(destroy_spell_id);

        // Resolve the spell
        let result = game.resolve_spell(destroy_spell_id, &[]);
        assert!(result.is_ok(), "Destroy spell should resolve successfully");

        // Indestructible creature should still be alive
        assert!(
            game.battlefield.contains(indestructible_id),
            "Indestructible creature should survive destroy effect"
        );
    }

    #[test]
    fn test_indestructible_survives_deathtouch() {
        // Test that indestructible creatures survive deathtouch damage
        use crate::game::zero_controller::ZeroController;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1: Create a 1/1 deathtouch creature (attacker)
        let deathtouch_id = game.next_entity_id();
        let mut deathtouch = Card::new(deathtouch_id, "Typhoid Rats".to_string(), p1_id);
        deathtouch.types.push(CardType::Creature);
        deathtouch.power = Some(1);
        deathtouch.toughness = Some(1);
        deathtouch.keywords.push(Keyword::Deathtouch);
        deathtouch.controller = p1_id;
        deathtouch.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        game.cards.insert(deathtouch_id, deathtouch);
        game.battlefield.add(deathtouch_id);

        // P2: Create a 5/5 indestructible creature (blocker)
        let indestructible_id = game.next_entity_id();
        let mut indestructible = Card::new(indestructible_id, "Darksteel Colossus".to_string(), p2_id);
        indestructible.types.push(CardType::Creature);
        indestructible.power = Some(5);
        indestructible.toughness = Some(5);
        indestructible.keywords.push(Keyword::Indestructible);
        indestructible.controller = p2_id;
        indestructible.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        game.cards.insert(indestructible_id, indestructible);
        game.battlefield.add(indestructible_id);

        // P1 attacks with deathtouch creature
        let mut controller1 = ZeroController::new(p1_id);
        let mut controller2 = ZeroController::new(p2_id);

        game.combat.declare_attacker(deathtouch_id, p2_id);

        // P2 blocks with indestructible creature
        let result = game.declare_blocker(p2_id, indestructible_id, vec![deathtouch_id]);
        assert!(result.is_ok(), "Failed to declare blocker: {result:?}");

        // Assign combat damage
        // Deathtouch 1/1 deals 1 damage to indestructible (deathtouch damage, but indestructible survives)
        // Indestructible 5/5 deals 5 damage to deathtouch (kills it)
        let result = game.assign_combat_damage(&mut controller1, &mut controller2, false);
        assert!(result.is_ok(), "Failed to assign combat damage: {result:?}");

        // Indestructible creature should survive deathtouch damage
        assert!(
            game.battlefield.contains(indestructible_id),
            "Indestructible creature should survive deathtouch damage"
        );

        // Deathtouch creature should be dead (took 5 damage, has 1 toughness)
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert!(
                zones.graveyard.contains(deathtouch_id),
                "Deathtouch creature should be in graveyard"
            );
        }
    }

    #[test]
    fn test_indestructible_vs_non_indestructible_combat() {
        // Test that normal creature dies while indestructible survives in mutual combat
        use crate::game::zero_controller::ZeroController;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1: Create a 3/3 indestructible creature (attacker)
        let indestructible_id = game.next_entity_id();
        let mut indestructible = Card::new(indestructible_id, "Indomitable".to_string(), p1_id);
        indestructible.types.push(CardType::Creature);
        indestructible.power = Some(3);
        indestructible.toughness = Some(3);
        indestructible.keywords.push(Keyword::Indestructible);
        indestructible.controller = p1_id;
        indestructible.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        game.cards.insert(indestructible_id, indestructible);
        game.battlefield.add(indestructible_id);

        // P2: Create a 3/3 normal creature (blocker)
        let normal_id = game.next_entity_id();
        let mut normal = Card::new(normal_id, "Hill Giant".to_string(), p2_id);
        normal.types.push(CardType::Creature);
        normal.power = Some(3);
        normal.toughness = Some(3);
        normal.controller = p2_id;
        normal.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        game.cards.insert(normal_id, normal);
        game.battlefield.add(normal_id);

        // P1 attacks with indestructible creature
        let mut controller1 = ZeroController::new(p1_id);
        let mut controller2 = ZeroController::new(p2_id);

        game.combat.declare_attacker(indestructible_id, p2_id);

        // P2 blocks with normal creature
        let result = game.declare_blocker(p2_id, normal_id, vec![indestructible_id]);
        assert!(result.is_ok(), "Failed to declare blocker: {result:?}");

        // Assign combat damage
        // Both deal 3 damage to each other (lethal)
        // Indestructible survives, normal dies
        let result = game.assign_combat_damage(&mut controller1, &mut controller2, false);
        assert!(result.is_ok(), "Failed to assign combat damage: {result:?}");

        // Indestructible creature should survive
        assert!(
            game.battlefield.contains(indestructible_id),
            "Indestructible creature should survive"
        );

        // Normal creature should be dead
        if let Some(zones) = game.get_player_zones(p2_id) {
            assert!(
                zones.graveyard.contains(normal_id),
                "Normal creature should be in graveyard"
            );
        }
    }

    #[test]
    fn test_shroud_blocks_destroy_from_opponent() {
        // Test that destroy spells from opponents can't target shroud creatures
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P2: Create a shroud creature
        let shroud_creature_id = game.next_entity_id();
        let mut shroud_creature = Card::new(shroud_creature_id, "Silhana Ledgewalker".to_string(), p2_id);
        shroud_creature.types.push(CardType::Creature);
        shroud_creature.power = Some(1);
        shroud_creature.toughness = Some(1);
        shroud_creature.keywords.push(Keyword::Shroud);
        game.cards.insert(shroud_creature_id, shroud_creature);
        game.battlefield.add(shroud_creature_id);

        // P2: Create a normal creature
        let normal_creature_id = game.next_entity_id();
        let mut normal_creature = Card::new(normal_creature_id, "Grizzly Bears".to_string(), p2_id);
        normal_creature.types.push(CardType::Creature);
        normal_creature.power = Some(2);
        normal_creature.toughness = Some(2);
        game.cards.insert(normal_creature_id, normal_creature);
        game.battlefield.add(normal_creature_id);

        // P1: Cast Terror - should target normal creature, not shroud one
        let destroy_spell_id = game.next_entity_id();
        let mut destroy_spell = Card::new(destroy_spell_id, "Terror".to_string(), p1_id);
        destroy_spell.types.push(CardType::Instant);
        destroy_spell.mana_cost = ManaCost::from_string("1B");
        destroy_spell
            .effects
            .push(Effect::DestroyPermanent { target: CardId::new(0) });
        game.cards.insert(destroy_spell_id, destroy_spell);
        game.stack.add(destroy_spell_id);

        let result = game.resolve_spell(destroy_spell_id, &[normal_creature_id]);
        assert!(result.is_ok(), "Destroy spell should resolve");

        // Shroud creature should still be alive
        assert!(
            game.battlefield.contains(shroud_creature_id),
            "Shroud creature should still be on battlefield"
        );

        // Normal creature was destroyed
        if let Some(zones) = game.get_player_zones(p2_id) {
            assert!(
                zones.graveyard.contains(normal_creature_id),
                "Normal creature should be in graveyard"
            );
        }
    }

    #[test]
    fn test_shroud_blocks_pump_from_controller() {
        // Test that shroud prevents targeting even by the controller (unlike hexproof)
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let _p2_id = game.players[1].id;

        // P1: Create a shroud creature
        let shroud_creature_id = game.next_entity_id();
        let mut shroud_creature = Card::new(shroud_creature_id, "Silhana Ledgewalker".to_string(), p1_id);
        shroud_creature.types.push(CardType::Creature);
        shroud_creature.power = Some(1);
        shroud_creature.toughness = Some(1);
        shroud_creature.keywords.push(Keyword::Shroud);
        game.cards.insert(shroud_creature_id, shroud_creature);
        game.battlefield.add(shroud_creature_id);

        // P1: Create a normal creature
        let normal_creature_id = game.next_entity_id();
        let mut normal_creature = Card::new(normal_creature_id, "Grizzly Bears".to_string(), p1_id);
        normal_creature.types.push(CardType::Creature);
        normal_creature.power = Some(2);
        normal_creature.toughness = Some(2);
        game.cards.insert(normal_creature_id, normal_creature);
        game.battlefield.add(normal_creature_id);

        // P1: Cast Giant Growth - should target normal creature, not shroud one
        let pump_spell_id = game.next_entity_id();
        let mut pump_spell = Card::new(pump_spell_id, "Giant Growth".to_string(), p1_id);
        pump_spell.types.push(CardType::Instant);
        pump_spell.mana_cost = ManaCost::from_string("G");
        pump_spell.effects.push(Effect::PumpCreature {
            target: CardId::new(0),
            power_bonus: 3,
            toughness_bonus: 3,
        });
        game.cards.insert(pump_spell_id, pump_spell);
        game.stack.add(pump_spell_id);

        let result = game.resolve_spell(pump_spell_id, &[normal_creature_id]);
        assert!(result.is_ok(), "Pump spell should resolve");

        // Shroud creature should NOT have the pump
        let shroud_card = game.cards.get(shroud_creature_id).unwrap();
        assert_eq!(
            shroud_card.current_power(),
            1,
            "Shroud creature should not have boosted power"
        );

        // Normal creature should have the pump
        let normal_card = game.cards.get(normal_creature_id).unwrap();
        assert_eq!(
            normal_card.current_power(),
            5,
            "Normal creature should have boosted power (2+3)"
        );
    }

    #[test]
    fn test_shroud_blocks_tap_effect() {
        // Test that tap effects can't target shroud creatures
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P2: Create a shroud creature
        let shroud_creature_id = game.next_entity_id();
        let mut shroud_creature = Card::new(shroud_creature_id, "Silhana Ledgewalker".to_string(), p2_id);
        shroud_creature.types.push(CardType::Creature);
        shroud_creature.power = Some(1);
        shroud_creature.toughness = Some(1);
        shroud_creature.keywords.push(Keyword::Shroud);
        game.cards.insert(shroud_creature_id, shroud_creature);
        game.battlefield.add(shroud_creature_id);

        // P2: Create a normal creature
        let normal_creature_id = game.next_entity_id();
        let mut normal_creature = Card::new(normal_creature_id, "Grizzly Bears".to_string(), p2_id);
        normal_creature.types.push(CardType::Creature);
        normal_creature.power = Some(2);
        normal_creature.toughness = Some(2);
        game.cards.insert(normal_creature_id, normal_creature);
        game.battlefield.add(normal_creature_id);

        // P1: Cast tap spell - should target normal creature, not shroud one
        let tap_spell_id = game.next_entity_id();
        let mut tap_spell = Card::new(tap_spell_id, "Frost Breath".to_string(), p1_id);
        tap_spell.types.push(CardType::Instant);
        tap_spell.mana_cost = ManaCost::from_string("2U");
        tap_spell.effects.push(Effect::TapPermanent { target: CardId::new(0) });
        game.cards.insert(tap_spell_id, tap_spell);
        game.stack.add(tap_spell_id);

        let result = game.resolve_spell(tap_spell_id, &[normal_creature_id]);
        assert!(result.is_ok(), "Tap spell should resolve");

        // Shroud creature should not be tapped
        let shroud_card = game.cards.get(shroud_creature_id).unwrap();
        assert!(!shroud_card.tapped, "Shroud creature should not be tapped");

        // Normal creature should be tapped
        let normal_card = game.cards.get(normal_creature_id).unwrap();
        assert!(normal_card.tapped, "Normal creature should be tapped");
    }

    #[test]
    fn test_execute_mill_effect() {
        use crate::core::Effect;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players.first().unwrap().id;

        // Add cards to library
        for i in 0..5 {
            let card_id = game.next_card_id();
            let card = Card::new(card_id, format!("Card {i}"), p1_id);
            game.cards.insert(card_id, card);
            if let Some(zones) = game.get_player_zones_mut(p1_id) {
                zones.library.add(card_id);
            }
        }

        // Mill 3 cards
        let effect = Effect::Mill {
            player: p1_id,
            count: 3,
        };

        assert!(game.execute_effect(&effect).is_ok());

        // Check cards were milled (library reduced, graveyard increased)
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert_eq!(zones.library.cards.len(), 2, "Should have 2 cards left in library");
            assert_eq!(zones.graveyard.cards.len(), 3, "Should have 3 cards in graveyard");
        }
    }

    #[test]
    fn test_mill_with_empty_library() {
        use crate::core::Effect;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players.first().unwrap().id;

        // Add only 2 cards to library
        for i in 0..2 {
            let card_id = game.next_card_id();
            let card = Card::new(card_id, format!("Card {i}"), p1_id);
            game.cards.insert(card_id, card);
            if let Some(zones) = game.get_player_zones_mut(p1_id) {
                zones.library.add(card_id);
            }
        }

        // Try to mill 5 cards (more than available)
        let effect = Effect::Mill {
            player: p1_id,
            count: 5,
        };

        assert!(game.execute_effect(&effect).is_ok());

        // Check only 2 cards were milled (library is empty)
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert_eq!(zones.library.cards.len(), 0, "Library should be empty");
            assert_eq!(zones.graveyard.cards.len(), 2, "Should have milled only 2 cards");
        }
    }

    #[test]
    fn test_counter_spell_effect() {
        use crate::core::{CardType, Effect, ManaCost};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let players: Vec<_> = game.players.iter().map(|p| p.id).collect();
        let p1_id = players[0];
        let p2_id = players[1];

        // P1 casts Lightning Bolt (target spell to be countered)
        let bolt_id = game.next_card_id();
        let mut bolt = Card::new(bolt_id, "Lightning Bolt".to_string(), p1_id);
        bolt.types.push(CardType::Instant);
        bolt.mana_cost = ManaCost::from_string("R");
        bolt.effects.push(Effect::DealDamage {
            target: crate::core::TargetRef::Player(p2_id),
            amount: 3,
        });
        game.cards.insert(bolt_id, bolt);
        game.stack.add(bolt_id);

        // P2 responds with Counterspell
        let counter_id = game.next_card_id();
        let mut counterspell = Card::new(counter_id, "Counterspell".to_string(), p2_id);
        counterspell.types.push(CardType::Instant);
        counterspell.mana_cost = ManaCost::from_string("UU");
        counterspell.effects.push(Effect::CounterSpell { target: bolt_id });
        game.cards.insert(counter_id, counterspell);
        game.stack.add(counter_id);

        // Verify both are on the stack
        assert!(game.stack.contains(bolt_id));
        assert!(game.stack.contains(counter_id));

        // Resolve counterspell (counters Lightning Bolt)
        assert!(game.resolve_spell(counter_id, &[]).is_ok());

        // Verify counterspell is in graveyard
        if let Some(zones) = game.get_player_zones(p2_id) {
            assert!(zones.graveyard.contains(counter_id));
        }

        // Verify Lightning Bolt was countered (removed from stack, in graveyard)
        assert!(!game.stack.contains(bolt_id));
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert!(
                zones.graveyard.contains(bolt_id),
                "Countered spell should be in graveyard"
            );
        }

        // Verify P2 didn't take damage (Lightning Bolt was countered before resolving)
        let p2 = game.get_player(p2_id).unwrap();
        assert_eq!(p2.life, 20, "Player 2 should still have 20 life");
    }

    #[test]
    fn test_counter_spell_with_placeholder_target() {
        use crate::core::{CardType, Effect, ManaCost};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let players: Vec<_> = game.players.iter().map(|p| p.id).collect();
        let p1_id = players[0];
        let p2_id = players[1];

        // P1 casts Lightning Bolt
        let bolt_id = game.next_card_id();
        let mut bolt = Card::new(bolt_id, "Lightning Bolt".to_string(), p1_id);
        bolt.types.push(CardType::Instant);
        bolt.mana_cost = ManaCost::from_string("R");
        bolt.effects.push(Effect::DealDamage {
            target: crate::core::TargetRef::Player(p2_id),
            amount: 3,
        });
        game.cards.insert(bolt_id, bolt);
        game.stack.add(bolt_id);

        // P2 responds with Counterspell using placeholder target (CardId::new(0))
        let counter_id = game.next_card_id();
        let mut counterspell = Card::new(counter_id, "Counterspell".to_string(), p2_id);
        counterspell.types.push(CardType::Instant);
        counterspell.mana_cost = ManaCost::from_string("UU");
        // Use placeholder target - should automatically target opponent's spell
        counterspell.effects.push(Effect::CounterSpell {
            target: crate::core::CardId::new(0),
        });
        game.cards.insert(counter_id, counterspell);
        game.stack.add(counter_id);

        // Resolve counterspell - should automatically find and counter Lightning Bolt
        assert!(game.resolve_spell(counter_id, &[bolt_id]).is_ok());

        // Verify Lightning Bolt was countered
        assert!(!game.stack.contains(bolt_id));
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert!(
                zones.graveyard.contains(bolt_id),
                "Countered spell should be in graveyard"
            );
        }
    }

    #[test]
    fn test_etb_trigger_draw() {
        use crate::core::{Effect, Trigger, TriggerEvent};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players.first().unwrap().id;

        // Add cards to P1's library for drawing
        for i in 0..5 {
            let card_id = game.next_card_id();
            let card = Card::new(card_id, format!("Card {i}"), p1_id);
            game.cards.insert(card_id, card);
            if let Some(zones) = game.get_player_zones_mut(p1_id) {
                zones.library.add(card_id);
            }
        }

        // Create a creature with an ETB trigger (like Elvish Visionary)
        let creature_id = game.next_entity_id();
        let mut creature = Card::new(creature_id, "Elvish Visionary".to_string(), p1_id);
        creature.types.push(CardType::Creature);
        creature.power = Some(1);
        creature.toughness = Some(1);
        creature.mana_cost = ManaCost::from_string("1G");

        // Add ETB trigger: "When this enters the battlefield, draw a card"
        creature.triggers.push(Trigger::new(
            TriggerEvent::EntersBattlefield,
            vec![Effect::DrawCards {
                player: p1_id,
                count: 1,
            }],
            "When Elvish Visionary enters, draw a card.".to_string(),
        ));

        game.cards.insert(creature_id, creature);

        // Put the creature on the stack (as if it was cast)
        game.stack.add(creature_id);

        // Resolve the creature spell (moves it to battlefield and triggers ETB)
        assert!(game.resolve_spell(creature_id, &[]).is_ok());

        // Verify the creature is on the battlefield
        assert!(game.battlefield.contains(creature_id));

        // Verify the ETB trigger drew a card
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert_eq!(zones.hand.cards.len(), 1, "Should have drawn 1 card");
            assert_eq!(zones.library.cards.len(), 4, "Should have 4 cards left in library");
        }
    }

    #[test]
    fn test_etb_trigger_damage() {
        use crate::core::{Effect, TargetRef, Trigger, TriggerEvent};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P2: Create a target creature
        let target_creature_id = game.next_entity_id();
        let mut target = Card::new(target_creature_id, "Grizzly Bears".to_string(), p2_id);
        target.types.push(CardType::Creature);
        target.power = Some(2);
        target.toughness = Some(2);
        game.cards.insert(target_creature_id, target);
        game.battlefield.add(target_creature_id);

        // P1: Create a creature with ETB damage trigger (like Flametongue Kavu)
        let kavu_id = game.next_entity_id();
        let mut kavu = Card::new(kavu_id, "Flametongue Kavu".to_string(), p1_id);
        kavu.types.push(CardType::Creature);
        kavu.power = Some(4);
        kavu.toughness = Some(2);
        kavu.mana_cost = ManaCost::from_string("3R");

        // Add ETB trigger: "When this enters the battlefield, deal 4 damage to target creature"
        kavu.triggers.push(Trigger::new(
            TriggerEvent::EntersBattlefield,
            vec![Effect::DealDamage {
                target: TargetRef::None, // Will be filled to target an opponent's creature
                amount: 4,
            }],
            "When Flametongue Kavu enters, it deals 4 damage to target creature.".to_string(),
        ));

        game.cards.insert(kavu_id, kavu);

        // Put the kavu on the stack (as if it was cast)
        game.stack.add(kavu_id);

        // Resolve the kavu spell (moves it to battlefield and triggers ETB)
        assert!(game.resolve_spell(kavu_id, &[]).is_ok());

        // Verify the kavu is on the battlefield
        assert!(game.battlefield.contains(kavu_id));

        // Verify the target creature took lethal damage (2 toughness, 4 damage dealt)
        // The creature should have been destroyed and moved to graveyard
        assert!(
            !game.battlefield.contains(target_creature_id),
            "Target creature should be destroyed"
        );
        if let Some(zones) = game.get_player_zones(p2_id) {
            assert!(
                zones.graveyard.contains(target_creature_id),
                "Target creature should be in graveyard"
            );
        }
    }

    #[test]
    fn test_elvish_visionary_from_cardsfolder() {
        // Test loading Elvish Visionary from the actual cardsfolder and verifying
        // its ETB trigger works correctly
        use crate::loader::CardDatabase;
        use std::path::PathBuf;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players.first().unwrap().id;

        // Add cards to P1's library for drawing
        for i in 0..5 {
            let card_id = game.next_card_id();
            let card = Card::new(card_id, format!("Card {i}"), p1_id);
            game.cards.insert(card_id, card);
            if let Some(zones) = game.get_player_zones_mut(p1_id) {
                zones.library.add(card_id);
            }
        }

        // Load Elvish Visionary from cardsfolder
        let cardsfolder = PathBuf::from("./cardsfolder");
        let db = CardDatabase::new(cardsfolder);

        let card_def = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async { db.get_card("Elvish Visionary").await })
            .expect("Failed to get card")
            .expect("Elvish Visionary not found");

        // Create card instance
        let creature_id = game.next_entity_id();
        let creature = card_def.instantiate(creature_id, p1_id);

        // Verify it has an ETB trigger
        assert!(!creature.triggers.is_empty(), "Elvish Visionary should have triggers");
        assert_eq!(creature.triggers.len(), 1, "Elvish Visionary should have 1 trigger");

        game.cards.insert(creature_id, creature);

        // Put the creature on the stack (as if it was cast)
        game.stack.add(creature_id);

        // Resolve the creature spell (moves it to battlefield and triggers ETB)
        assert!(game.resolve_spell(creature_id, &[]).is_ok());

        // Verify the creature is on the battlefield
        assert!(game.battlefield.contains(creature_id));

        // Verify the ETB trigger drew a card
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert_eq!(zones.hand.cards.len(), 1, "Should have drawn 1 card from ETB trigger");
            assert_eq!(zones.library.cards.len(), 4, "Should have 4 cards left in library");
        }
    }

    #[test]
    fn test_counterspell_from_cardsfolder() {
        // Test loading Counterspell from the actual cardsfolder and verifying
        // it can counter spells
        use crate::loader::CardDatabase;
        use std::path::PathBuf;

        let mut game = GameState::new_two_player("Alice".to_string(), "Bob".to_string(), 20);
        let players: Vec<_> = game.players.iter().map(|p| p.id).collect();
        let alice_id = players[0];
        let bob_id = players[1];

        // Load Lightning Bolt and Counterspell from cardsfolder
        let cardsfolder = PathBuf::from("./cardsfolder");
        let db = CardDatabase::new(cardsfolder);

        let runtime = tokio::runtime::Runtime::new().unwrap();

        let bolt_def = runtime
            .block_on(async { db.get_card("Lightning Bolt").await })
            .expect("Failed to get card")
            .expect("Lightning Bolt not found");

        let counter_def = runtime
            .block_on(async { db.get_card("Counterspell").await })
            .expect("Failed to get card")
            .expect("Counterspell not found");

        // Alice casts Lightning Bolt targeting Bob
        let bolt_id = game.next_card_id();
        let bolt = bolt_def.instantiate(bolt_id, alice_id);
        game.cards.insert(bolt_id, bolt);
        game.stack.add(bolt_id);

        // Bob responds with Counterspell targeting Lightning Bolt
        let counter_id = game.next_card_id();
        let mut counterspell = counter_def.instantiate(counter_id, bob_id);
        // Manually set the target since we're bypassing the full casting process
        if let Some(Effect::CounterSpell { target }) = counterspell.effects.get_mut(0) {
            *target = bolt_id;
        }
        game.cards.insert(counter_id, counterspell);
        game.stack.add(counter_id);

        // Verify both are on the stack
        assert!(game.stack.contains(bolt_id), "Bolt should be on stack");
        assert!(game.stack.contains(counter_id), "Counterspell should be on stack");

        // Resolve Counterspell (should counter Lightning Bolt)
        assert!(
            game.resolve_spell(counter_id, &[]).is_ok(),
            "Counterspell should resolve successfully"
        );

        // Verify Counterspell is in graveyard
        if let Some(zones) = game.get_player_zones(bob_id) {
            assert!(
                zones.graveyard.contains(counter_id),
                "Counterspell should be in graveyard"
            );
        }

        // Verify Lightning Bolt was countered (removed from stack, in graveyard)
        assert!(!game.stack.contains(bolt_id), "Lightning Bolt should not be on stack");
        if let Some(zones) = game.get_player_zones(alice_id) {
            assert!(
                zones.graveyard.contains(bolt_id),
                "Countered spell should be in graveyard"
            );
        }

        // Verify Bob didn't take damage (Lightning Bolt was countered)
        let bob = game.get_player(bob_id).unwrap();
        assert_eq!(bob.life, 20, "Bob should still have 20 life");
    }

    #[test]
    fn test_etb_trigger_gain_life() {
        use crate::core::{Effect, Trigger, TriggerEvent};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Create a creature with ETB gain life trigger
        let creature_id = game.next_entity_id();
        let mut creature = Card::new(creature_id, "Soul Warden".to_string(), p1_id);
        creature.types.push(CardType::Creature);
        creature.power = Some(1);
        creature.toughness = Some(1);
        creature.mana_cost = ManaCost::from_string("W");

        // Add ETB trigger: "When this enters the battlefield, you gain 3 life"
        creature.triggers.push(Trigger::new(
            TriggerEvent::EntersBattlefield,
            vec![Effect::GainLife {
                player: crate::core::PlayerId::new(0), // Placeholder
                amount: 3,
            }],
            "When this enters, you gain 3 life.".to_string(),
        ));

        game.cards.insert(creature_id, creature);

        // Record life before
        let life_before = game.get_player(p1_id).unwrap().life;

        // Put the creature on the stack and resolve it
        game.stack.add(creature_id);
        assert!(game.resolve_spell(creature_id, &[]).is_ok());

        // Verify the creature is on the battlefield
        assert!(game.battlefield.contains(creature_id));

        // Verify the ETB trigger gained life
        let life_after = game.get_player(p1_id).unwrap().life;
        assert_eq!(
            life_after,
            life_before + 3,
            "Should have gained 3 life from ETB trigger"
        );
    }

    #[test]
    fn test_etb_trigger_pump() {
        use crate::core::{Effect, Trigger, TriggerEvent};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Create a target creature on battlefield
        let target_id = game.next_entity_id();
        let mut target = Card::new(target_id, "Grizzly Bears".to_string(), p1_id);
        target.types.push(CardType::Creature);
        target.power = Some(2);
        target.toughness = Some(2);
        game.cards.insert(target_id, target);
        game.battlefield.add(target_id);

        // Create a creature with ETB pump trigger
        let creature_id = game.next_entity_id();
        let mut creature = Card::new(creature_id, "Glorious Anthem".to_string(), p1_id);
        creature.types.push(CardType::Creature);
        creature.power = Some(1);
        creature.toughness = Some(1);

        // Add ETB trigger: "When this enters, target creature gets +2/+2"
        creature.triggers.push(Trigger::new(
            TriggerEvent::EntersBattlefield,
            vec![Effect::PumpCreature {
                target: crate::core::CardId::new(0), // Placeholder
                power_bonus: 2,
                toughness_bonus: 2,
            }],
            "When this enters, target creature gets +2/+2.".to_string(),
        ));

        game.cards.insert(creature_id, creature);

        // Put the creature on the stack and resolve it
        game.stack.add(creature_id);
        assert!(game.resolve_spell(creature_id, &[]).is_ok());

        // Verify the creature is on the battlefield
        assert!(game.battlefield.contains(creature_id));

        // Verify the target got pumped
        let pumped_card = game.cards.get(target_id).unwrap();
        assert_eq!(pumped_card.power_bonus, 2, "Target should have +2 power bonus");
        assert_eq!(pumped_card.toughness_bonus, 2, "Target should have +2 toughness bonus");
    }
}
