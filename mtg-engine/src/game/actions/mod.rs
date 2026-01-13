//! Game actions and mechanics

use crate::core::{CardId, CardType, Effect, PlayerId, TargetRef, TriggerEvent};
use crate::game::GameState;
use crate::zones::Zone;
use crate::{MtgError, Result};

impl GameState {
    /// Play a land from hand to battlefield
    ///
    /// Per NETWORK_ARCHITECTURE.md, cards are revealed to ALL players before moving
    /// to battlefield (which is a public zone).
    ///
    /// # Errors
    ///
    /// Returns an error if the player cannot play more lands, the card is not a land,
    /// or the card is not in hand.
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

        // Reveal to all players before logging movement
        // (playing a land reveals to all since battlefield is public)
        let prior_log_size = self.logger.log_count();
        self.maybe_reveal_to_all(card_id, prior_log_size);

        // Move card to battlefield (move_card logs the MoveCard action)
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
    ///
    /// Per NETWORK_ARCHITECTURE.md, cards are revealed to ALL players before moving
    /// to stack (which is a public zone).
    ///
    /// # Errors
    ///
    /// Returns an error if the card is not in hand or if insufficient mana to pay the cost.
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

        // Pay the mana cost (from both regular and combat mana pools)
        let player = self.get_player_mut(player_id)?;
        player
            .pay_from_total_mana(&mana_cost)
            .map_err(MtgError::InvalidAction)?;

        // Reveal to all players before logging movement
        // (casting a spell reveals to all since stack is public)
        let prior_log_size = self.logger.log_count();
        self.maybe_reveal_to_all(card_id, prior_log_size);

