//! Game actions and mechanics

use crate::core::{CardId, CardType, Effect, PlayerId, TargetRef, TriggerEvent};
use crate::game::GameState;
use crate::zones::Zone;
use crate::{MtgError, Result};

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
            // Capture old value and log size before mutation
            let old_value = card.turn_entered_battlefield;
            let prior_log_size = self.logger.log_count();

            let new_value = Some(self.turn.turn_number);
            card.turn_entered_battlefield = new_value;

            // Log the mutation for undo
            self.undo_log.log(
                crate::undo::GameAction::SetTurnEnteredBattlefield {
                    card_id,
                    old_value,
                    new_value,
                },
                prior_log_size,
            );
        }

        // Increment lands played
        // Capture old value and log size before mutation (before get_player_mut to avoid borrow issues)
        let old_value = self.get_player(player_id)?.lands_played_this_turn;
        let prior_log_size = self.logger.log_count();

        let player = self.get_player_mut(player_id)?;
        player.play_land();
        let new_value = player.lands_played_this_turn;

        // Log the mutation for undo
        self.undo_log.log(
            crate::undo::GameAction::SetLandsPlayedThisTurn {
                player_id,
                old_value,
                new_value,
            },
            prior_log_size,
        );

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
                Effect::DestroyPermanent { target, restriction }
                    if target.as_u32() == 0 && target_index < chosen_targets.len() =>
                {
                    // Use the chosen target
                    *effect = Effect::DestroyPermanent {
                        target: chosen_targets[target_index],
                        restriction: restriction.clone(),
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
        // Check all effects that target permanents or spells on the stack
        let mut all_targets_illegal = false;
        if !chosen_targets.is_empty() {
            // Check if this spell targets permanents (any effect type)
            let targets_permanents = effects.iter().any(|effect| {
                matches!(
                    effect,
                    Effect::DealDamage {
                        target: TargetRef::Permanent(_),
                        ..
                    } | Effect::DestroyPermanent { .. }
                        | Effect::ExilePermanent { .. }
                        | Effect::TapPermanent { .. }
                        | Effect::UntapPermanent { .. }
                        | Effect::PumpCreature { .. }
                )
            });

            if targets_permanents {
                // Check if any permanent target is no longer on the battlefield
                // This happens when multiple spells target the same permanent
                let any_target_gone = chosen_targets
                    .iter()
                    .any(|&target_id| !self.battlefield.contains(target_id));
                if any_target_gone {
                    // Spell fizzles - permanent targets are no longer valid
                    all_targets_illegal = true;
                }
            }

            // Check if this spell counters spells on the stack
            let targets_stack = effects
                .iter()
                .any(|effect| matches!(effect, Effect::CounterSpell { .. }));

            if targets_stack {
                // Check if any stack target is no longer on the stack
                // This happens when multiple counterspells target the same spell
                let any_target_gone = chosen_targets.iter().any(|&target_id| !self.stack.contains(target_id));
                if any_target_gone {
                    // Spell fizzles - target spell is no longer on the stack
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
                // Capture old value and log size before mutation
                let old_value = card.turn_entered_battlefield;
                let prior_log_size = self.logger.log_count();

                let new_value = Some(self.turn.turn_number);
                card.turn_entered_battlefield = new_value;

                // Log the mutation for undo
                self.undo_log.log(
                    crate::undo::GameAction::SetTurnEnteredBattlefield {
                        card_id,
                        old_value,
                        new_value,
                    },
                    prior_log_size,
                );
            }

            // Check for ETB triggers on all permanents (including the one that just entered)
            self.check_triggers(TriggerEvent::EntersBattlefield, card_id)?;
        }

        Ok(())
    }

    /// Attach Equipment or Aura to a target card
    ///
    /// This is called when:
    /// - An Equip activated ability resolves
    /// - An Aura spell resolves (attaching to its target)
    /// - An effect moves an Equipment to attach to a new target
    ///
    /// ## Rules Implementation (CR 301.5, 303.4)
    /// - Equipment can only attach to creatures
    /// - Auras can attach based on their enchant ability
    /// - If already attached, detaches from previous target first
    /// - Updates timestamp on the Equipment/Aura (CR 613.7e)
    pub fn attach_equipment(&mut self, equipment_id: CardId, target_id: CardId) -> Result<()> {
        // Validate Equipment is on battlefield
        if !self.battlefield.contains(equipment_id) {
            return Err(MtgError::InvalidAction(
                "Equipment must be on battlefield to attach".to_string(),
            ));
        }

        // Validate target is on battlefield
        if !self.battlefield.contains(target_id) {
            return Err(MtgError::InvalidAction("Target must be on battlefield".to_string()));
        }

        // Get Equipment and target
        let equipment = self.cards.get(equipment_id)?;
        if !equipment.is_equipment() && !equipment.is_aura() {
            return Err(MtgError::InvalidAction(
                "Only Equipment or Auras can be attached".to_string(),
            ));
        }

        let target = self.cards.get(target_id)?;
        if !target.is_creature() {
            return Err(MtgError::InvalidAction(
                "Equipment can only attach to creatures".to_string(),
            ));
        }

        // Check controller ownership (Equipment can only attach to creatures you control)
        let equipment_controller = equipment.controller;
        let target_controller = target.controller;
        if equipment_controller != target_controller {
            return Err(MtgError::InvalidAction(
                "Equipment can only attach to creatures you control".to_string(),
            ));
        }

        // Detach from previous target if needed
        let equipment = self.cards.get_mut(equipment_id)?;
        let equipment_name = equipment.name.to_string();
        if let Some(old_target) = equipment.attached_to {
            // Log detachment
            let old_target_name = self
                .cards
                .get(old_target)
                .map(|c| c.name.to_string())
                .unwrap_or_else(|_| "unknown".to_string());
            self.logger
                .verbose(&format!("{} detaches from {}", equipment_name, old_target_name));
        }

        // Attach to new target
        // Capture old value and log size before mutation
        let old_target = self.cards.get(equipment_id)?.attached_to;
        let prior_log_size = self.logger.log_count();

        let equipment = self.cards.get_mut(equipment_id)?;
        let new_target = Some(target_id);
        equipment.attached_to = new_target;

        // Log the mutation for undo
        self.undo_log.log(
            crate::undo::GameAction::SetAttachedTo {
                equipment_id,
                old_target,
                new_target,
            },
            prior_log_size,
        );

        // Log attachment
        let target_name = self
            .cards
            .get(target_id)
            .map(|c| c.name.to_string())
            .unwrap_or_else(|_| "unknown".to_string());
        self.logger
            .verbose(&format!("{} attaches to {}", equipment_name, target_name));

        Ok(())
    }

    /// Detach Equipment or Aura from its target
    ///
    /// This is called when:
    /// - The attached creature leaves the battlefield (state-based action)
    /// - An effect explicitly detaches the Equipment
    /// - The Equipment/Aura leaves the battlefield
    ///
    /// ## Rules Implementation
    /// - Equipment remains on battlefield when detached
    /// - Auras that become unattached typically go to graveyard (handled elsewhere)
    pub fn detach_equipment(&mut self, equipment_id: CardId) -> Result<()> {
        // Get names and attached_to before mutable borrow
        let equipment = self.cards.get(equipment_id)?;
        let equipment_name = equipment.name.to_string();
        let target_id_opt = equipment.attached_to;

        if let Some(target_id) = target_id_opt {
            // Log detachment
            let target_name = self
                .cards
                .get(target_id)
                .map(|c| c.name.to_string())
                .unwrap_or_else(|_| "unknown".to_string());
            self.logger
                .verbose(&format!("{} detaches from {}", equipment_name, target_name));

            // Now do the actual detachment
            // Capture old value and log size before mutation
            let old_target = target_id_opt; // We already have this from above
            let prior_log_size = self.logger.log_count();

            let equipment = self.cards.get_mut(equipment_id)?;
            let new_target = None;
            equipment.attached_to = new_target;

            // Log the mutation for undo
            self.undo_log.log(
                crate::undo::GameAction::SetAttachedTo {
                    equipment_id,
                    old_target,
                    new_target,
                },
                prior_log_size,
            );
        }

        Ok(())
    }

    /// Get all Equipment attached to a creature
    ///
    /// Used for:
    /// - Calculating creature's effective power/toughness with Equipment buffs
    /// - Determining which Equipment to detach when creature leaves battlefield
    /// - AI evaluation of creature strength
    pub fn get_attached_equipment(&self, creature_id: CardId) -> Vec<CardId> {
        self.battlefield
            .cards
            .iter()
            .filter_map(|&card_id| {
                let card = self.cards.get(card_id).ok()?;
                if card.is_equipment() && card.attached_to == Some(creature_id) {
                    Some(card_id)
                } else {
                    None
                }
            })
            .collect()
    }

    /// Get all Auras attached to a permanent
    ///
    /// Used for:
    /// - Calculating creature's effective power/toughness with Aura buffs
    /// - Determining which Auras to move to graveyard when enchanted permanent leaves
    /// - AI evaluation of permanent strength
    pub fn get_attached_auras(&self, permanent_id: CardId) -> Vec<CardId> {
        self.battlefield
            .cards
            .iter()
            .filter_map(|&card_id| {
                let card = self.cards.get(card_id).ok()?;
                if card.is_aura() && card.attached_to == Some(permanent_id) {
                    Some(card_id)
                } else {
                    None
                }
            })
            .collect()
    }

    /// Get a creature's effective power using CR 613 layer system.
    ///
    /// ## Comprehensive Rules 613.4 (Layer 7: Power and Toughness)
    ///
    /// This implements the full layer calculation:
    /// 1. Layer 7a (CHARACTERISTIC): Characteristic-defining abilities (e.g., Tarmogoyf)
    /// 2. Layer 7b (SETPT): Effects that SET P/T (e.g., "becomes 0/1")
    /// 3. Layer 7c (MODIFYPT): Effects and counters that MODIFY P/T (Equipment, +1/+1 counters)
    /// 4. Layer 7d (SWITCH): Effects that switch P/T
    ///
    /// See `continuous_effects::PTBreakdown` for detailed implementation.
    ///
    /// ## Current Implementation Status
    ///
    /// - ✅ Layer 7a: Stubbed (no characteristic-defining abilities yet)
    /// - ✅ Layer 7b: Stubbed (no set P/T effects yet)
    /// - ✅ Layer 7c: Equipment bonuses + counter bonuses
    /// - ✅ Layer 7d: Stubbed (no switch effects yet)
    ///
    /// ## Returns
    ///
    /// Final power after applying all layers, or error if creature not found.
    pub fn get_effective_power(&self, creature_id: CardId) -> Result<i32> {
        let breakdown = self.get_pt_breakdown(creature_id)?;
        Ok(breakdown.power())
    }

    /// Get a creature's effective toughness using CR 613 layer system.
    ///
    /// See `get_effective_power()` for full documentation of the layer system.
    ///
    /// ## Returns
    ///
    /// Final toughness after applying all layers, or error if creature not found.
    pub fn get_effective_toughness(&self, creature_id: CardId) -> Result<i32> {
        let breakdown = self.get_pt_breakdown(creature_id)?;
        Ok(breakdown.toughness())
    }

    // TODO: Implement get_valid_targets function that filters game entities to find valid targets
    // based on effect type (damage, destroy, tap, etc.), targeting restrictions (hexproof,
    // shroud, protection), controller ownership, and zone requirements.

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
    /// - `mana_engine`: Pre-computed ManaEngine for mana payment (step 6)
    ///
    /// ## Java Forge Equivalent
    /// This matches `ComputerUtil.handlePlayingSpellAbility()` which:
    /// 1. Moves spell to stack (line 99)
    /// 2. Handles targeting
    /// 3. Pays costs with `CostPayment.payComputerCosts()` (line 125)
    pub fn cast_spell_8_step<TargetFn>(
        &mut self,
        player_id: PlayerId,
        card_id: CardId,
        mut choose_targets_fn: TargetFn,
        mana_engine: &crate::game::mana_engine::ManaEngine,
    ) -> Result<()>
    where
        TargetFn: FnMut(&GameState, CardId) -> Vec<CardId>,
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
        // Use the pre-computed ManaEngine to determine tap order
        use crate::game::mana_payment::{GreedyManaResolver, ManaPaymentResolver};

        let mana_sources = mana_engine.all_sources();
        let resolver = GreedyManaResolver::new();
        let mut sources_to_tap = Vec::new();

        if !resolver.compute_tap_order(&mana_cost, mana_sources, &mut sources_to_tap) {
            // Cannot pay the cost - unwind the spell cast
            self.move_card(card_id, Zone::Stack, Zone::Hand, player_id)?;
            return Err(MtgError::InvalidAction(format!(
                "Failed to pay mana cost {:?}: Insufficient mana",
                mana_cost
            )));
        }

        // Track which sources we've successfully tapped for unwinding if needed
        let mut tapped_sources = Vec::new();

        for &source_id in &sources_to_tap {
            if let Err(e) = self.tap_for_mana_for_cost(player_id, source_id, &mana_cost) {
                // Tapping failed - unwind the spell cast
                // Move card back to hand
                self.move_card(card_id, Zone::Stack, Zone::Hand, player_id)?;

                // Untap all sources that were successfully tapped so far
                for &tapped_id in &tapped_sources {
                    // Use helper that handles untap + undo log + mana version
                    let _ = self.untap_permanent(tapped_id);
                }

                // Clear the mana pool (remove any mana that was added)
                let player = self.get_player_mut(player_id)?;
                player.mana_pool.clear();

                return Err(MtgError::InvalidAction(format!("Failed to tap mana source: {e}")));
            }
            tapped_sources.push(source_id);
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
            for &source_id in &tapped_sources {
                // Use helper that handles untap + undo log + mana version
                let _ = self.untap_permanent(source_id);
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
                // Capture log size before life gain
                let prior_log_size = self.logger.log_count();

                let p = self.get_player_mut(*player)?;
                p.gain_life(*amount);

                // Log the life gain
                self.undo_log.log(
                    crate::undo::GameAction::ModifyLife {
                        player_id: *player,
                        delta: *amount,
                    },
                    prior_log_size,
                );
            }
            Effect::DestroyPermanent { target, .. } => {
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
                // Use helper that handles tap + undo log + mana version
                self.tap_permanent(*target)?;
            }
            Effect::UntapPermanent { target } => {
                // Use helper that handles untap + undo log + mana version
                self.untap_permanent(*target)?;
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
                // Capture log size before pump
                let prior_log_size = self.logger.log_count();

                let card = self.cards.get_mut(*target)?;
                card.power_bonus += power_bonus;
                card.toughness_bonus += toughness_bonus;

                // Log the pump effect
                self.undo_log.log(
                    crate::undo::GameAction::PumpCreature {
                        card_id: *target,
                        power_delta: *power_bonus,
                        toughness_delta: *toughness_bonus,
                    },
                    prior_log_size,
                );
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
                // Capture log size before mana addition
                let prior_log_size = self.logger.log_count();

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
                self.undo_log.log(
                    crate::undo::GameAction::AddMana {
                        player_id: *player,
                        mana: *mana,
                    },
                    prior_log_size,
                );
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
            Effect::SearchLibrary {
                player,
                card_type_filter,
                destination,
                enters_tapped,
                shuffle,
            } => {
                // Search library for a card matching the filter and move it to destination
                // MTG Rules 701.19a: To search a zone, a player looks at all cards in that zone

                // Get the library zone for the player
                let library_cards = self
                    .player_zones
                    .iter()
                    .find(|(id, _)| *id == *player)
                    .map(|(_, zones)| zones.library.cards.clone())
                    .ok_or_else(|| MtgError::InvalidAction(format!("Player {:?} has no library", player)))?;

                // Search for a card matching the filter
                // Filter format examples:
                // - "Land.Basic" = Land type + Basic subtype
                // - "Creature" = Any Creature
                // - "Plains,Island" = Land with Plains OR Island subtype (fetch lands)
                // - "Artifact.Equipment" = Artifact type + Equipment subtype
                let mut found_card = None;
                for &card_id in &library_cards {
                    if let Ok(card) = self.cards.get(card_id) {
                        let card_matches = Self::card_matches_search_filter(card, card_type_filter);

                        if card_matches {
                            found_card = Some(card_id);
                            break;
                        }
                    }
                }

                // If we found a matching card, move it to the destination
                if let Some(card_id) = found_card {
                    // Move the card from library to destination
                    self.move_card(card_id, Zone::Library, *destination, *player)?;

                    // If destination is battlefield and enters_tapped is true, tap the card
                    if *destination == Zone::Battlefield && *enters_tapped {
                        // Use helper that handles tap + undo log + mana version
                        let _ = self.tap_permanent(card_id);
                    }
                }

                // Shuffle the library if required (MTG Rules 701.19b)
                if *shuffle {
                    self.shuffle_library(*player);
                }
            }
            Effect::AttachEquipment {
                source_equipment,
                target_creature,
            } => {
                // Attach Equipment to target creature
                // Skip if target is still placeholder (0) - no valid targets found
                if target_creature.as_u32() == 0 {
                    // Ability fizzles - no valid targets
                    return Ok(());
                }

                // Call the attach_equipment method from Phase 1
                self.attach_equipment(*source_equipment, *target_creature)?;
            }
            Effect::CreateToken {
                controller,
                token_script,
                amount,
            } => {
                // Create token(s) on the battlefield
                // MTG Rules 111.2: The player who creates a token is its owner and controller

                // Look up token definition from cache (loaded during game initialization)
                let token_def = self.token_definitions.get(token_script).cloned();

                if let Some(token_def) = token_def {
                    // Use actual token definition from tokenscripts/
                    for _ in 0..*amount {
                        let token_id = self.next_card_id();

                        // Instantiate token from definition
                        let mut token = token_def.instantiate(token_id, *controller);

                        // Ensure controller is set correctly (owner and controller are the same for tokens)
                        token.controller = *controller;

                        // Add token to game
                        let token_name = token.name.to_string();
                        self.cards.insert(token_id, token);

                        // Put token onto the battlefield
                        self.battlefield.add(token_id);

                        // Log token creation
                        self.logger.normal(&format!(
                            "Created {} under {}'s control",
                            token_name,
                            self.get_player(*controller)?.name
                        ));
                    }
                } else {
                    // Token definition not found - this is an error
                    // The token should have been preloaded during game initialization
                    return Err(crate::MtgError::InvalidAction(format!(
                        "Token definition not found: '{}' (should have been preloaded)",
                        token_script
                    )));
                }
            }
        }
        Ok(())
    }

    /// Check if a card matches a library search filter
    ///
    /// Filter formats supported:
    /// - "Land.Basic" = Land type + Basic subtype (matches any basic land)
    /// - "Creature" = Any Creature type
    /// - "Plains,Island" = Land with Plains OR Island subtype (fetch lands)
    /// - "Artifact.Equipment" = Artifact type + Equipment subtype
    /// - "Forest" = Land with Forest subtype (single subtype)
    fn card_matches_search_filter(card: &crate::core::Card, filter: &str) -> bool {
        // Check if filter is comma-separated subtypes (e.g., "Plains,Island")
        // This is the format used by fetch lands
        if filter.contains(',') {
            // Parse as comma-separated subtypes
            // These are land subtypes, so check if card is a land and has any of the subtypes
            if !card.is_land() {
                return false;
            }

            let subtypes: Vec<&str> = filter.split(',').collect();
            return subtypes
                .iter()
                .any(|subtype| card.subtypes.iter().any(|st| st.as_str() == *subtype));
        }

        // Check if filter has type.subtype format (e.g., "Land.Basic")
        if filter.contains('.') {
            let parts: Vec<&str> = filter.split('.').collect();
            let main_type = parts.first().unwrap_or(&"Card");
            let subtype = parts.get(1);

            // Check if card matches the main type
            let type_matches = Self::card_matches_type(card, main_type);

            // Check if card matches the subtype (if specified)
            let subtype_matches = if let Some(sub) = subtype {
                Self::card_matches_subtype(card, sub)
            } else {
                true
            };

            return type_matches && subtype_matches;
        }

        // Single word filter - could be a type OR a subtype
        // First check if it's a known card type
        if matches!(
            filter,
            "Card" | "Land" | "Creature" | "Artifact" | "Enchantment" | "Instant" | "Sorcery" | "Planeswalker"
        ) {
            return Self::card_matches_type(card, filter);
        }

        // Otherwise treat as a subtype (e.g., "Forest", "Plains", "Island")
        // For land subtypes, also verify card is a land
        if matches!(filter, "Plains" | "Island" | "Swamp" | "Mountain" | "Forest") {
            return card.is_land() && Self::card_matches_subtype(card, filter);
        }

        // Generic subtype check
        Self::card_matches_subtype(card, filter)
    }

    /// Check if a card matches a card type
    fn card_matches_type(card: &crate::core::Card, type_name: &str) -> bool {
        match type_name {
            "Card" => true, // Any card
            "Land" => card.is_land(),
            "Creature" => card.is_creature(),
            "Artifact" => card.is_artifact(),
            "Enchantment" => card.is_enchantment(),
            "Instant" => card.is_instant(),
            "Sorcery" => card.types.contains(&CardType::Sorcery),
            "Planeswalker" => card.types.contains(&CardType::Planeswalker),
            _ => false,
        }
    }

    /// Check if a card matches a subtype
    fn card_matches_subtype(card: &crate::core::Card, subtype: &str) -> bool {
        if subtype == "Basic" {
            // "Basic" means any basic land subtype
            card.subtypes.iter().any(|st| {
                let st_str = st.as_str();
                st_str == "Plains"
                    || st_str == "Island"
                    || st_str == "Swamp"
                    || st_str == "Mountain"
                    || st_str == "Forest"
            })
        } else {
            // Check for specific subtype
            card.subtypes.iter().any(|st| st.as_str() == subtype)
        }
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
                        eprintln!(
                            "DEBUG: Found {} triggers on card {} ({})",
                            matching_triggers.len(),
                            card_id.as_u32(),
                            card.name
                        );
                        for effect in &matching_triggers {
                            eprintln!("  Trigger effect: {:?}", effect);
                        }
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
        for (trigger_source, effects) in triggered_effects {
            for mut effect in effects {
                // Fill in placeholder values in trigger effects
                // Similar to resolve_spell, we need to fill in targets
                match &mut effect {
                    Effect::DrawCards { player, .. } if player.as_u32() == 0 => {
                        // Placeholder player ID 0 means the controller of the trigger source
                        let controller = self.cards.get(trigger_source)?.controller;
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
                        let controller = self.cards.get(trigger_source)?.controller;
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
                        let controller = self.cards.get(trigger_source)?.controller;
                        effect = Effect::GainLife {
                            player: controller,
                            amount: *amount,
                        };
                    }
                    Effect::DestroyPermanent { target, restriction } if target.as_u32() == 0 => {
                        // Find a valid target (opponent's creature matching restriction)
                        let controller = self.cards.get(trigger_source)?.controller;
                        if let Some(target_id) = self
                            .battlefield
                            .cards
                            .iter()
                            .find(|&card_id| {
                                if let Ok(card) = self.cards.get(*card_id) {
                                    restriction.matches(card)
                                        && card.owner != controller
                                        && !card.has_hexproof()
                                        && !card.has_shroud()
                                } else {
                                    false
                                }
                            })
                            .copied()
                        {
                            effect = Effect::DestroyPermanent {
                                target: target_id,
                                restriction: restriction.clone(),
                            };
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
                    Effect::CreateToken {
                        controller,
                        token_script,
                        amount,
                    } if controller.as_u32() == 0 => {
                        // Placeholder player ID 0 means the controller of the trigger source
                        let source_controller = self.cards.get(source_card_id)?.controller;
                        effect = Effect::CreateToken {
                            controller: source_controller,
                            token_script: token_script.clone(),
                            amount: *amount,
                        };
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
            // Capture log size before life change
            let prior_log_size = self.logger.log_count();

            let player = self.get_player_mut(target_id)?;
            player.lose_life(amount);

            // Get new life total for logging
            let new_life = player.life;
            let player_name = player.name.clone();

            // Log the life change
            self.undo_log.log(
                crate::undo::GameAction::ModifyLife {
                    player_id: target_id,
                    delta: -amount,
                },
                prior_log_size,
            );

            // Log damage with new life total
            let message = format!("{} takes {} damage (life: {})", player_name, amount, new_life);
            self.logger.normal(&message);

            return Ok(());
        }

        Err(MtgError::InvalidAction("Invalid damage target".to_string()))
    }

    /// Deal damage to a creature
    ///
    /// MTG Rules 120.3: Damage dealt to a creature or planeswalker remains until the cleanup step
    /// MTG Rules 704.5g: State-based actions check if creature has lethal damage and destroys it
    pub fn deal_damage_to_creature(&mut self, target_id: CardId, amount: i32) -> Result<()> {
        // Get info about the creature first (without holding the borrow)
        let (is_creature, creature_name) = {
            let card = self.cards.get(target_id)?;
            (card.is_creature(), card.name.clone())
        };

        if is_creature {
            // Mark damage on the creature (MTG CR 120.3)
            // Damage persists until cleanup step (CR 704.5f)
            let card = self.cards.get_mut(target_id)?;
            card.damage += amount;

            let message = format!(
                "{} ({}) takes {} damage (total: {})",
                creature_name, target_id, amount, card.damage
            );
            self.logger.normal(&message);

            // Note: We don't destroy the creature here - that happens in state-based actions
            // This allows multiple damage sources to accumulate before checking lethal damage
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

    /// Tap a permanent for mana with a cost hint to guide color production
    ///
    /// This method handles both:
    /// - Lands with implicit mana abilities (based on subtypes)
    /// - Creatures/artifacts with explicit mana abilities (e.g., "Guy in the Chair")
    pub fn tap_for_mana_for_cost(
        &mut self,
        player_id: PlayerId,
        card_id: CardId,
        cost_hint: &crate::core::ManaCost,
    ) -> Result<()> {
        let card = self.cards.get_mut(card_id)?;

        // Check if card is untapped
        if card.tapped {
            return Err(MtgError::InvalidAction("Permanent is already tapped".to_string()));
        }

        // Check if card can produce mana (either land or has mana ability)
        let is_land = card.is_land();
        let has_mana_ability = card.activated_abilities.iter().any(|ab| ab.is_mana_ability);

        if !is_land && !has_mana_ability {
            return Err(MtgError::InvalidAction("Permanent cannot produce mana".to_string()));
        }

        // Get card name and check for explicit mana ability before tapping
        let card_name = card.name.to_lowercase();
        let explicit_mana = if !is_land && has_mana_ability {
            // For non-lands (creatures, artifacts) with mana abilities,
            // extract the mana from the activated ability's AddMana effect
            card.activated_abilities
                .iter()
                .find(|ab| ab.is_mana_ability)
                .and_then(|ab| {
                    ab.effects.iter().find_map(|effect| {
                        if let crate::core::Effect::AddMana { mana, .. } = effect {
                            Some(*mana)
                        } else {
                            None
                        }
                    })
                })
        } else {
            None
        };

        // Capture log size before tap
        let prior_log_size = self.logger.log_count();

        // Tap the permanent
        card.tap();

        // Log the tap
        self.undo_log.log(
            crate::undo::GameAction::TapCard { card_id, tapped: true },
            prior_log_size,
        );

        // Update mana caches (event-driven incremental update)
        // Read card data to avoid borrow conflicts
        if let Some(card) = self.cards.try_get(card_id) {
            for (_, cache) in &mut self.mana_caches {
                cache.on_tap(card_id, card);
            }
        }

        // Increment mana state version to invalidate ManaEngine cache
        self.increment_mana_version();

        // Handle non-land mana sources with explicit mana abilities
        if let Some(mana_to_add) = explicit_mana {
            // For creatures with "Add mana of any color", we need to choose based on cost hint
            // Check if this is an any-color source using the pre-computed cache
            // (derived from parsed abilities, not text)
            let is_any_color = self
                .cards
                .get(card_id)
                .map(|c| matches!(c.cache.mana_production.kind, crate::core::ManaProductionKind::AnyColor))
                .unwrap_or(false);

            // Capture log size before mana addition (before get_player_mut to avoid borrow issues)
            let prior_log_size = self.logger.log_count();

            let player = self.get_player_mut(player_id)?;

            if is_any_color {
                // Choose color based on cost hint
                let color = if cost_hint.white > 0 {
                    crate::core::Color::White
                } else if cost_hint.blue > 0 {
                    crate::core::Color::Blue
                } else if cost_hint.black > 0 {
                    crate::core::Color::Black
                } else if cost_hint.red > 0 {
                    crate::core::Color::Red
                } else if cost_hint.green > 0 {
                    crate::core::Color::Green
                } else {
                    // Default to green if no specific color needed
                    crate::core::Color::Green
                };

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
                self.undo_log
                    .log(crate::undo::GameAction::AddMana { player_id, mana }, prior_log_size);

                // Log visible message
                if self.logger.verbosity() >= crate::game::VerbosityLevel::Normal {
                    let card = self.cards.get(card_id).ok();
                    let name = card.map(|c| c.name.as_str()).unwrap_or("Unknown");
                    let message = format!("Tap {} for {{{}}}", name, color_symbol);
                    self.logger.normal(&message);
                }
            } else {
                // Add the specific mana from the ability
                if mana_to_add.white > 0 {
                    player.mana_pool.white += mana_to_add.white;
                }
                if mana_to_add.blue > 0 {
                    player.mana_pool.blue += mana_to_add.blue;
                }
                if mana_to_add.black > 0 {
                    player.mana_pool.black += mana_to_add.black;
                }
                if mana_to_add.red > 0 {
                    player.mana_pool.red += mana_to_add.red;
                }
                if mana_to_add.green > 0 {
                    player.mana_pool.green += mana_to_add.green;
                }
                if mana_to_add.colorless > 0 {
                    player.mana_pool.colorless += mana_to_add.colorless;
                }

                self.undo_log.log(
                    crate::undo::GameAction::AddMana {
                        player_id,
                        mana: mana_to_add,
                    },
                    prior_log_size,
                );

                // Log visible message
                if self.logger.verbosity() >= crate::game::VerbosityLevel::Normal {
                    let card = self.cards.get(card_id).ok();
                    let name = card.map(|c| c.name.as_str()).unwrap_or("Unknown");
                    let message = format!("Tap {} for mana", name);
                    self.logger.normal(&message);
                }
            }

            return Ok(());
        }

        // Add mana to player's pool based on land type
        // For basic lands and simple cases, check subtypes
        // For dual lands (e.g., Underground Sea = Island Swamp), we need smarter logic
        // First, check subtypes and mana production cache before we borrow player_mut
        let (
            has_swamp_subtype,
            has_mountain_subtype,
            has_island_subtype,
            has_forest_subtype,
            has_plains_subtype,
            is_any_color_land,
            produces_colorless,
        ) = {
            let card = self.cards.get(card_id)?;
            // Use the pre-computed cache for mana production type (derived from abilities, not text)
            let is_any_color = matches!(
                card.cache.mana_production.kind,
                crate::core::ManaProductionKind::AnyColor
            );
            let is_colorless = matches!(
                card.cache.mana_production.kind,
                crate::core::ManaProductionKind::Colorless
            );
            (
                card.subtypes.iter().any(|s| s.as_str().eq_ignore_ascii_case("swamp")),
                card.subtypes
                    .iter()
                    .any(|s| s.as_str().eq_ignore_ascii_case("mountain")),
                card.subtypes.iter().any(|s| s.as_str().eq_ignore_ascii_case("island")),
                card.subtypes.iter().any(|s| s.as_str().eq_ignore_ascii_case("forest")),
                card.subtypes.iter().any(|s| s.as_str().eq_ignore_ascii_case("plains")),
                is_any_color,
                is_colorless,
            )
        };

        // Capture log size before mana addition (before get_player_mut to avoid borrow issues)
        let prior_log_size = self.logger.log_count();

        let player = self.get_player_mut(player_id)?;

        // Determine what colors this land can produce
        let mut available_colors = Vec::new();
        if has_plains_subtype || card_name.contains("plains") {
            available_colors.push(crate::core::Color::White);
        }
        if has_island_subtype || card_name.contains("island") {
            available_colors.push(crate::core::Color::Blue);
        }
        if has_swamp_subtype || card_name.contains("swamp") {
            available_colors.push(crate::core::Color::Black);
        }
        if has_mountain_subtype || card_name.contains("mountain") {
            available_colors.push(crate::core::Color::Red);
        }
        if has_forest_subtype || card_name.contains("forest") {
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
        } else if produces_colorless {
            // Colorless mana land (e.g., Mishra's Factory, Wastes)
            Some(crate::core::Color::Colorless)
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
            self.undo_log
                .log(crate::undo::GameAction::AddMana { player_id, mana }, prior_log_size);

            // Log visible message for mana tapping
            if self.logger.verbosity() >= crate::game::VerbosityLevel::Normal {
                let card_name = self.cards.get(card_id).map(|c| c.name.as_str()).unwrap_or("Unknown");
                let message = format!("Tap {} for {{{}}}", card_name, color_symbol);
                self.logger.normal(&message);
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
                // Tap the permanent (this updates cache and increments mana_version)
                self.tap_permanent(card_id)?;
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
                // Check if already tapped
                {
                    let card = self.cards.get(card_id)?;
                    if card.tapped {
                        return Err(MtgError::InvalidAction("Permanent is already tapped".to_string()));
                    }
                }

                // Tap the permanent (this updates cache and increments mana_version)
                self.tap_permanent(card_id)?;

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

// Submodules
mod combat;
mod targeting;

#[cfg(test)]
mod tests;