        // Move card to stack (move_card logs the MoveCard action)
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
    ///
    /// # Errors
    ///
    /// Returns an error if the card is not found or if spell resolution fails.
    pub fn resolve_spell(&mut self, card_id: CardId, chosen_targets: &[CardId]) -> Result<()> {
        // Get card owner and effects count (without cloning effects)
        let (card_owner, effects_len) = {
            let card = self.cards.get(card_id)?;
            (card.owner, card.effects.len())
        };

        log::debug!(target: "resolve_spell", "resolve_spell card_id={}, chosen_targets={:?}, effects_len={}", card_id.as_u32(), chosen_targets.iter().map(|c| c.as_u32()).collect::<Vec<_>>(), effects_len);

        // Find opponent ID for untargeted damage (resolve once)
        let opponent_id = self.players.iter().map(|p| p.id).find(|id| *id != card_owner);

        // Check if targets are still valid before executing effects
        // MTG Rules 608.2b: If all targets are illegal, the spell doesn't resolve
        let all_targets_illegal = if !chosen_targets.is_empty() {
            // Check if any permanent target is no longer on the battlefield
            // This handles spells that target permanents
            let any_permanent_gone = chosen_targets
                .iter()
                .any(|&target_id| !self.battlefield.contains(target_id) && !self.stack.contains(target_id));

            // If spell has targets and they're all gone, it fizzles
            any_permanent_gone
        } else {
            false
        };

        // Execute effects only if targets are still valid
        if !all_targets_illegal {
            // Execute effects by index, resolving targets at execution time
            // This avoids cloning the entire Vec<Effect>
            let mut target_index = 0;
            let mut last_resolved_target: Option<CardId> = None;
            for effect_index in 0..effects_len {
                // Re-fetch effect each iteration (card ref can't be held across execute calls)
                let effect = self.cards.get(card_id)?.effects.get(effect_index).cloned();

                if let Some(effect) = effect {
                    log::debug!(target: "resolve_spell", "Effect[{}] before resolve: {:?}", effect_index, effect);
                    // Resolve the effect with context, advancing target_index as needed
                    let resolved = self.resolve_effect_target(
                        &effect,
                        chosen_targets,
                        &mut target_index,
                        card_owner,
                        opponent_id,
                        &mut last_resolved_target,
                    );
                    log::debug!(target: "resolve_spell", "Effect[{}] after resolve: {:?}", effect_index, resolved);
                    self.execute_effect(&resolved)?;
                }
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

            // MTG Rule 303.4a: An Aura spell that resolves attaches to its target
            // The target was already chosen and validated when casting the Aura
            let is_aura = self.cards.get(card_id).map(|c| c.is_aura()).unwrap_or(false);
            if is_aura && !chosen_targets.is_empty() {
                let aura_target = chosen_targets[0];
                // Attach the Aura to its target (if target is still valid)
                if self.battlefield.contains(aura_target) {
                    self.attach_aura(card_id, aura_target)?;
                } else {
                    // Target became invalid - move Aura to graveyard (CR 303.4a)
                    self.move_card(card_id, Zone::Battlefield, Zone::Graveyard, card_owner)?;
                }
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
    ///
    /// # Errors
    ///
    /// Returns an error if the equipment/target is not on battlefield,
    /// the card is not equipment/aura, or target is not a valid creature.
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

    /// Attach Aura to a target card
    ///
    /// This is called when an Aura spell resolves and enters the battlefield.
    ///
    /// ## Rules Implementation (CR 303.4)
    /// - Auras can attach to any legal target (including opponent's creatures)
    /// - The target is determined by the "enchant" keyword (e.g., "Enchant creature")
    /// - If already attached, detaches from previous target first
    ///
    /// # Errors
    ///
    /// Returns an error if the aura/target is not on battlefield, or the card is not an aura.
    pub fn attach_aura(&mut self, aura_id: CardId, target_id: CardId) -> Result<()> {
        // Validate Aura is on battlefield
        if !self.battlefield.contains(aura_id) {
            return Err(MtgError::InvalidAction(
                "Aura must be on battlefield to attach".to_string(),
            ));
        }

        // Validate target is on battlefield
        if !self.battlefield.contains(target_id) {
            return Err(MtgError::InvalidAction("Target must be on battlefield".to_string()));
        }

        // Get Aura and target
        let aura = self.cards.get(aura_id)?;
        if !aura.is_aura() {
            return Err(MtgError::InvalidAction(
                "Only Auras can be attached via attach_aura".to_string(),
            ));
        }
        let aura_name = aura.name.to_string();

        // Validate target type based on enchant restriction
        // For now, assume "Enchant creature" (most common case)
        // TODO: Parse enchant restriction from KeywordArgs::Enchant
        let target = self.cards.get(target_id)?;
        if !target.is_creature() {
            return Err(MtgError::InvalidAction(
                "This Aura can only enchant creatures".to_string(),
            ));
        }

        // Detach from previous target if needed (unlikely for newly-resolved Aura)
        let aura = self.cards.get_mut(aura_id)?;
        if let Some(old_target) = aura.attached_to {
            let old_target_name = self
                .cards
                .get(old_target)
                .map(|c| c.name.to_string())
                .unwrap_or_else(|_| "unknown".to_string());
            self.logger
                .verbose(&format!("{} detaches from {}", aura_name, old_target_name));
        }

        // Attach to new target
        // Capture old value and log size before mutation
        let old_target = self.cards.get(aura_id)?.attached_to;
        let prior_log_size = self.logger.log_count();

        let aura = self.cards.get_mut(aura_id)?;
        let new_target = Some(target_id);
        aura.attached_to = new_target;

        // Log the mutation for undo
        self.undo_log.log(
            crate::undo::GameAction::SetAttachedTo {
                equipment_id: aura_id,
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
        self.logger.gamelog(&format!("{} enchants {}", aura_name, target_name));

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
    ///
    /// # Errors
    ///
    /// Returns an error if the equipment cannot be found.
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
    ///
    /// # Errors
    ///
    /// Returns an error if the creature cannot be found.
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
    ///
    /// # Errors
    ///
    /// Returns an error if the creature cannot be found.
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
    ///
    /// # Errors
    ///
    /// Returns an error if the card is not in hand, cannot move to stack, or mana payment fails.
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

        // Reveal to all players before logging movement
        // (casting a spell reveals to all since stack is public)
        let prior_log_size = self.logger.log_count();
        self.maybe_reveal_to_all(card_id, prior_log_size);

        // Step 1: Propose the spell - move card to stack (move_card logs the MoveCard action)
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
        //
        // IMPORTANT: Check if the player already has floating mana in their pool
        // (e.g., from Dark Ritual). We should use that first, then tap sources
        // only for the remaining cost.
        use crate::core::ManaCost;
        use crate::game::mana_payment::{GreedyManaResolver, ManaPaymentResolver};

        // Get total available mana (regular pool + combat mana from Firebending)
        // Combat mana lasts until end of combat and can be used for spells
        let current_pool = self.get_player(player_id)?.total_available_mana();

        // Calculate the remaining cost after using pool mana
        // First satisfy colored requirements from pool, then generic
        let remaining_white = mana_cost.white.saturating_sub(current_pool.white);
        let remaining_blue = mana_cost.blue.saturating_sub(current_pool.blue);
        let remaining_black = mana_cost.black.saturating_sub(current_pool.black);
        let remaining_red = mana_cost.red.saturating_sub(current_pool.red);
        let remaining_green = mana_cost.green.saturating_sub(current_pool.green);
        let remaining_colorless = mana_cost.colorless.saturating_sub(current_pool.colorless);

        // Calculate pool mana used for colored requirements
        let used_white = mana_cost.white.min(current_pool.white);
        let used_blue = mana_cost.blue.min(current_pool.blue);
        let used_black = mana_cost.black.min(current_pool.black);
        let used_red = mana_cost.red.min(current_pool.red);
        let used_green = mana_cost.green.min(current_pool.green);
        let used_colorless = mana_cost.colorless.min(current_pool.colorless);

        // Pool mana remaining after colored requirements can be used for generic
        let pool_for_generic = (current_pool.white.saturating_sub(used_white))
            + (current_pool.blue.saturating_sub(used_blue))
            + (current_pool.black.saturating_sub(used_black))
            + (current_pool.red.saturating_sub(used_red))
            + (current_pool.green.saturating_sub(used_green))
            + (current_pool.colorless.saturating_sub(used_colorless));

        let remaining_generic = mana_cost.generic.saturating_sub(pool_for_generic);

        // Create the remaining cost that must be paid by tapping sources
        let remaining_cost = ManaCost {
            generic: remaining_generic,
            white: remaining_white,
            blue: remaining_blue,
            black: remaining_black,
            red: remaining_red,
            green: remaining_green,
            colorless: remaining_colorless,
            x_count: 0,
        };

        let mana_sources = mana_engine.all_sources();
        let resolver = GreedyManaResolver::new();
        let mut sources_to_tap = Vec::new();

        // Only compute tap order for the remaining cost (after pool mana is used)
        // If remaining cost is zero, we don't need to tap any sources
        if remaining_cost.cmc() > 0 && !resolver.compute_tap_order(&remaining_cost, mana_sources, &mut sources_to_tap) {
            // Cannot pay the cost - unwind the spell cast
            self.move_card(card_id, Zone::Stack, Zone::Hand, player_id)?;
            return Err(MtgError::InvalidAction(format!(
                "Failed to pay mana cost {:?}: Insufficient mana",
                mana_cost
            )));
        }

        // Track which sources we've successfully tapped for unwinding if needed
        let mut tapped_sources = Vec::new();

        // Track remaining cost as hint for each land tap
        // This ensures dual lands produce the right color based on what's still needed
        let mut remaining_hint = remaining_cost;

        for &source_id in &sources_to_tap {
            if let Err(e) = self.tap_for_mana_for_cost(player_id, source_id, &remaining_hint) {
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

            // Update remaining hint based on what color this source produced
            // Check mana production kind to know what color was produced
            if let Some(card) = self.cards.try_get(source_id) {
                match &card.cache.mana_production.kind {
                    crate::core::ManaProductionKind::Fixed(color) => {
                        // Deduct the fixed color from remaining hint
                        match color {
                            crate::core::ManaColor::White => {
                                remaining_hint.white = remaining_hint.white.saturating_sub(1);
                            }
                            crate::core::ManaColor::Blue => {
                                remaining_hint.blue = remaining_hint.blue.saturating_sub(1);
                            }
                            crate::core::ManaColor::Black => {
                                remaining_hint.black = remaining_hint.black.saturating_sub(1);
                            }
                            crate::core::ManaColor::Red => {
                                remaining_hint.red = remaining_hint.red.saturating_sub(1);
                            }
                            crate::core::ManaColor::Green => {
                                remaining_hint.green = remaining_hint.green.saturating_sub(1);
                            }
                        }
                    }
                    crate::core::ManaProductionKind::Colorless => {
                        // Colorless reduces colorless or generic
                        if remaining_hint.colorless > 0 {
                            remaining_hint.colorless = remaining_hint.colorless.saturating_sub(1);
                        } else {
                            remaining_hint.generic = remaining_hint.generic.saturating_sub(1);
                        }
                    }
                    crate::core::ManaProductionKind::Choice(_) | crate::core::ManaProductionKind::AnyColor => {
                        // For choice/any-color lands, we produced the first needed color
                        // Deduct in same priority order as tap_for_mana_for_cost
                        if remaining_hint.white > 0 {
                            remaining_hint.white = remaining_hint.white.saturating_sub(1);
                        } else if remaining_hint.blue > 0 {
                            remaining_hint.blue = remaining_hint.blue.saturating_sub(1);
                        } else if remaining_hint.black > 0 {
                            remaining_hint.black = remaining_hint.black.saturating_sub(1);
                        } else if remaining_hint.red > 0 {
                            remaining_hint.red = remaining_hint.red.saturating_sub(1);
                        } else if remaining_hint.green > 0 {
                            remaining_hint.green = remaining_hint.green.saturating_sub(1);
                        } else {
                            remaining_hint.generic = remaining_hint.generic.saturating_sub(1);
                        }
                    }
                }
            }
        }

        // Step 7: Pay costs (from both regular and combat mana pools)
        let player = self.get_player_mut(player_id)?;
        if let Err(e) = player.pay_from_total_mana(&mana_cost) {
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
        // Trigger "whenever you cast a spell" abilities (like Boar-q-pine, Prowess)
        // MTG Rules 601.2i: The spell becomes cast once all costs are paid
        self.check_spellcast_triggers(card_id, player_id)?;

        Ok(())
    }

    /// Resolve an effect's placeholder targets and player IDs
    ///
    /// This helper function resolves placeholder values (target ID 0, player ID 0) in effects
    /// without requiring a clone of the entire Vec<Effect>. It returns a resolved copy of the
    /// effect with targets filled in from the provided context.
    ///
    /// ## Parameters
    /// - `effect`: The effect to resolve (borrowed)
    /// - `chosen_targets`: Slice of targets chosen during spell casting
    /// - `target_index`: Mutable index tracking which target to consume next
    /// - `card_owner`: The controller of the spell (for "you" player references)
    /// - `opponent_id`: Pre-computed opponent ID for untargeted damage effects
    /// - `last_resolved_target`: Tracks the most recently resolved target for SubAbility chains
    ///   with `Defined$ Targeted` (reuse_previous sentinel)
    ///
    /// Note: Wildcard match is intentional - effects without placeholder targets
    /// are returned unchanged. New Effect variants should be reviewed for target
    /// resolution needs.
    #[inline]
    #[allow(clippy::wildcard_enum_match_arm)]
    fn resolve_effect_target(
        &self,
        effect: &Effect,
        chosen_targets: &[CardId],
        target_index: &mut usize,
        card_owner: PlayerId,
        opponent_id: Option<PlayerId>,
        last_resolved_target: &mut Option<CardId>,
    ) -> Effect {
        match effect {
            // Target resolution for permanent-targeting effects
            Effect::DealDamage {
                target: TargetRef::None,
                amount,
            } => {
                if *target_index < chosen_targets.len() {
                    let target = chosen_targets[*target_index];
                    *target_index += 1;
                    *last_resolved_target = Some(target);
                    Effect::DealDamage {
                        target: TargetRef::Permanent(target),
                        amount: *amount,
                    }
                } else if let Some(opp) = opponent_id {
                    // Default to opponent for untargeted damage
                    Effect::DealDamage {
                        target: TargetRef::Player(opp),
                        amount: *amount,
                    }
                } else {
                    effect.clone()
                }
            }
            Effect::DestroyPermanent { target, restriction } if target.as_u32() == 0 => {
                if *target_index < chosen_targets.len() {
                    let resolved_target = chosen_targets[*target_index];
                    *target_index += 1;
                    *last_resolved_target = Some(resolved_target);
                    Effect::DestroyPermanent {
                        target: resolved_target,
                        restriction: restriction.clone(),
                    }
                } else {
                    effect.clone()
                }
            }
            Effect::PumpCreature {
                target,
                power_bonus,
                toughness_bonus,
                keywords_granted,
            } if target.as_u32() == 0 => {
                if *target_index < chosen_targets.len() {
                    let resolved_target = chosen_targets[*target_index];
                    *target_index += 1;
                    *last_resolved_target = Some(resolved_target);
                    Effect::PumpCreature {
                        target: resolved_target,
                        power_bonus: *power_bonus,
                        toughness_bonus: *toughness_bonus,
                        keywords_granted: keywords_granted.clone(),
                    }
                } else {
                    effect.clone()
                }
            }
            Effect::TapPermanent { target } if target.as_u32() == 0 => {
                if *target_index < chosen_targets.len() {
                    let resolved_target = chosen_targets[*target_index];
                    *target_index += 1;
                    *last_resolved_target = Some(resolved_target);
                    Effect::TapPermanent {
                        target: resolved_target,
                    }
                } else {
                    effect.clone()
                }
            }
            // Handle UntapPermanent with reuse_previous sentinel (from Defined$ Targeted)
            Effect::UntapPermanent { target } if target.is_reuse_previous() => {
                // Reuse the target from the previous effect in the chain
                if let Some(prev_target) = *last_resolved_target {
                    Effect::UntapPermanent { target: prev_target }
                } else {
                    log::warn!(target: "resolve_effect", "UntapPermanent has reuse_previous but no previous target");
                    effect.clone()
                }
            }
            Effect::UntapPermanent { target } if target.as_u32() == 0 => {
                if *target_index < chosen_targets.len() {
                    let resolved_target = chosen_targets[*target_index];
                    *target_index += 1;
                    *last_resolved_target = Some(resolved_target);
                    Effect::UntapPermanent {
                        target: resolved_target,
                    }
                } else {
                    effect.clone()
                }
            }
            Effect::CounterSpell { target } if target.as_u32() == 0 => {
                if *target_index < chosen_targets.len() {
                    let resolved_target = chosen_targets[*target_index];
                    *target_index += 1;
                    Effect::CounterSpell {
                        target: resolved_target,
                    }
                } else {
                    effect.clone()
                }
            }
            Effect::ExilePermanent { target } if target.as_u32() == 0 => {
                if *target_index < chosen_targets.len() {
                    let resolved_target = chosen_targets[*target_index];
                    *target_index += 1;
                    Effect::ExilePermanent {
                        target: resolved_target,
                    }
                } else {
                    effect.clone()
                }
            }
            // Player ID resolution for player-targeting effects
            Effect::DrawCards { player, count } if player.as_u32() == 0 => Effect::DrawCards {
                player: card_owner,
                count: *count,
            },
            Effect::GainLife { player, amount } if player.as_u32() == 0 => Effect::GainLife {
                player: card_owner,
                amount: *amount,
            },
            Effect::Mill { player, count } if player.as_u32() == 0 => Effect::Mill {
                player: card_owner,
                count: *count,
            },
            Effect::Loot {
                player,
                discard_count,
                draw_count,
            } if player.as_u32() == 0 => Effect::Loot {
                player: card_owner,
                discard_count: *discard_count,
                draw_count: *draw_count,
            },
            Effect::AddMana {
                player,
                mana,
                produces_chosen_color,
            } if player.as_u32() == 0 => Effect::AddMana {
                player: card_owner,
                mana: *mana,
                produces_chosen_color: *produces_chosen_color,
            },
            // Earthbend: Target land becomes 0/0 creature with haste
            Effect::Earthbend { target, num_counters } if target.as_u32() == 0 => {
                if *target_index < chosen_targets.len() {
                    let resolved_target = chosen_targets[*target_index];
                    *target_index += 1;
                    Effect::Earthbend {
                        target: resolved_target,
                        num_counters: *num_counters,
                    }
                } else {
                    effect.clone()
                }
            }
            // Airbend: Exile target, owner may cast for {2}
            Effect::Airbend { target } if target.as_u32() == 0 => {
                if *target_index < chosen_targets.len() {
                    let resolved_target = chosen_targets[*target_index];
                    *target_index += 1;
                    Effect::Airbend {
                        target: resolved_target,
                    }
                } else {
                    effect.clone()
                }
            }
            // RemoveCounter: Remove counters from target permanent
            Effect::RemoveCounter {
                target,
                counter_type,
                amount,
            } if target.as_u32() == 0 => {
                if *target_index < chosen_targets.len() {
                    let resolved_target = chosen_targets[*target_index];
                    *target_index += 1;
                    Effect::RemoveCounter {
                        target: resolved_target,
                        counter_type: *counter_type,
                        amount: *amount,
                    }
                } else {
                    effect.clone()
                }
            }
            // PutCounter: Put counters on target permanent
            Effect::PutCounter {
                target,
                counter_type,
                amount,
            } if target.as_u32() == 0 => {
                if *target_index < chosen_targets.len() {
                    let resolved_target = chosen_targets[*target_index];
                    *target_index += 1;
                    Effect::PutCounter {
                        target: resolved_target,
                        counter_type: *counter_type,
                        amount: *amount,
                    }
                } else {
                    effect.clone()
                }
            }
            // CopyPermanent: Create token copy of target permanent
            Effect::CopyPermanent {
                target,
                controller,
                non_legendary,
                set_power,
                set_toughness,
                add_types,
                num_copies,
                restriction,
            } if target.as_u32() == 0 => {
                if *target_index < chosen_targets.len() {
                    let resolved_target = chosen_targets[*target_index];
                    *target_index += 1;
                    Effect::CopyPermanent {
                        target: resolved_target,
                        controller: if controller.as_u32() == 0 {
                            card_owner
                        } else {
                            *controller
                        },
                        non_legendary: *non_legendary,
                        set_power: *set_power,
                        set_toughness: *set_toughness,
                        add_types: add_types.clone(),
                        num_copies: *num_copies,
                        restriction: restriction.clone(),
                    }
                } else {
                    effect.clone()
                }
            }
            // CreateDelayedTrigger: fill in tracked_card from chosen_targets
            // This is for spells like Fatal Fissure that target a creature and create
            // a delayed trigger for when that creature dies
            Effect::CreateDelayedTrigger {
                tracked_card,
                condition,
                effect: delayed_effect,
                expiry,
            } if tracked_card.as_u32() == 0 => {
                if *target_index < chosen_targets.len() {
                    let resolved_target = chosen_targets[*target_index];
                    *target_index += 1;
                    *last_resolved_target = Some(resolved_target);
                    Effect::CreateDelayedTrigger {
                        tracked_card: resolved_target,
                        condition: condition.clone(),
                        effect: delayed_effect.clone(),
                        expiry: expiry.clone(),
                    }
                } else {
                    effect.clone()
                }
            }
            // No resolution needed - return clone of original
            _ => effect.clone(),
        }
    }

    /// Execute a single effect
    ///
    /// # Errors
    ///
    /// Returns an error if the effect cannot be executed (e.g., invalid target).
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
            Effect::Loot {
                player,
                discard_count,
                draw_count,
            } => {
                // Looting: discard first, then draw
                // Use AI to choose what to discard
                for _ in 0..*discard_count {
                    let card_to_discard = self.choose_card_to_discard(*player)?;
                    if let Some(card_id) = card_to_discard {
                        self.discard_card(*player, card_id)?;
                    } else {
                        // No cards to discard, can't complete the loot
                        break;
                    }
                }
                for _ in 0..*draw_count {
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
                    // Check death triggers BEFORE moving the card (trigger still has access to card data)
                    let _ = self.check_death_triggers(*target);
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
                keywords_granted,
            } => {
                // Skip if target is still placeholder (0) - no valid targets found
                if target.as_u32() == 0 {
                    // Spell fizzles - no valid targets
                    log::warn!(target: "pump", "PumpCreature fizzled: target is still placeholder 0");
                    return Ok(());
                }
                log::debug!(target: "pump", "PumpCreature executing: target={}, power_bonus={}, toughness_bonus={}, keywords={:?}", target.as_u32(), power_bonus, toughness_bonus, keywords_granted);
                // Capture log size before pump
                let prior_log_size = self.logger.log_count();

                let card = self.cards.get_mut(*target)?;
                card.power_bonus += power_bonus;
                card.toughness_bonus += toughness_bonus;
                // Grant keywords
                for keyword in keywords_granted.iter() {
                    card.keywords.insert(*keyword);
                }

                // Log the pump effect
                self.undo_log.log(
                    crate::undo::GameAction::PumpCreature {
                        card_id: *target,
                        power_delta: *power_bonus,
                        toughness_delta: *toughness_bonus,
                        keywords_granted: keywords_granted.clone(),
                    },
                    prior_log_size,
                );
            }
            Effect::PumpAllCreatures {
                controller,
                filter,
                power_bonus,
                toughness_bonus,
            } => {
                // Mass pump: "Creatures you control get +X/+Y until end of turn"
                // Find all creatures matching the filter and pump them
                let targets: Vec<CardId> = self
                    .battlefield
                    .cards
                    .iter()
                    .filter_map(|&card_id| {
                        if let Ok(card) = self.cards.get(card_id) {
                            // Check if it's a creature
                            if !card.is_creature() {
                                return None;
                            }
                            // Check filter: "Creature.YouCtrl" means controller's creatures
                            if filter.contains("YouCtrl") && card.controller != *controller {
                                return None;
                            }
                            // Check filter: "Creature.OppCtrl" means opponent's creatures
                            if filter.contains("OppCtrl") && card.controller == *controller {
                                return None;
                            }
                            Some(card_id)
                        } else {
                            None
                        }
                    })
                    .collect();

                // Apply pump to all matching creatures
                for target in targets {
                    let prior_log_size = self.logger.log_count();
                    if let Ok(card) = self.cards.get_mut(target) {
                        card.power_bonus += power_bonus;
                        card.toughness_bonus += toughness_bonus;
                        log::debug!(
                            "PumpAllCreatures: {} gets +{}/+{}",
                            card.name,
                            power_bonus,
                            toughness_bonus
                        );
                    }
                    self.undo_log.log(
                        crate::undo::GameAction::PumpCreature {
                            card_id: target,
                            power_delta: *power_bonus,
                            toughness_delta: *toughness_bonus,
                            keywords_granted: smallvec::SmallVec::new(),
                        },
                        prior_log_size,
                    );
                }
            }
            Effect::Mill { player, count } => {
                // Mill cards from library to graveyard
                self.mill_cards(*player, *count)?;
            }
            Effect::CounterSpell { target } => {
                // Counter a spell on the stack
                self.counter_spell(*target)?;
            }
            Effect::AddMana {
                player,
                mana,
                produces_chosen_color,
            } => {
                // Capture log size before mana addition
                let prior_log_size = self.logger.log_count();

                // Add mana to player's mana pool
                // Note: For mana abilities, produces_chosen_color is handled in tap_for_mana_for_cost
                // where we have access to the source card's chosen_color.
                // This path is mainly for spell effects (Dark Ritual) and triggered abilities (Su-Chi).
                if *produces_chosen_color {
                    // This shouldn't happen in practice since mana abilities go through tap_for_mana_for_cost
                    // but log a warning if it does
                    self.logger
                        .normal("Warning: produces_chosen_color in execute_effect - source card unknown");
                }
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
                if let Some(ct) = counter_type {
                    // Specific counter type
                    self.remove_counters(*target, *ct, *amount)?;
                } else {
                    // CounterType$ Any - remove counters of any type
                    // Get all counter types present on the card and remove up to `amount` total
                    let mut remaining = *amount;
                    let counter_types: smallvec::SmallVec<[crate::core::CounterType; 4]> = {
                        let card = self.cards.get(*target)?;
                        card.counters.iter().map(|(ct, _)| *ct).collect()
                    };

                    for ct in counter_types {
                        if remaining == 0 {
                            break;
                        }
                        let removed = self.remove_counters(*target, ct, remaining)?;
                        remaining = remaining.saturating_sub(removed);
                    }
                }
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
            Effect::SetBasePowerToughness {
                target,
                power,
                toughness,
            } => {
                // Skip if target is still placeholder (0) - no valid targets found
                if target.as_u32() == 0 {
                    // Spell fizzles - no valid targets
                    return Ok(());
                }
                // Set temporary base P/T override (until end of turn)
                // This is used by Animate effects like Flexible Waterbender
                let card = self.cards.get_mut(*target)?;
                let card_name = card.name.clone();
                let old_power = card.current_power();
                let old_toughness = card.current_toughness();

                card.set_temp_base_power(*power as i8);
                card.set_temp_base_toughness(*toughness as i8);

                let new_power = card.current_power();
                let new_toughness = card.current_toughness();

                // Log the effect
                if self.logger.verbosity() >= crate::game::VerbosityLevel::Normal {
                    self.logger.gamelog(&format!(
                        "{} base P/T set to {}/{} (was {}/{})",
                        card_name, new_power, new_toughness, old_power, old_toughness
                    ));
                }
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
            Effect::Balance {
                card_type: _,
                zone: _,
                sub_ability: _,
            } => {
                // Balance effect is handled interactively in the game loop
                // This is a no-op here - the game loop will detect Balance effects
                // and call resolve_balance_effect_interactive with controllers
                //
                // For non-interactive contexts (e.g., unit tests), call
                // execute_balance_effect() directly on GameState.
            }

            Effect::CreateToken {
                controller,
                token_script,
                amount,
            } => {
                // Create token(s) on the battlefield
                // MTG Rules 111.2: The player who creates a token is its owner and controller

                // Look up token definition from cache (loaded during game initialization)
                // For native builds, tokens are loaded from tokenscripts/ directory.
                // For WASM builds, tokens are bundled with deck data.
                let token_def = self.token_definitions.get(token_script).cloned();

                if let Some(token_def) = token_def {
                    // Use actual token definition
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

                        // Debug log token creation
                        log::debug!(target: "token", "Created token {} (id={}) under player {}'s control",
                            token_name, token_id.as_u32(), controller.as_u32());

                        // Log token creation (official game action)
                        self.logger.gamelog(&format!(
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

            Effect::Airbend { target } => {
                // Airbend effect: Exile target, grant owner permission to cast for {2}
                // CR 701.65b: Avatar set mechanic
                //
                // Implementation:
                // 1. Skip if target is still placeholder (0)
                // 2. Get the card's owner before exile
                // 3. Exile the target card
                // 4. Create a PersistentEffect (MayPlayFromExile) for the owner
                // 5. The effect is cleaned up when the card leaves exile or is cast

                // Skip if target is still placeholder (0) - no valid targets found
                if target.as_u32() == 0 {
                    // Ability fizzles - no valid targets
                    return Ok(());
                }

                // Get card info before exile
                let (owner, card_name) = {
                    let card = self.cards.get(*target)?;
                    (card.owner, card.name.clone())
                };

                // Move card from battlefield to exile
                self.move_card(*target, Zone::Battlefield, Zone::Exile, owner)?;

                // Create a PersistentEffect granting MayPlay from exile for {2}
                use crate::core::{CleanupCondition, ManaCost, PersistentEffectKind};

                let cleanup = CleanupCondition::Any(vec![
                    CleanupCondition::TrackedCardLeavesZone {
                        card: *target,
                        zone: Zone::Exile,
                    },
                    CleanupCondition::TrackedCardIsCast { card: *target },
                ]);

                self.persistent_effects.add(
                    PersistentEffectKind::MayPlayFromExile {
                        tracked_card: *target,
                        alternative_cost: ManaCost::from_string("2"), // {2} alternative cost
                        owner,
                    },
                    *target, // source_card - the airbended card itself is the source
                    owner,   // controller - the owner controls this permission
                    cleanup,
                );

                // Log the airbend
                self.logger.gamelog(&format!(
                    "{} is airbended (exiled, owner may cast for {{2}})",
                    card_name
                ));
            }

            Effect::Earthbend { target, num_counters } => {
                // Earthbend effect: Target land becomes 0/0 creature with haste, gets N +1/+1 counters
                // When it dies or is exiled, return it to battlefield tapped
                // CR 701.XX: Avatar set mechanic (custom)
                //
                // Implementation:
                // 1. Skip if target is still placeholder (0)
                // 2. Add Creature type to the land (it stays a land too)
                // 3. Set base power/toughness to 0/0 (temp, for animate effects)
                // 4. Add Haste keyword
                // 5. Put N +1/+1 counters
                // 6. Register delayed trigger for return-to-battlefield on death/exile

                // Skip if target is still placeholder (0) - no valid targets found
                if target.as_u32() == 0 {
                    // Ability fizzles - no valid targets
                    return Ok(());
                }

                // Get card info and apply modifications
                let card_name = {
                    let card = self.cards.get_mut(*target)?;

                    // Must be a land to earthbend
                    if !card.is_land() {
                        return Err(crate::MtgError::InvalidAction(
                            "Earthbend target must be a land".to_string(),
                        ));
                    }

                    // Add Creature type (still remains a land)
                    if !card.is_creature() {
                        card.add_type(CardType::Creature);
                    }

                    // Set temp base power/toughness to 0/0 (animate effect)
                    card.set_temp_base_power(0);
                    card.set_temp_base_toughness(0);

                    // Add Haste keyword so it can attack immediately
                    use crate::core::Keyword;
                    card.keywords.insert(Keyword::Haste);

                    card.name.clone()
                };

                // Add +1/+1 counters
                use crate::core::CounterType;
                self.add_counters(*target, CounterType::P1P1, *num_counters)?;

                // Get controller for the delayed trigger
                let controller = self.turn.active_player;

                // Register delayed trigger: when this land dies or is exiled, return it to battlefield tapped
                use crate::core::{DelayedEffect, DelayedTrigger, DelayedTriggerCondition};
                use smallvec::smallvec;

                let trigger = DelayedTrigger::new(
                    crate::core::DelayedTriggerId::new(0), // ID will be assigned by store
                    *target,                               // tracked_card
                    *target,                               // source_card (the land itself)
                    controller,
                    DelayedTriggerCondition::ZoneChange {
                        from_zones: smallvec![Zone::Battlefield],
                        to_zones: smallvec![Zone::Graveyard, Zone::Exile],
                    },
                    DelayedEffect::ReturnToBattlefield {
                        tapped: true,
                        to_owner: true,
                    },
                );

                let trigger_id = self.delayed_triggers.add(trigger);

                // Log the earthbend
                self.logger.gamelog(&format!(
                    "{} is earthbent! (0/0 creature with haste, {} +1/+1 counters, returns when dies/exiled)",
                    card_name, num_counters
                ));

                // Log trigger creation for debugging
                self.logger.gamelog(&format!(
                    "  -> Delayed trigger {} registered: return {} to battlefield tapped when it leaves",
                    trigger_id.as_u32(),
                    card_name
                ));
            }

            Effect::Firebend { controller, amount } => {
                // Firebend effect: Add N red mana to controller's combat mana pool
                // This mana lasts until end of combat (cleared in end_combat_step)
                // CR 701.XX: Avatar set mechanic (custom)

                // Get player name before mutable borrow for logging
                let player_name = self
                    .get_player(*controller)
                    .map(|p| p.name.clone())
                    .unwrap_or_else(|_| "Unknown".into());

                // Add red mana to combat mana pool (lazy initialization)
                let player = self.get_player_mut(*controller)?;
                for _ in 0..*amount {
                    player.add_combat_mana(crate::core::Color::Red);
                }

                // Log the firebend
                self.logger.gamelog(&format!(
                    "{} adds {} {{R}} (combat mana, lasts until end of combat)",
                    player_name, amount
                ));
            }

            Effect::GrantCantBeBlocked { target } => {
                // GrantCantBeBlocked effect: Target creature can't be blocked this turn
                // Created by AB$ Effect abilities with StaticAbilities$ containing "unblock"
                //
                // Implementation:
                // 1. Skip if target is still placeholder (0)
                // 2. Create a PersistentEffect (CantBeBlocked) for the target
                // 3. The effect is cleaned up at end of turn

                // Skip if target is still placeholder (0) - no valid targets found
                if target.as_u32() == 0 {
                    // Ability fizzles - no valid targets
                    return Ok(());
                }

                // Get card name for logging
                let card_name = self.cards.get(*target).map(|c| c.name.as_str()).unwrap_or("Unknown");

                // Get the effect controller (the player who activated the ability)
                let controller = self.turn.active_player;

                // Create a PersistentEffect granting "can't be blocked"
                use crate::core::{CleanupCondition, PersistentEffectKind};

                self.persistent_effects.add(
                    PersistentEffectKind::CantBeBlocked { creature: *target },
                    *target,    // source_card - the targeted creature
                    controller, // controller - the active player
                    CleanupCondition::EndOfTurn,
                );

                // Log the effect
                self.logger
                    .gamelog(&format!("{} can't be blocked this turn", card_name));
            }

            Effect::CreateDelayedTrigger {
                tracked_card,
                condition,
                effect: delayed_effect,
                expiry,
            } => {
                // CreateDelayedTrigger effect: Register a delayed trigger that fires on a condition
                // Created by SP$ DelayedTrigger spells (e.g., Fatal Fissure)
                //
                // Implementation:
                // 1. Verify the tracked card is still on battlefield (target still valid)
                // 2. Create a DelayedTrigger with the specified condition
                // 3. Store the effect to execute when triggered
                // 4. Register the trigger in the delayed_triggers store

                // Skip if tracked_card is still placeholder (0) - no valid targets found
                if tracked_card.as_u32() == 0 {
                    // Spell fizzles - no valid targets
                    log::debug!(target: "actions", "CreateDelayedTrigger: tracked_card is placeholder, spell fizzles");
                    return Ok(());
                }

                // Verify the target is still on battlefield
                if !self.battlefield.contains(*tracked_card) {
                    log::debug!(target: "actions", "CreateDelayedTrigger: target no longer on battlefield, spell fizzles");
                    return Ok(());
                }

                // Get card name for logging
                let card_name = self
                    .cards
                    .get(*tracked_card)
                    .map(|c| c.name.as_str())
                    .unwrap_or("Unknown");

                // Get the spell controller
                let controller = self.turn.active_player;

                // Create the delayed trigger
                use crate::core::{DelayedEffect, DelayedTrigger, DelayedTriggerId};

                let trigger = DelayedTrigger::new(
                    DelayedTriggerId::new(0), // ID will be assigned by store
                    *tracked_card,            // tracked_card - the creature that needs to die
                    *tracked_card,            // source_card - same as tracked for spell-created triggers
                    controller,
                    condition.clone(),
                    DelayedEffect::ExecuteEffect {
                        effect: delayed_effect.clone(),
                    },
                );

                // Apply expiry if specified
                let trigger = match expiry {
                    Some(exp) => trigger.with_expiry(exp.clone()),
                    None => trigger,
                };

                let trigger_id = self.delayed_triggers.add(trigger);

                // Log the delayed trigger creation
                self.logger.gamelog(&format!(
                    "Delayed trigger {} created: watching {} for death",
                    trigger_id.as_u32(),
                    card_name
                ));

                log::debug!(
                    target: "actions",
                    "CreateDelayedTrigger: trigger {} for {} with effect {:?}",
                    trigger_id.as_u32(), card_name, delayed_effect
                );
            }

            Effect::ModalChoice { modes, .. } => {
                // Modal spells are handled during casting, not execution.
                // When the spell resolves, only the selected mode's effect is executed.
                // This variant should not be encountered during execute_effect.
                //
                // If we get here, it means the modal choice wasn't processed during casting.
                // Log a warning and skip execution.
                log::warn!(
                    target: "actions",
                    "ModalChoice effect reached execute_effect - should have been resolved during casting. {} modes available.",
                    modes.len()
                );
            }

            Effect::CopyPermanent {
                target,
                controller,
                non_legendary: _, // TODO(mtg-8pen1): Implement legendary rule removal when legendary is tracked
                set_power,
                set_toughness,
                ref add_types,
                num_copies,
                restriction: _, // Used at targeting time, not execution time
            } => {
                // Create token copies of the target permanent
                // MTG Rules 707.2: A copy of a permanent has the same characteristics
                // as the original, except for any modifications specified

                // Verify target is still on battlefield
                if !self.battlefield.contains(*target) {
                    // Target was removed - spell fizzles
                    log::debug!(target: "actions", "CopyPermanent target no longer on battlefield");
                    return Ok(());
                }

                let original = self.cards.get(*target)?;
                let original_name = original.name.clone();
                let original_base_power = original.base_power();
                let original_base_toughness = original.base_toughness();

                for _ in 0..*num_copies {
                    let token_id = self.next_card_id();

                    // Clone the original card to get all characteristics
                    let original = self.cards.get(*target)?;
                    let mut token = original.clone();

                    // Update identity for the new token
                    token.id = token_id;
                    token.owner = *controller;
                    token.controller = *controller;

                    // Reset state for new permanent
                    token.tapped = false;
                    token.turn_entered_battlefield = None; // Will be set when it enters battlefield
                    token.counters.clear();
                    token.damage = 0;
                    token.attached_to = None;

                    // Apply modifications

                    // SetPower$ N - override power
                    if let Some(power) = set_power {
                        // Power is i8 in Card but i32 in Effect, clamp to i8 range
                        token.set_base_power(Some(*power as i8));
                    }

                    // SetToughness$ N - override toughness
                    if let Some(toughness) = set_toughness {
                        token.set_base_toughness(Some(*toughness as i8));
                    }

                    // AddTypes$ Type1 & Type2 - add creature types (subtypes)
                    for type_str in add_types {
                        let subtype = crate::core::Subtype::from(type_str.as_str());
                        if !token.subtypes.contains(&subtype) {
                            token.subtypes.push(subtype);
                        }
                    }

                    // Add token to game
                    let token_name = token.name.to_string();
                    self.cards.insert(token_id, token);

                    // Put token onto battlefield
                    self.battlefield.add(token_id);

                    // Log token creation
                    let modification_desc = if set_power.is_some() || set_toughness.is_some() || !add_types.is_empty() {
                        let p = set_power.map(|x| x as i8).or(original_base_power).unwrap_or(0);
                        let t = set_toughness.map(|x| x as i8).or(original_base_toughness).unwrap_or(0);
                        let types_str = if add_types.is_empty() {
                            String::new()
                        } else {
                            format!(" {}", add_types.join(" "))
                        };
                        format!(" (as {}/{}{} copy)", p, t, types_str)
                    } else {
                        String::new()
                    };

                    log::debug!(
                        target: "token",
                        "Created token copy of {} (id={}) under player {}'s control{}",
                        original_name, token_id.as_u32(), controller.as_u32(), modification_desc
                    );

                    self.logger.gamelog(&format!(
                        "Created a token copy of {}{} under {}'s control",
                        token_name,
                        modification_desc,
                        self.get_player(*controller)?.name
                    ));
                }
            }

            Effect::Dig {
                dig_count,
                change_count: _,
                change_all: _, // TODO: implement partial dig (change fewer than looked at)
                destination,
                may_play,
                may_play_without_mana_cost,
            } => {
                // Dig effect: Exile top N cards from each opponent's library
                // For Fire Lord Ozai: "Exile the top card of each opponent's library.
                //                      Until end of turn, you may play one of those cards
                //                      without paying its mana cost."
                //
                // Implementation:
                // 1. Get current player (the digger)
                // 2. For each opponent, exile top N cards from their library
                // 3. If may_play, create persistent effects for playing those cards

                let digger = self.turn.active_player;

                // Collect opponent IDs first (SmallVec for stack allocation - typically 1-3 opponents)
                // This releases the borrow on self.players before we call self.move_card()
                let opponent_ids: smallvec::SmallVec<[PlayerId; 4]> =
                    self.players.iter().filter(|p| p.id != digger).map(|p| p.id).collect();

                // Pre-size exiled_cards buffer (at most dig_count cards per opponent)
                let mut exiled_cards: Vec<CardId> = Vec::with_capacity(opponent_ids.len() * (*dig_count as usize));

                // For each opponent, exile top card(s) from their library
                for opponent_id in opponent_ids {
                    // Get opponent's library
                    let library = self
                        .player_zones
                        .iter()
                        .find(|(id, _)| *id == opponent_id)
                        .map(|(_, zones)| &zones.library);

                    if let Some(library) = library {
                        // Collect card IDs to exile first (SmallVec for stack allocation)
                        let take_count = *dig_count as usize;
                        let card_ids: smallvec::SmallVec<[CardId; 4]> =
                            library.cards.iter().take(take_count).copied().collect();

                        // Now exile each card
                        for card_id in card_ids {
                            // Get names as references for logging (avoid clone)
                            let card_name = self.cards.get(card_id)?.name.as_str();
                            let opponent_name = self.get_player(opponent_id)?.name.as_str();

                            // Log before move (need the names)
                            self.logger
                                .gamelog(&format!("{} exiled from {}'s library", card_name, opponent_name));

                            // Move card from library to destination (usually exile)
                            self.move_card(card_id, Zone::Library, *destination, opponent_id)?;

                            exiled_cards.push(card_id);
                        }
                    }
                }

                // If may_play is true, create persistent effect to allow playing exiled cards
                if *may_play && !exiled_cards.is_empty() {
                    let mana_cost_text = if *may_play_without_mana_cost {
                        " without paying its mana cost"
                    } else {
                        ""
                    };

                    self.logger.gamelog(&format!(
                        "Until end of turn, you may play one of those cards{}",
                        mana_cost_text
                    ));

                    // Create persistent effect for "may play one without paying mana cost"
                    // This effect tracks all exiled cards and allows the digger to play ONE
                    use crate::core::{CleanupCondition, PersistentEffectKind};

                    // For Fire Lord Ozai, we always grant may-play-without-cost
                    // (other Dig effects might have different behavior)
                    if *may_play_without_mana_cost {
                        // Get source card ID (if available) for the persistent effect
                        // Since we're in an activated ability, the source should be on the battlefield
                        // For now, use the first exiled card as the "source" for tracking
                        let source_card = exiled_cards[0];
                        let num_exiled = exiled_cards.len();

                        // Move exiled_cards into the persistent effect (avoid clone)
                        self.persistent_effects.add(
                            PersistentEffectKind::MayPlayOneWithoutManaCost {
                                tracked_cards: std::mem::take(&mut exiled_cards),
                                beneficiary: digger,
                            },
                            source_card,
                            digger,
                            CleanupCondition::EndOfTurn,
                        );

                        log::debug!(
                            target: "dig",
                            "Created MayPlayOneWithoutManaCost effect for {} cards, beneficiary: player {}",
                            num_exiled,
                            digger.as_u32()
                        );
                    }
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
    pub fn card_matches_search_filter(card: &crate::core::Card, filter: &str) -> bool {
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

    /// Choose the best permanent to sacrifice for an optional trigger cost.
    /// Returns None if no valid target exists.
    ///
    /// AI heuristic: pick the "lowest value" permanent matching the pattern.
    /// For creatures, this is based on P/T sum. For non-creatures, we prefer tokens.
    pub fn choose_sacrifice_target(
        &self,
        pattern: &str,
        source_card_id: CardId,
        player_id: PlayerId,
    ) -> Option<CardId> {
        // Parse the pattern - multiple options separated by semicolons
        let patterns: Vec<&str> = pattern.split(';').collect();

        // Collect all valid sacrifice targets with their "value" for AI comparison
        let mut candidates: Vec<(CardId, i32)> = self
            .battlefield
            .cards
            .iter()
            .filter_map(|&card_id| {
                let card = self.cards.get(card_id).ok()?;

                // Must be controlled by the player
                if card.controller != player_id {
                    return None;
                }

                // Check each pattern option (OR logic)
                for p in &patterns {
                    let mut matches = false;

                    // Check card type
                    if p.contains("Artifact") && card.is_artifact() {
                        matches = true;
                    }
                    if p.contains("Creature") && card.is_creature() {
                        matches = true;
                    }
                    if p.contains("Land") && card.is_land() {
                        matches = true;
                    }

                    // Check "Other" modifier - can't sacrifice the source
                    if p.contains(".Other") && card_id == source_card_id {
                        matches = false;
                    }

                    if matches {
                        // Calculate a "value" for this permanent (lower = better to sacrifice)
                        // Creatures: P/T sum (prefer low P/T creatures)
                        // Non-creatures: CMC (prefer low CMC)
                        let value = if card.is_creature() {
                            i32::from(card.current_power()) + i32::from(card.current_toughness())
                        } else {
                            i32::from(card.mana_cost.cmc())
                        };

                        return Some((card_id, value));
                    }
                }
                None
            })
            .collect();

        // Sort by value (ascending - lowest value first)
        candidates.sort_by_key(|(_, value)| *value);

        // Return the lowest-value target
        candidates.first().map(|(id, _)| *id)
    }

    /// Check if a sacrifice pattern cost can be paid by the given player.
    /// Returns true if the player has enough valid permanents to sacrifice.
    ///
    /// Sacrifice patterns are strings like:
    /// - "Artifact.Other" - an artifact other than the source
    /// - "Creature.Other" - a creature other than the source
    /// - "Artifact.Other;Creature.Other" - an artifact or creature other than the source
    pub fn can_pay_sacrifice_pattern(
        &self,
        pattern: &str,
        count: u8,
        source_card_id: CardId,
        player_id: PlayerId,
    ) -> bool {
        // Parse the pattern - it can be multiple options separated by semicolons
        // e.g., "Artifact.Other;Creature.Other" means artifact OR creature
        let patterns: Vec<&str> = pattern.split(';').collect();

        // Count valid sacrifice targets
        let valid_targets: usize = self
            .battlefield
            .cards
            .iter()
            .filter(|&&card_id| {
                if let Ok(card) = self.cards.get(card_id) {
                    // Must be controlled by the player
                    if card.controller != player_id {
                        return false;
                    }

                    // Check each pattern option (OR logic)
                    for p in &patterns {
                        let mut matches = false;

                        // Check card type
                        if p.contains("Artifact") && card.is_artifact() {
                            matches = true;
                        }
                        if p.contains("Creature") && card.is_creature() {
                            matches = true;
                        }
                        if p.contains("Land") && card.is_land() {
                            matches = true;
                        }

                        // Check "Other" modifier - can't sacrifice self
                        if p.contains(".Other") && card_id == source_card_id {
                            matches = false;
                        }

                        if matches {
                            return true;
                        }
                    }
                }
                false
            })
            .count();

        valid_targets >= count as usize
    }

    /// Choose a card to discard from the player's hand.
    ///
    /// AI heuristic: pick the "lowest value" card.
    /// - Lands are preferred to discard (since hand is usually full of spells)
    /// - For spells, prefer higher CMC (less likely to cast soon)
    ///
    /// Returns None if the player has no cards in hand.
    ///
    /// # Errors
    ///
    /// Returns an error if the player's zones cannot be found.
    pub fn choose_card_to_discard(&self, player_id: PlayerId) -> Result<Option<CardId>> {
        let zones = self
            .get_player_zones(player_id)
            .ok_or_else(|| MtgError::InvalidAction("Player zones not found".to_string()))?;

        if zones.hand.is_empty() {
            return Ok(None);
        }

        // Collect cards with their "discard value" (higher = more desirable to discard)
        let mut candidates: Vec<(CardId, i32)> = zones
            .hand
            .cards
            .iter()
            .filter_map(|&card_id| {
                let card = self.cards.get(card_id).ok()?;

                // Calculate discard value (higher = better to discard)
                // Lands are most desirable to discard when looting (value 1000)
                // Spells: prefer discarding high CMC spells (value = CMC)
                // since we're likely to draw into something better
                let value = if card.is_land() {
                    1000
                } else {
                    i32::from(card.mana_cost.cmc())
                };

                Some((card_id, value))
            })
            .collect();

        // Sort by value (descending - highest value first = best to discard)
        candidates.sort_by_key(|(_, value)| -(*value));

        Ok(candidates.first().map(|(id, _)| *id))
    }

    /// Discard a specific card from the player's hand.
    ///
    /// Moves the card from hand to graveyard and logs the action.
    ///
    /// # Errors
    ///
    /// Returns an error if the card or player cannot be found, or if the
    /// card cannot be moved from hand to graveyard.
    pub fn discard_card(&mut self, player_id: PlayerId, card_id: CardId) -> Result<()> {
        // Get card name for logging before move
        let card_name = self.cards.get(card_id)?.name.to_string();
        let player_name = self.get_player(player_id)?.name.clone();

        // Move card from hand to graveyard
        self.move_card(card_id, Zone::Hand, Zone::Graveyard, player_id)?;

        // Log the discard
        self.logger.gamelog(&format!("{} discards {}", player_name, card_name));

        Ok(())
    }

    ///
    /// This checks all permanents on the battlefield for triggers matching the given event.
    /// When triggers are found, their effects are executed immediately (for now).
    ///
    /// Optional triggers with costs (e.g., "you may sacrifice...") are skipped if:
    /// - The cost cannot be paid (auto-decline)
    ///
    /// If the cost CAN be paid, the trigger fires (AI auto-accepts for now).
    /// TODO: Add player choice for optional triggers when the cost is payable.
    ///
    /// TODO: In full MTG rules, triggers should go on the stack and wait for priority,
    /// but for simplicity we're executing them immediately.
    ///
    /// Note: Wildcard match is intentional - only specific effects need placeholder
    /// target resolution; others execute as-is.
    ///
    /// # Errors
    ///
    /// Returns an error if effect execution fails.
    #[allow(clippy::wildcard_enum_match_arm)]
    pub fn check_triggers(&mut self, event: TriggerEvent, source_card_id: CardId) -> Result<()> {
        use crate::core::Trigger;

        // Info needed to check trigger payability and execute costs
        struct TriggerInfo {
            card_id: CardId,
            card_name: String,
            controller: PlayerId,
            trigger: Trigger,
        }

        // Collected trigger with cost info for execution
        struct TriggerToExecute {
            source_card_id: CardId,
            effects: Vec<Effect>,
            sacrifice_target: Option<CardId>, // Card to sacrifice for the cost
            sacrificed_power: u8,             // Power of sacrifice target (for Firebend effects)
        }

        // Phase 1: Collect matching triggers with their metadata
        let candidate_triggers: Vec<TriggerInfo> = self
            .battlefield
            .cards
            .iter()
            .filter_map(|&card_id| {
                if let Ok(card) = self.cards.get(card_id) {
                    let controller = card.controller;
                    let card_name = card.name.clone();

                    // Find triggers matching this event
                    let triggers: Vec<TriggerInfo> = card
                        .triggers
                        .iter()
                        .filter(|trigger| {
                            // Check event type matches
                            if trigger.event != event {
                                return false;
                            }

                            // Self-only triggers only fire when the trigger source is the event source
                            if trigger.trigger_self_only && card_id != source_card_id {
                                return false;
                            }

                            // "[other]" triggers only fire when the event source is DIFFERENT from trigger source
                            // (e.g., "whenever you sacrifice another permanent" on Pirate Peddlers)
                            if trigger.description.contains("[other]") && card_id == source_card_id {
                                return false;
                            }

                            true
                        })
                        .map(|trigger| TriggerInfo {
                            card_id,
                            card_name: card_name.to_string(),
                            controller,
                            trigger: trigger.clone(),
                        })
                        .collect();

                    if triggers.is_empty() {
                        None
                    } else {
                        Some(triggers)
                    }
                } else {
                    None
                }
            })
            .flatten()
            .collect();

        // Phase 2: Filter by cost payability, choose sacrifice targets, and collect effects
        let triggered_effects: Vec<TriggerToExecute> = candidate_triggers
            .into_iter()
            .filter_map(|info| {
                let mut sacrifice_target: Option<CardId> = None;
                let mut sacrificed_power: u8 = 0;

                // For optional triggers with costs, check payability and choose targets
                if info.trigger.optional {
                    if let Some(ref cost) = info.trigger.cost {
                        // Check if sacrifice cost can be paid
                        if let Some((count, pattern)) = cost.get_sacrifice_pattern() {
                            if !self.can_pay_sacrifice_pattern(pattern, count, info.card_id, info.controller) {
                                log::debug!(
                                    "Skipping optional trigger on {} - sacrifice cost not payable (need {} {})",
                                    info.card_name,
                                    count,
                                    pattern
                                );
                                return None; // Auto-decline if can't pay
                            }

                            // Choose which permanent to sacrifice (AI heuristic: pick lowest P/T creature or artifact)
                            sacrifice_target = self.choose_sacrifice_target(pattern, info.card_id, info.controller);

                            // Capture power of sacrifice target for Firebend effects (Fire Lord Ozai)
                            if let Some(sac_id) = sacrifice_target {
                                if let Ok(sac_card) = self.cards.get(sac_id) {
                                    sacrificed_power = sac_card.current_power().max(0) as u8;
                                }
                            }
                        }
                        // TODO: Check other cost types (mana, life, etc.)
                    }
                }

                // Trigger passes all checks - collect effects
                if !info.trigger.effects.is_empty() {
                    log::debug!(
                        "Found {} triggers on card {} ({})",
                        info.trigger.effects.len(),
                        info.card_id.as_u32(),
                        info.card_name
                    );
                    for effect in &info.trigger.effects {
                        log::debug!("  Trigger effect: {:?}", effect);
                    }
                    if let Some(sac_id) = sacrifice_target {
                        if let Ok(sac_card) = self.cards.get(sac_id) {
                            log::debug!(
                                "  Will sacrifice: {} ({}) power={}",
                                sac_card.name,
                                sac_id.as_u32(),
                                sacrificed_power
                            );
                        }
                    }
                    Some(TriggerToExecute {
                        source_card_id: info.card_id,
                        effects: info.trigger.effects,
                        sacrifice_target,
                        sacrificed_power,
                    })
                } else {
                    None
                }
            })
            .collect();

        // Phase 3: Execute sacrifices and triggered effects
        for trigger_to_exec in triggered_effects {
            let trigger_source = trigger_to_exec.source_card_id;
            let sacrificed_power = trigger_to_exec.sacrificed_power;

            // Execute sacrifice cost first (if any)
            if let Some(sac_target) = trigger_to_exec.sacrifice_target {
                if let Ok(sac_card) = self.cards.get(sac_target) {
                    let sac_name = sac_card.name.to_string();
                    let sac_owner = sac_card.owner;
                    log::info!("Sacrificing {} ({}) for trigger cost", sac_name, sac_target.as_u32());

                    // Move from battlefield to graveyard
                    self.move_card(sac_target, Zone::Battlefield, Zone::Graveyard, sac_owner)?;

                    // Check sacrifice triggers (e.g., Pirate Peddlers Mode$ Sacrificed)
                    self.check_triggers(TriggerEvent::Sacrificed, sac_target)?;
                }
            }

            // Now execute all trigger effects
            for mut effect in trigger_to_exec.effects {
                // Fill in placeholder values in trigger effects
                // Similar to resolve_spell, we need to fill in targets
                match &mut effect {
                    Effect::DrawCards { player, count } if player.as_u32() == 0 => {
                        // Placeholder player ID 0 means the controller of the trigger source
                        let controller = self.cards.get(trigger_source)?.controller;
                        effect = Effect::DrawCards {
                            player: controller,
                            count: *count,
                        };
                    }
                    Effect::Loot {
                        player,
                        discard_count,
                        draw_count,
                    } if player.as_u32() == 0 => {
                        // Placeholder player ID 0 means the controller of the trigger source
                        let controller = self.cards.get(trigger_source)?.controller;
                        effect = Effect::Loot {
                            player: controller,
                            discard_count: *discard_count,
                            draw_count: *draw_count,
                        };
                    }
                    Effect::DealDamage {
                        target: TargetRef::Player(player_id),
                        amount,
                    } if player_id.as_u32() == 0 => {
                        // Placeholder player ID 0 means the controller of the trigger source
                        // Used by cards like Juzám Djinn ("deals 1 damage to you")
                        let controller = self.cards.get(trigger_source)?.controller;
                        effect = Effect::DealDamage {
                            target: TargetRef::Player(controller),
                            amount: *amount,
                        };
                    }
                    Effect::DealDamage {
                        target: TargetRef::None,
                        amount,
                    } => {
                        // Find a valid target: prefer opponent's creature, else opponent player
                        // This handles "any target" effects like Mongoose Lizard's ETB
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
                        } else {
                            // No valid creature target found - target opponent player instead
                            // This is correct for "any target" effects (ValidTgts$ Any)
                            // In a 2-player game, the opponent is the other player
                            let opponent = self.players.iter().find(|p| p.id != controller).map(|p| p.id);
                            if let Some(opponent_id) = opponent {
                                effect = Effect::DealDamage {
                                    target: TargetRef::Player(opponent_id),
                                    amount: *amount,
                                };
                            }
                            // If somehow no opponent found (shouldn't happen), effect stays TargetRef::None
                            // and will fizzle when executed
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
                    Effect::PutCounter {
                        target,
                        counter_type,
                        amount,
                    } if target.as_u32() == 0 => {
                        // Placeholder CardId 0 means "put counter on self" (the trigger source)
                        // Used by Beetle-Headed Merchants: "put a +1/+1 counter on this creature"
                        effect = Effect::PutCounter {
                            target: trigger_source,
                            counter_type: *counter_type,
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
                        keywords_granted,
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
                                keywords_granted: keywords_granted.clone(),
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
                    Effect::ExilePermanent { target } if target.as_u32() == 0 => {
                        // Find a valid target (opponent's nonland permanent)
                        // Web Up and similar cards: "exile target nonland permanent an opponent controls"
                        let controller = self.cards.get(trigger_source)?.controller;
                        if let Some(target_id) = self
                            .battlefield
                            .cards
                            .iter()
                            .find(|&card_id| {
                                if let Ok(card) = self.cards.get(*card_id) {
                                    // Target nonland permanents controlled by opponents
                                    !card.is_land()
                                        && card.controller != controller
                                        && !card.has_hexproof()
                                        && !card.has_shroud()
                                } else {
                                    false
                                }
                            })
                            .copied()
                        {
                            effect = Effect::ExilePermanent { target: target_id };
                        }
                    }
                    Effect::Earthbend { target, num_counters } if target.as_u32() == 0 => {
                        // Placeholder CardId 0 means we need to target a land the controller controls
                        // For now, pick the first land they control (AI could choose better targets)
                        let controller = self.cards.get(trigger_source)?.controller;

                        // Find a land controlled by the trigger's controller
                        if let Some(land_id) = self
                            .battlefield
                            .cards
                            .iter()
                            .filter_map(|cid| {
                                let card = self.cards.get(*cid).ok()?;
                                if card.controller == controller && card.is_land() {
                                    Some(*cid)
                                } else {
                                    None
                                }
                            })
                            .next()
                        {
                            effect = Effect::Earthbend {
                                target: land_id,
                                num_counters: *num_counters,
                            };
                        } else {
                            // No valid land target - skip this trigger
                            continue;
                        }
                    }
                    Effect::PumpAllCreatures {
                        controller,
                        filter,
                        power_bonus,
                        toughness_bonus,
                    } if controller.as_u32() == 0 => {
                        // Placeholder controller 0 means the controller of the trigger source
                        let source_controller = self.cards.get(trigger_source)?.controller;
                        effect = Effect::PumpAllCreatures {
                            controller: source_controller,
                            filter: filter.clone(),
                            power_bonus: *power_bonus,
                            toughness_bonus: *toughness_bonus,
                        };
                    }
                    Effect::Firebend {
                        controller: ctrl,
                        amount,
                    } if ctrl.as_u32() == 0 => {
                        // Resolve placeholder controller to the actual creature controller
                        // amount=254 means "use sacrificed creature's power" (Fire Lord Ozai)
                        let source_controller = self.cards.get(trigger_source)?.controller;
                        let actual_amount = if *amount == 254 { sacrificed_power } else { *amount };

                        effect = Effect::Firebend {
                            controller: source_controller,
                            amount: actual_amount,
                        };

                        // Log the firebend trigger
                        if actual_amount > 0 {
                            if let Ok(card) = self.cards.get(trigger_source) {
                                self.logger.gamelog(&format!(
                                    "{} triggers Firebending {} (adding {} {{R}} to combat mana)",
                                    card.name, actual_amount, actual_amount
                                ));
                            }
                        }
                    }
                    _ => {}
                }

                self.execute_effect(&effect)?;
            }
        }

        Ok(())
    }

    /// Check and execute triggered abilities for a specific card only
    ///
    /// This is used by phase triggers where we've already determined which cards
    /// should trigger based on the active player (controller_only filtering).
    /// Accepts the active player for proper trigger filtering.
    ///
    /// Note: Wildcard matches are intentional - only specific effects need placeholder
    /// resolution or formatted logging; others pass through unchanged.
    ///
    /// # Errors
    ///
    /// Returns an error if the card cannot be found or effect execution fails.
    #[allow(clippy::wildcard_enum_match_arm)]
    pub fn check_triggers_for_controller(
        &mut self,
        event: TriggerEvent,
        card_id: CardId,
        active_player: PlayerId,
    ) -> Result<()> {
        // Get the card's triggers
        let effects_to_execute: Vec<Effect> = {
            let card = self.cards.get(card_id)?;

            // Only process triggers where the controller matches the active player
            // OR the trigger doesn't have the [controller_only] flag
            card.triggers
                .iter()
                .filter(|trigger| {
                    if trigger.event != event {
                        return false;
                    }
                    // [controller_only] triggers should only fire on the controller's turn
                    // This was already checked in check_phase_triggers, but verify here too
                    if trigger.description.starts_with("[controller_only]") {
                        return card.controller == active_player;
                    }
                    true
                })
                .flat_map(|trigger| trigger.effects.clone())
                .collect()
        };

        // Execute each effect with placeholder resolution
        for mut effect in effects_to_execute {
            // Fill in placeholder values in trigger effects
            match &mut effect {
                Effect::DealDamage {
                    target: TargetRef::Player(player_id),
                    amount,
                } if player_id.as_u32() == 0 => {
                    // Placeholder player ID 0 means the controller of the trigger source
                    // Used by cards like Juzám Djinn ("deals 1 damage to you")
                    let controller = self.cards.get(card_id)?.controller;
                    effect = Effect::DealDamage {
                        target: TargetRef::Player(controller),
                        amount: *amount,
                    };
                }
                Effect::GainLife { player, amount } if player.as_u32() == 0 => {
                    // Placeholder player ID 0 means the controller of the trigger source
                    let controller = self.cards.get(card_id)?.controller;
                    effect = Effect::GainLife {
                        player: controller,
                        amount: *amount,
                    };
                }
                Effect::DrawCards { player, count } if player.as_u32() == 0 => {
                    // Placeholder player ID 0 means the controller of the trigger source
                    let controller = self.cards.get(card_id)?.controller;
                    effect = Effect::DrawCards {
                        player: controller,
                        count: *count,
                    };
                }
                Effect::Loot {
                    player,
                    discard_count,
                    draw_count,
                } if player.as_u32() == 0 => {
                    // Placeholder player ID 0 means the controller of the trigger source
                    let controller = self.cards.get(card_id)?.controller;
                    effect = Effect::Loot {
                        player: controller,
                        discard_count: *discard_count,
                        draw_count: *draw_count,
                    };
                }
                Effect::Earthbend { target, num_counters } if target.as_u32() == 0 => {
                    // Placeholder CardId 0 means we need to target a land the controller controls
                    // For now, pick the first land they control (AI could choose better targets)
                    let controller = self.cards.get(card_id)?.controller;

                    // Find a land controlled by the trigger's controller
                    let land_target = self
                        .battlefield
                        .cards
                        .iter()
                        .filter_map(|cid| {
                            let card = self.cards.get(*cid).ok()?;
                            if card.controller == controller && card.is_land() {
                                Some(*cid)
                            } else {
                                None
                            }
                        })
                        .next();

                    if let Some(land_id) = land_target {
                        effect = Effect::Earthbend {
                            target: land_id,
                            num_counters: *num_counters,
                        };
                    } else {
                        // No valid land target - skip this trigger
                        continue;
                    }
                }
                Effect::PumpAllCreatures {
                    controller,
                    filter,
                    power_bonus,
                    toughness_bonus,
                } if controller.as_u32() == 0 => {
                    // Placeholder controller 0 means the controller of the trigger source
                    let source_controller = self.cards.get(card_id)?.controller;
                    effect = Effect::PumpAllCreatures {
                        controller: source_controller,
                        filter: filter.clone(),
                        power_bonus: *power_bonus,
                        toughness_bonus: *toughness_bonus,
                    };
                }
                _ => {}
            }

            // Log the trigger effect
            if let Ok(card) = self.cards.get(card_id) {
                let card_name = card.name.clone();
                let message = match &effect {
                    Effect::DealDamage {
                        target: TargetRef::Player(player_id),
                        amount,
                    } => {
                        let player_name = self
                            .get_player(*player_id)
                            .map(|p| p.name.as_str().to_string())
                            .unwrap_or_else(|_| "player".to_string());
                        format!("{} deals {} damage to {}", card_name, amount, player_name)
                    }
                    Effect::GainLife { player, amount } => {
                        let player_name = self
                            .get_player(*player)
                            .map(|p| p.name.as_str().to_string())
                            .unwrap_or_else(|_| "player".to_string());
                        format!("{} causes {} to gain {} life", card_name, player_name, amount)
                    }
                    _ => format!("{} trigger effect", card_name),
                };
                self.logger.normal(&message);
            }

            self.execute_effect(&effect)?;
        }

        Ok(())
    }

    /// Check and execute SpellCast triggers when a spell is cast
    ///
    /// This handles "Whenever you cast a [noncreature] spell" triggers like:
    /// - Boar-q-pine: Whenever you cast a noncreature spell, put a +1/+1 counter on this creature
    /// - Prowess: Whenever you cast a noncreature spell, this creature gets +1/+1 until end of turn
    ///
    /// MTG Rules 601.2i: The spell becomes cast after costs are paid, triggering these abilities.
    ///
    /// # Parameters
    /// - `cast_spell_id`: The spell that was just cast (used to check if it's noncreature)
    /// - `caster_id`: The player who cast the spell (triggers only fire for spells cast by the controller)
    ///
    /// # Errors
    ///
    /// Returns an error if trigger effects fail to resolve.
    pub fn check_spellcast_triggers(&mut self, cast_spell_id: CardId, caster_id: PlayerId) -> Result<()> {
        use crate::core::Trigger;

        // Check if the cast spell is a creature (for noncreature-only triggers)
        let is_creature_spell = self.cards.get(cast_spell_id).map(|c| c.is_creature()).unwrap_or(false);

        // Collect SpellCast triggers from permanents on the battlefield
        // These triggers fire when their controller casts a spell
        struct TriggerToExecute {
            source_card_id: CardId,
            source_name: String,
            effects: Vec<Effect>,
            description: String,
        }

        let triggers_to_execute: Vec<TriggerToExecute> = self
            .battlefield
            .cards
            .iter()
            .filter_map(|&card_id| {
                if let Ok(card) = self.cards.get(card_id) {
                    // Only trigger for permanents controlled by the caster
                    if card.controller != caster_id {
                        return None;
                    }

                    // Find SpellCast triggers on this permanent
                    let matching_triggers: Vec<&Trigger> = card
                        .triggers
                        .iter()
                        .filter(|trigger| {
                            if trigger.event != TriggerEvent::SpellCast {
                                return false;
                            }

                            // Check noncreature-only triggers (marked with [noncreature] in description)
                            if trigger.description.contains("[noncreature]")
                                || trigger.description.contains("noncreature")
                            {
                                // This trigger only fires on noncreature spells
                                if is_creature_spell {
                                    return false;
                                }
                            }

                            true
                        })
                        .collect();

                    if matching_triggers.is_empty() {
                        None
                    } else {
                        Some(
                            matching_triggers
                                .into_iter()
                                .map(|trigger| TriggerToExecute {
                                    source_card_id: card_id,
                                    source_name: card.name.to_string(),
                                    effects: trigger.effects.clone(),
                                    description: trigger.description.clone(),
                                })
                                .collect::<Vec<_>>(),
                        )
                    }
                } else {
                    None
                }
            })
            .flatten()
            .collect();

        // Execute each trigger's effects
        for trigger in triggers_to_execute {
            // Log the trigger
            self.logger
                .gamelog(&format!("Trigger: {} - {}", trigger.source_name, trigger.description));

            // Execute effects with placeholder resolution
            for effect in trigger.effects {
                // Resolve placeholder targets (only specific effects need resolution)
                #[allow(clippy::wildcard_enum_match_arm)]
                let resolved_effect = match effect {
                    Effect::PutCounter {
                        target,
                        counter_type,
                        amount,
                    } if target.as_u32() == 0 => {
                        // Placeholder CardId 0 means "put counter on self" (the trigger source)
                        // Log the counter addition
                        let current_counters = self
                            .cards
                            .get(trigger.source_card_id)
                            .map(|c| c.get_counter(crate::core::CounterType::P1P1))
                            .unwrap_or(0);
                        self.logger.normal(&format!(
                            "{} gets a +1/+1 counter (now {} counters)",
                            trigger.source_name,
                            current_counters + amount
                        ));

                        Effect::PutCounter {
                            target: trigger.source_card_id,
                            counter_type,
                            amount,
                        }
                    }
                    Effect::PumpCreature {
                        target,
                        power_bonus,
                        toughness_bonus,
                        keywords_granted,
                    } if target.as_u32() == 0 => {
                        // Placeholder CardId 0 means "pump self" (for Prowess-like effects)
                        self.logger.normal(&format!(
                            "{} gets +{}/+{} until end of turn",
                            trigger.source_name, power_bonus, toughness_bonus
                        ));

                        Effect::PumpCreature {
                            target: trigger.source_card_id,
                            power_bonus,
                            toughness_bonus,
                            keywords_granted,
                        }
                    }
                    other => other,
                };

                self.execute_effect(&resolved_effect)?;
            }
        }

        Ok(())
    }

    /// Check and execute death triggers for a creature that is dying
    ///
    /// Called BEFORE the creature is moved to the graveyard, so its triggers
    /// are still accessible. This handles "When CARDNAME dies" triggers like Su-Chi.
    ///
    /// MTG Rules 603.6c: Triggered abilities look back in time to determine if
    /// the event occurred. Death triggers trigger when a creature moves from
    /// battlefield to graveyard.
    ///
    /// Note: Wildcard match is intentional - only AddMana effects need player
    /// resolution; others execute as-is.
    ///
    /// # Errors
    ///
    /// Returns an error if the card cannot be found or effect execution fails.
    #[allow(clippy::wildcard_enum_match_arm)]
    pub fn check_death_triggers(&mut self, dying_card_id: CardId) -> Result<()> {
        // Get the card's triggers and controller while it's still on battlefield
        let (effects_to_execute, controller): (Vec<Effect>, PlayerId) = {
            let card = self.cards.get(dying_card_id)?;

            // Collect LeavesBattlefield triggers (which we use for "dies" events)
            let effects: Vec<Effect> = card
                .triggers
                .iter()
                .filter(|trigger| trigger.event == TriggerEvent::LeavesBattlefield)
                .flat_map(|trigger| trigger.effects.clone())
                .collect();

            (effects, card.controller)
        };

        if effects_to_execute.is_empty() {
            return Ok(());
        }

        // Log the trigger (official game action)
        if let Ok(card) = self.cards.get(dying_card_id) {
            for trigger in &card.triggers {
                if trigger.event == TriggerEvent::LeavesBattlefield {
                    self.logger
                        .gamelog(&format!("Trigger: {} - {}", card.name, trigger.description));
                }
            }
        }

        // Execute each effect with placeholder resolution
        for mut effect in effects_to_execute {
            // Fill in placeholder values in trigger effects
            match &mut effect {
                Effect::AddMana {
                    player,
                    mana,
                    produces_chosen_color,
                } if player.as_u32() == 0 => {
                    // Placeholder player ID 0 means the controller of the trigger source
                    // Su-Chi adds mana to its controller's pool when it dies
                    effect = Effect::AddMana {
                        player: controller,
                        mana: *mana,
                        produces_chosen_color: *produces_chosen_color,
                    };

                    // Log the mana addition (official game action)
                    if let Ok(card) = self.cards.get(dying_card_id) {
                        let player_name = self
                            .get_player(controller)
                            .map(|p| p.name.as_str().to_string())
                            .unwrap_or_else(|_| "player".to_string());
                        self.logger.gamelog(&format!(
                            "{} dies, {} adds mana to {}'s pool",
                            card.name, card.name, player_name
                        ));
                    }
                }
                Effect::DealDamage {
                    target: TargetRef::Player(player_id),
                    amount,
                } if player_id.as_u32() == 0 => {
                    effect = Effect::DealDamage {
                        target: TargetRef::Player(controller),
                        amount: *amount,
                    };
                }
                Effect::GainLife { player, amount } if player.as_u32() == 0 => {
                    effect = Effect::GainLife {
                        player: controller,
                        amount: *amount,
                    };
                }
                _ => {}
            }

            self.execute_effect(&effect)?;
        }

        Ok(())
    }

    /// Check and execute attack triggers for an attacking creature
    ///
    /// Called after each attacker is declared. Handles "Whenever this creature attacks"
    /// triggers like Firebending, which add combat mana.
    ///
    /// MTG Rules 508.1m: Abilities that trigger on declaring attackers go on the stack.
    ///
    /// Note: Wildcard match is intentional - only AddMana/Firebend effects need player
    /// resolution; others execute as-is.
    ///
    /// # Errors
    ///
    /// Returns an error if the attacker card is not found or effect execution fails.
    #[allow(clippy::wildcard_enum_match_arm)]
    pub fn check_attack_triggers(&mut self, attacker_id: CardId, _active_player: PlayerId) -> Result<()> {
        use smallvec::SmallVec;

        // Fast path: Check if card has any attack triggers BEFORE any allocation
        // Most cards have no triggers at all, so this skips all work in the common case
        let has_attack_triggers = {
            let card = self.cards.get(attacker_id)?;
            card.triggers.iter().any(|t| t.event == TriggerEvent::Attacks)
        };

        if !has_attack_triggers {
            return Ok(());
        }

        // Slow path: Card has attack triggers - extract data and execute
        // Use SmallVec to avoid heap allocation (most triggers have 1-2 effects)
        struct TriggerData {
            card_name: String,
            description: String,
            effects: SmallVec<[Effect; 2]>,
            optional: bool,
            cost: Option<crate::core::Cost>,
        }

        let (controller, creature_power, triggers): (PlayerId, u8, SmallVec<[TriggerData; 1]>) = {
            let card = self.cards.get(attacker_id)?;
            let power = card.current_power().max(0) as u8;

            let mut triggers: SmallVec<[TriggerData; 1]> = SmallVec::new();
            for trigger in &card.triggers {
                if trigger.event == TriggerEvent::Attacks {
                    triggers.push(TriggerData {
                        card_name: card.name.to_string(),
                        description: trigger.description.clone(),
                        effects: SmallVec::from_iter(trigger.effects.iter().cloned()),
                        optional: trigger.optional,
                        cost: trigger.cost.clone(),
                    });
                }
            }

            (card.controller, power, triggers)
        };

        // Process each trigger - borrow is released, safe to call execute_effect
        for trigger_data in triggers {
            // For optional triggers with costs, check if cost can be paid
            let mut sacrifice_target: Option<CardId> = None;
            let mut sacrificed_power: u8 = 0;

            if trigger_data.optional {
                if let Some(ref cost) = trigger_data.cost {
                    // Check if sacrifice cost can be paid
                    if let Some((count, pattern)) = cost.get_sacrifice_pattern() {
                        if !self.can_pay_sacrifice_pattern(pattern, count, attacker_id, controller) {
                            log::debug!(
                                "Skipping optional attack trigger on {} - sacrifice cost not payable (need {} {})",
                                trigger_data.card_name,
                                count,
                                pattern
                            );
                            continue; // Skip this trigger - can't pay cost
                        }

                        // Choose which permanent to sacrifice (AI heuristic: pick lowest P/T creature)
                        sacrifice_target = self.choose_sacrifice_target(pattern, attacker_id, controller);

                        // Get the power of the creature we're about to sacrifice
                        if let Some(sac_id) = sacrifice_target {
                            if let Ok(sac_card) = self.cards.get(sac_id) {
                                sacrificed_power = sac_card.current_power().max(0) as u8;
                            }
                        }
                    }
                    // TODO: Check other cost types (mana, life, etc.)
                }
            }

            // Log the trigger (official game action)
            self.logger.gamelog(&format!(
                "Trigger: {} - {}",
                trigger_data.card_name, trigger_data.description
            ));

            // Execute sacrifice cost first (if any)
            if let Some(sac_target) = sacrifice_target {
                if let Ok(sac_card) = self.cards.get(sac_target) {
                    let sac_name = sac_card.name.to_string();
                    let sac_owner = sac_card.owner;
                    log::info!(
                        "Sacrificing {} ({}) for attack trigger cost",
                        sac_name,
                        sac_target.as_u32()
                    );

                    self.logger
                        .gamelog(&format!("Sacrifices {} for trigger cost", sac_name));

                    // Move from battlefield to graveyard
                    self.move_card(sac_target, Zone::Battlefield, Zone::Graveyard, sac_owner)?;

                    // Check sacrifice triggers (e.g., Pirate Peddlers)
                    self.check_triggers(TriggerEvent::Sacrificed, sac_target)?;
                }
            }

            // Execute each effect with placeholder resolution
            for mut effect in trigger_data.effects {
                // Fill in placeholder values in trigger effects
                match &mut effect {
                    Effect::Firebend {
                        controller: ctrl,
                        amount,
                    } if ctrl.as_u32() == 0 => {
                        // Resolve placeholder controller to the actual creature controller
                        // amount=0 means "use creature's power" (Firebending X)
                        // amount=254 means "use sacrificed creature's power" (Fire Lord Ozai)
                        let actual_amount = match *amount {
                            0 => creature_power,
                            254 => sacrificed_power,
                            n => n,
                        };

                        effect = Effect::Firebend {
                            controller,
                            amount: actual_amount,
                        };

                        // Log the firebend trigger (using card_name already extracted)
                        if actual_amount > 0 {
                            self.logger.gamelog(&format!(
                                "{} triggers Firebending {} (adding {} {{R}} to combat mana)",
                                trigger_data.card_name, actual_amount, actual_amount
                            ));
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
    ///
    /// # Errors
    ///
    /// Returns an error if the target player does not exist.
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

            // Log damage with new life total (use gamelog for official action)
            let message = format!("{} takes {} damage (life: {})", player_name, amount, new_life);
            self.logger.gamelog(&message);

            return Ok(());
        }

        Err(MtgError::InvalidAction("Invalid damage target".to_string()))
    }

    /// Deal damage to a creature
    ///
    /// MTG Rules 120.3: Damage dealt to a creature or planeswalker remains until the cleanup step
    /// MTG Rules 704.5g: State-based actions check if creature has lethal damage and destroys it
    ///
    /// # Errors
    ///
    /// Returns an error if the target is not a creature or cannot be found.
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
    ///
    /// # Errors
    ///
    /// Returns an error if the card cannot be tapped for mana.
    pub fn tap_for_mana(&mut self, player_id: PlayerId, card_id: CardId) -> Result<()> {
        // Create an empty cost hint
        let empty_cost = crate::core::ManaCost::new();
        self.tap_for_mana_for_cost(player_id, card_id, &empty_cost)
    }

    /// Tap a permanent for mana with a cost hint to guide color production
    ///
    /// This method handles both:
    /// - Lands with implicit mana abilities (based on subtypes)
    /// - Creatures/artifacts with explicit mana abilities (e.g., "Guy in the Chair", Black Lotus)
    ///
    /// For mana abilities with sacrifice costs (e.g., Black Lotus), this will also
    /// sacrifice the permanent after activating the mana ability.
    ///
    /// # Errors
    ///
    /// Returns an error if the card cannot be tapped for mana or is already tapped.
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

        // Check for explicit mana ability and its cost before tapping
        // We need both the mana production and the full cost (for sacrifice, etc.)
        let (explicit_mana, mana_ability_cost) = if !is_land && has_mana_ability {
            // For non-lands (creatures, artifacts) with mana abilities,
            // extract the mana from the activated ability's AddMana effect
            // and also capture the full cost for non-tap costs (like sacrifice)
            card.activated_abilities
                .iter()
                .find(|ab| ab.is_mana_ability)
                .map(|ab| {
                    let mana = ab.effects.iter().find_map(|effect| {
                        if let crate::core::Effect::AddMana { mana, .. } = effect {
                            Some(*mana)
                        } else {
                            None
                        }
                    });
                    (mana, Some(ab.cost.clone()))
                })
                .unwrap_or((None, None))
        } else {
            (None, None)
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

                // Log visible message (use gamelog for official action)
                if self.logger.verbosity() >= crate::game::VerbosityLevel::Normal {
                    let card = self.cards.get(card_id).ok();
                    let name = card.map(|c| c.name.as_str()).unwrap_or("Unknown");
                    let message = format!("Tap {} for {{{}}}", name, color_symbol);
                    self.logger.gamelog(&message);
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

                // Log visible message (use gamelog for official action)
                if self.logger.verbosity() >= crate::game::VerbosityLevel::Normal {
                    let card = self.cards.get(card_id).ok();
                    let name = card.map(|c| c.name.as_str()).unwrap_or("Unknown");
                    let message = format!("Tap {} for mana", name);
                    self.logger.gamelog(&message);
                }
            }

            // Pay any additional costs from the mana ability (e.g., sacrifice for Black Lotus)
            // For non-land mana sources, handle sacrifice costs before returning
            if let Some(cost) = mana_ability_cost {
                use crate::core::Cost;
                match cost {
                    Cost::Tap => {
                        // Already handled above
                    }
                    Cost::SacrificePattern { .. } | Cost::Sacrifice { .. } => {
                        // Pay the sacrifice cost (moves permanent to graveyard)
                        self.pay_ability_cost(player_id, card_id, &cost)?;
                    }
                    Cost::Composite(costs) => {
                        // For composite costs, pay everything except tap (already paid)
                        for sub_cost in costs {
                            if !matches!(sub_cost, Cost::Tap) {
                                self.pay_ability_cost(player_id, card_id, &sub_cost)?;
                            }
                        }
                    }
                    // Other costs not yet handled by mana abilities:
                    Cost::Untap
                    | Cost::Mana(_)
                    | Cost::TapAndMana(_)
                    | Cost::PayLife { .. }
                    | Cost::Discard { .. }
                    | Cost::Waterbend { .. } => {
                        // These cost types aren't currently used in mana ability costs
                    }
                }
            }

            return Ok(());
        }

        // Add mana to player's pool based on land type
        // For basic lands and simple cases, check subtypes
        // For dual lands (e.g., Underground Sea = Island Swamp), we need smarter logic
        // First, check subtypes and mana production cache before we borrow player_mut
        // Get mana production info and build available colors from BOTH subtypes AND mana production cache
        // This handles both basic lands (with subtypes) and non-basic dual lands (with Choice abilities)
        let (is_any_color_land, produces_colorless, available_colors) = {
            let card = self.cards.get(card_id)?;
            // Use pre-computed cache for mana production type (derived from abilities, not text)
            let is_any_color = matches!(
                card.cache.mana_production.kind,
                crate::core::ManaProductionKind::AnyColor
            );
            let is_colorless = matches!(
                card.cache.mana_production.kind,
                crate::core::ManaProductionKind::Colorless
            );

            // Build available_colors from BOTH sources:
            // 1. Land subtypes (Island, Forest, etc.) - for basic/dual lands with land types
            // 2. ManaProductionKind::Choice - for non-basic duals like Blooming Marsh
            let mut colors = Vec::new();

            // First, add colors from land subtypes
            if card.cache.has_plains_subtype {
                colors.push(crate::core::Color::White);
            }
            if card.cache.has_island_subtype {
                colors.push(crate::core::Color::Blue);
            }
            if card.cache.has_swamp_subtype {
                colors.push(crate::core::Color::Black);
            }
            if card.cache.has_mountain_subtype {
                colors.push(crate::core::Color::Red);
            }
            if card.cache.has_forest_subtype {
                colors.push(crate::core::Color::Green);
            }

            // Second, add colors from mana production cache (for non-basic lands)
            // This handles lands without basic land subtypes
            use crate::core::ManaColor;
            match &card.cache.mana_production.kind {
                crate::core::ManaProductionKind::Fixed(mana_color) => {
                    // Non-basic land that produces a fixed color (e.g., Ba Sing Se produces {G})
                    let color = match mana_color {
                        ManaColor::White => crate::core::Color::White,
                        ManaColor::Blue => crate::core::Color::Blue,
                        ManaColor::Black => crate::core::Color::Black,
                        ManaColor::Red => crate::core::Color::Red,
                        ManaColor::Green => crate::core::Color::Green,
                    };
                    if !colors.contains(&color) {
                        colors.push(color);
                    }
                }
                crate::core::ManaProductionKind::Choice(mana_colors) => {
                    // Dual/multi lands (e.g., Blooming Marsh)
                    if mana_colors.contains(ManaColor::White) && !colors.contains(&crate::core::Color::White) {
                        colors.push(crate::core::Color::White);
                    }
                    if mana_colors.contains(ManaColor::Blue) && !colors.contains(&crate::core::Color::Blue) {
                        colors.push(crate::core::Color::Blue);
                    }
                    if mana_colors.contains(ManaColor::Black) && !colors.contains(&crate::core::Color::Black) {
                        colors.push(crate::core::Color::Black);
                    }
                    if mana_colors.contains(ManaColor::Red) && !colors.contains(&crate::core::Color::Red) {
                        colors.push(crate::core::Color::Red);
                    }
                    if mana_colors.contains(ManaColor::Green) && !colors.contains(&crate::core::Color::Green) {
                        colors.push(crate::core::Color::Green);
                    }
                }
                crate::core::ManaProductionKind::AnyColor | crate::core::ManaProductionKind::Colorless => {
                    // Handled by is_any_color and is_colorless checks
                }
            }

            // Third, add chosen_color for lands like Thriving Grove
            // (cards with "choose a color" ETB effects that produce mana of that color)
            if let Some(chosen) = card.chosen_color {
                if !colors.contains(&chosen) {
                    colors.push(chosen);
                }
            }

            (is_any_color, is_colorless, colors)
        };

        // Capture log size before mana addition (before get_player_mut to avoid borrow issues)
        let prior_log_size = self.logger.log_count();

        let player = self.get_player_mut(player_id)?;

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

            // Log visible message for mana tapping (use gamelog for official action)
            if self.logger.verbosity() >= crate::game::VerbosityLevel::Normal {
                let card_name = self.cards.get(card_id).map(|c| c.name.as_str()).unwrap_or("Unknown");
                let message = format!("Tap {} for {{{}}}", card_name, color_symbol);
                self.logger.gamelog(&message);
            }
        }

        // Note: For lands, mana_ability_cost is None (set at line 1623), so no additional
        // costs need to be paid. Non-land mana sources with sacrifice costs are handled
        // in the explicit_mana path above (lines 1760-1784), which returns early.

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
    ///
    /// # Errors
    ///
    /// Returns an error if the cost cannot be paid.
    pub fn pay_ability_cost(&mut self, player_id: PlayerId, card_id: CardId, cost: &crate::core::Cost) -> Result<()> {
        use crate::core::{Cost, ManaCost};

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

                // Sacrifice the permanents (move to graveyard) and check triggers
                for sac_id in to_sacrifice.iter().take(*count as usize) {
                    let owner = self.cards.get(*sac_id)?.owner;
                    self.move_card(*sac_id, Zone::Battlefield, Zone::Graveyard, owner)?;
                    // Check sacrifice triggers (e.g., Pirate Peddlers)
                    self.check_triggers(TriggerEvent::Sacrificed, *sac_id)?;
                }

                Ok(())
            }

            Cost::Sacrifice { card_id: sac_id } => {
                // Sacrifice a specific permanent (move to graveyard)
                let owner = self.cards.get(*sac_id)?.owner;
                self.move_card(*sac_id, Zone::Battlefield, Zone::Graveyard, owner)?;
                // Check sacrifice triggers
                self.check_triggers(TriggerEvent::Sacrificed, *sac_id)
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

            Cost::Waterbend { amount } => {
                // Waterbend cost - Avatar set mechanic (like Convoke)
                // Player can tap untapped creatures/artifacts to pay for {1} each.
                // Any remaining cost must be paid with mana from the mana pool.

                // First, count available mana and tappable creatures/artifacts
                let available_mana = {
                    let player = self.get_player(player_id)?;
                    player.mana_pool.total()
                };

                // Find untapped creatures and artifacts controlled by this player
                // (excluding the source card - can't tap itself to pay its own cost)
                let battlefield_cards = self.battlefield.cards.to_vec();
                let tappable_permanents: Vec<CardId> = battlefield_cards
                    .into_iter()
                    .filter(|&cid| {
                        if cid == card_id {
                            return false; // Can't tap the source to pay its own cost
                        }
                        if let Ok(card) = self.cards.get(cid) {
                            // Must be untapped, controlled by player, and be creature or artifact
                            !card.tapped && card.controller == player_id && (card.is_creature() || card.is_artifact())
                        } else {
                            false
                        }
                    })
                    .collect();

                let total_available = available_mana + tappable_permanents.len() as u8;

                if total_available < *amount {
                    return Err(MtgError::InvalidAction(format!(
                        "Cannot pay Waterbend {}: only {} available (mana: {}, tappable: {})",
                        amount,
                        total_available,
                        available_mana,
                        tappable_permanents.len()
                    )));
                }

                // Greedily tap permanents first, then use mana pool for remainder
                let permanents_to_tap = (*amount as usize).min(tappable_permanents.len());
                let mana_needed = *amount - permanents_to_tap as u8;

                // Tap the permanents
                for &perm_id in tappable_permanents.iter().take(permanents_to_tap) {
                    if let Ok(card) = self.cards.get_mut(perm_id) {
                        card.tapped = true;
                    }
                }

                // Pay the remaining cost from mana pool
                if mana_needed > 0 {
                    let mana_cost = ManaCost::from_string(&mana_needed.to_string());
                    let player = self.get_player_mut(player_id)?;
                    player.mana_pool.pay_cost(&mana_cost).map_err(MtgError::InvalidAction)?;
                }

                Ok(())
            }
        }
    }

    /// Execute a Balance effect
    ///
    /// Balance equalizes permanents/cards of a specified type across all players.
    /// Each player with more than the minimum must sacrifice/discard down to the minimum.
    ///
    /// # Arguments
    /// * `card_type` - Type filter (e.g., "Creature", "Land", or empty for any)
    /// * `zone` - Zone to balance ("Battlefield" or "Hand")
    ///
    /// # MTG Rules
    /// - 701.17: To sacrifice means to move a permanent to graveyard
    /// - Balance card: Each player chooses, then sacrifices simultaneously
    ///
    /// Note: This is a non-interactive implementation. For proper interactive
    /// sacrifice choice (where players select which permanents to sacrifice),
    /// this must be called through the game loop which has access to controllers.
    ///
    /// # Errors
    ///
    /// Returns an error if balance effect execution fails.
    pub fn execute_balance_effect(&mut self, card_type: &str, zone: &str) -> Result<()> {
        // Get all player IDs
        let player_ids: Vec<PlayerId> = self.players.iter().map(|p| p.id).collect();

        // Handle Hand zone (discard) vs Battlefield zone (sacrifice)
        if zone == "Hand" {
            // Count cards in each player's hand
            let hand_counts: Vec<(PlayerId, usize)> = player_ids
                .iter()
                .map(|&pid| {
                    let count = self
                        .player_zones
                        .iter()
                        .find(|(id, _)| *id == pid)
                        .map(|(_, zones)| zones.hand.cards.len())
                        .unwrap_or(0);
                    (pid, count)
                })
                .collect();

            // Find minimum hand size
            let min_hand = hand_counts.iter().map(|(_, c)| *c).min().unwrap_or(0);

            // Log the balance action
            self.logger
                .gamelog(&format!("Balance: Hand sizes equalize to {}", min_hand));

            // Each player discards down to min (non-interactive: discard from end of hand)
            for (player_id, current_count) in hand_counts {
                if current_count > min_hand {
                    let discard_count = current_count - min_hand;

                    // Get the cards to discard (from end of hand)
                    let cards_to_discard: Vec<CardId> = self
                        .player_zones
                        .iter()
                        .find(|(id, _)| *id == player_id)
                        .map(|(_, zones)| zones.hand.cards.iter().rev().take(discard_count).copied().collect())
                        .unwrap_or_default();

                    // Discard each card
                    for card_id in cards_to_discard {
                        self.move_card(card_id, Zone::Hand, Zone::Graveyard, player_id)?;

                        // Log the discard
                        if let Ok(card) = self.cards.get(card_id) {
                            let player_name = self
                                .get_player(player_id)
                                .map(|p| p.name.to_string())
                                .unwrap_or_else(|_| "Player".to_string());
                            self.logger
                                .gamelog(&format!("{} discards {} to Balance", player_name, card.name));
                        }
                    }
                }
            }
        } else {
            // Battlefield zone - sacrifice permanents
            // Filter by card type if specified
            let counts_and_permanents: Vec<(PlayerId, usize, Vec<CardId>)> = player_ids
                .iter()
                .map(|&pid| {
                    // Get this player's permanents matching the type
                    let matching_permanents: Vec<CardId> = self
                        .battlefield
                        .cards
                        .iter()
                        .filter(|&&card_id| {
                            if let Ok(card) = self.cards.get(card_id) {
                                // Must be controlled by this player
                                if card.controller != pid {
                                    return false;
                                }
                                // Filter by card type
                                match card_type {
                                    "Creature" => card.is_creature(),
                                    "Land" => card.is_land(),
                                    "Artifact" => card.is_artifact(),
                                    "Enchantment" => card.is_enchantment(),
                                    "" => true, // Any permanent
                                    _ => true,  // Default to any
                                }
                            } else {
                                false
                            }
                        })
                        .copied()
                        .collect();

                    let count = matching_permanents.len();
                    (pid, count, matching_permanents)
                })
                .collect();

            // Find minimum count
            let min_count = counts_and_permanents.iter().map(|(_, c, _)| *c).min().unwrap_or(0);

            // Log the balance action
            let type_str = if card_type.is_empty() { "permanents" } else { card_type };
            self.logger
                .gamelog(&format!("Balance: {} equalize to {}", type_str, min_count));

            // Each player sacrifices down to min
            // Non-interactive: sacrifice from end of list (last in battlefield order)
            for (player_id, current_count, permanents) in counts_and_permanents {
                if current_count > min_count {
                    let sacrifice_count = current_count - min_count;

                    // Get permanents to sacrifice (from end of list)
                    let to_sacrifice: Vec<CardId> = permanents.into_iter().rev().take(sacrifice_count).collect();

                    // Sacrifice each permanent
                    for card_id in to_sacrifice {
                        let owner = self.cards.get(card_id)?.owner;

                        // Log before moving
                        if let Ok(card) = self.cards.get(card_id) {
                            let player_name = self
                                .get_player(player_id)
                                .map(|p| p.name.to_string())
                                .unwrap_or_else(|_| "Player".to_string());
                            self.logger
                                .gamelog(&format!("{} sacrifices {} to Balance", player_name, card.name));
                        }

                        // Check death triggers BEFORE moving the card
                        let _ = self.check_death_triggers(card_id);

                        // Move to graveyard
                        self.move_card(card_id, Zone::Battlefield, Zone::Graveyard, owner)?;

                        // Check sacrifice triggers (e.g., Pirate Peddlers)
                        let _ = self.check_triggers(TriggerEvent::Sacrificed, card_id);
                    }
                }
            }
        }

        Ok(())
    }
}

// Submodules
mod combat;
mod targeting;

#[cfg(test)]
mod tests;
