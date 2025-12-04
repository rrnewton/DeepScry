//! Priority system and spell resolution
//!
//! This module handles the priority system where players alternate making choices
//! until both pass in succession, then resolves spells from the stack.

use crate::core::CardId;
use crate::game::controller::{format_choice_menu, GameStateView, PlayerController};
use crate::game::GameState;
use crate::{handle_choice_result, handle_choice_result_break, Result};
use smallvec::SmallVec;

use super::{GameLoop, GameResult, VerbosityLevel};

impl<'a> GameLoop<'a> {
    /// Resolve the top spell from the stack
    ///
    /// This removes the spell from the stack and executes its effects.
    /// Implements MTG Comprehensive Rules 608 (Resolving Spells and Abilities).
    pub(super) fn resolve_top_spell_from_stack(&mut self, spell_id: CardId) -> Result<()> {
        // Look up the targets for this spell (already stored as SmallVec)
        let targets: SmallVec<[CardId; 2]> = self
            .spell_targets
            .iter()
            .find(|(id, _)| *id == spell_id)
            .map(|(_, t)| t.clone())
            .unwrap_or_default();

        // Check if verbose logging is enabled (avoids allocations when not logging)
        let should_log = self.verbosity >= VerbosityLevel::Normal && !self.replaying;

        // Get card info for logging (only clone effects if we'll actually log them)
        let (card_name, card_effects, card_owner) = if let Ok(card) = self.game.cards.get(spell_id) {
            // Only clone effects when logging is enabled - this is expensive otherwise
            let effects = if should_log { card.effects.clone() } else { Vec::new() };
            (card.name.to_string(), effects, card.owner)
        } else {
            return Err(crate::MtgError::EntityNotFound(spell_id.as_u32()));
        };

        if should_log {
            let message = format!("{} ({}) resolves", card_name, spell_id);
            self.game.logger.normal(&message);
        }

        // Resolve the spell (this modifies effects with target replacement)
        self.game.resolve_spell(spell_id, &targets)?;

        // Log effects for instants/sorceries (only when verbose logging is enabled)
        // Note: We need to manually replace placeholder targets for logging
        if should_log {
            use crate::core::Effect;
            let mut target_index = 0;
            for effect in &card_effects {
                // Replace placeholder targets with chosen targets for logging
                let effect_to_log = match effect {
                    Effect::CounterSpell { target } if target.as_u32() == 0 && target_index < targets.len() => {
                        let replaced = Effect::CounterSpell {
                            target: targets[target_index],
                        };
                        target_index += 1;
                        replaced
                    }
                    Effect::DestroyPermanent { target, restriction }
                        if target.as_u32() == 0 && target_index < targets.len() =>
                    {
                        let replaced = Effect::DestroyPermanent {
                            target: targets[target_index],
                            restriction: restriction.clone(),
                        };
                        target_index += 1;
                        replaced
                    }
                    Effect::TapPermanent { target } if target.as_u32() == 0 && target_index < targets.len() => {
                        let replaced = Effect::TapPermanent {
                            target: targets[target_index],
                        };
                        target_index += 1;
                        replaced
                    }
                    Effect::UntapPermanent { target } if target.as_u32() == 0 && target_index < targets.len() => {
                        let replaced = Effect::UntapPermanent {
                            target: targets[target_index],
                        };
                        target_index += 1;
                        replaced
                    }
                    Effect::PumpCreature {
                        target,
                        power_bonus,
                        toughness_bonus,
                    } if target.as_u32() == 0 && target_index < targets.len() => {
                        let replaced = Effect::PumpCreature {
                            target: targets[target_index],
                            power_bonus: *power_bonus,
                            toughness_bonus: *toughness_bonus,
                        };
                        target_index += 1;
                        replaced
                    }
                    Effect::ExilePermanent { target } if target.as_u32() == 0 && target_index < targets.len() => {
                        let replaced = Effect::ExilePermanent {
                            target: targets[target_index],
                        };
                        target_index += 1;
                        replaced
                    }
                    // Player-targeting effects: resolve placeholder (0) to card owner
                    Effect::AddMana { player, mana } if player.as_u32() == 0 => Effect::AddMana {
                        player: card_owner,
                        mana: *mana,
                    },
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
                    _ => effect.clone(),
                };

                self.log_effect_execution(&card_name, spell_id, &effect_to_log, card_owner);
            }

            // Check if it's a permanent entering battlefield
            if let Ok(card) = self.game.cards.get(spell_id) {
                if card.is_creature() {
                    // Get effective P/T applying all continuous effects (CR 613)
                    let power = self
                        .game
                        .get_effective_power(spell_id)
                        .unwrap_or(card.current_power() as i32);
                    let toughness = self
                        .game
                        .get_effective_toughness(spell_id)
                        .unwrap_or(card.current_toughness() as i32);
                    let message = format!(
                        "{} ({}) enters the battlefield as a {}/{} creature",
                        card_name, spell_id, power, toughness
                    );
                    self.game.logger.normal(&message);
                }
            }
        }

        // Remove the spell from our targets tracking
        self.spell_targets.retain(|(id, _)| *id != spell_id);

        Ok(())
    }

    /// Priority round - players get chances to act until both pass
    ///
    /// This implements the priority system where players alternate making choices
    /// until both pass in succession, then resolves spells from the stack.
    ///
    /// ## MTG Rules Implementation
    /// - Gets all available spell abilities (lands, spells, abilities)
    /// - Calls controller.choose_spell_ability_to_play() for each priority window
    /// - Handles the chosen ability appropriately:
    ///   - PlayLand: Resolves directly (no stack)
    ///   - CastSpell: Puts spell on stack (MTG Rules 601)
    ///   - ActivateAbility: TODO - should go on stack for non-mana abilities
    /// - When both players pass with spells on stack, resolves top spell (MTG Rules 117.4)
    /// - Repeats until stack is empty and both players pass
    pub(super) fn priority_round(
        &mut self,
        controller1: &mut dyn PlayerController,
        controller2: &mut dyn PlayerController,
    ) -> Result<Option<GameResult>> {
        let active_player = self.game.turn.active_player;
        let non_active_player = self
            .game
            .get_other_player_id(active_player)
            .expect("Should have non-active player");

        // Outer loop: resolve stack until empty
        loop {
            // Active player gets priority first in each round
            let mut current_priority = active_player;
            let mut consecutive_passes = 0;
            let mut action_count = 0;
            const MAX_ACTIONS_PER_PRIORITY: usize = 1000;

            // Inner loop: pass priority until both players pass
            while consecutive_passes < 2 {
                // Safety check to prevent infinite loops
                action_count += 1;
                if action_count > MAX_ACTIONS_PER_PRIORITY {
                    return Err(crate::MtgError::InvalidAction(format!(
                        "Priority round exceeded max actions ({MAX_ACTIONS_PER_PRIORITY}), possible infinite loop"
                    )));
                }

                // Get the appropriate controller
                let controller: &mut dyn PlayerController = if current_priority == controller1.player_id() {
                    controller1
                } else {
                    controller2
                };

                // Loop to allow undo/retry for spell ability choices
                let choice = loop {
                    // Get all available spell abilities for this player.
                    //
                    // OPTIMIZATION: get_available_spell_abilities now returns &[SpellAbility] from a
                    // reused internal buffer, eliminating repeated Vec allocations. We check emptiness
                    // first (no copy needed), then copy into SmallVec only when there are abilities
                    // (avoiding heap allocation for typical hand sizes up to 16 cards).
                    let available_count = self.get_available_spell_abilities(current_priority).len();

                    // If no actions available, automatically pass priority without asking controller
                    // Only invoke controller when there's an actual choice to make
                    if available_count == 0 {
                        // No available actions - automatically pass priority
                        break None;
                    }

                    // Copy abilities into SmallVec now that we know there are some.
                    // SmallVec<[_; 16]> covers typical hand sizes without heap allocation.
                    // We need a copy because self.game is accessed later in the loop.
                    let available: smallvec::SmallVec<[_; 16]> = self.abilities_buffer.iter().cloned().collect();

                    // Clear replay mode if all choices have been replayed
                    // This happens BEFORE checking stop conditions, so a snapshot taken here will NOT
                    // include the upcoming choice (which hasn't been presented yet)
                    //
                    // We stay in replay mode until BOTH conditions are met:
                    // 1. All intra-turn choices have been replayed (replay_choices_remaining == 0)
                    // 2. We've passed the baseline choice count from the snapshot
                    //
                    // This ensures that automatic actions (like draws) that happen before the first
                    // NEW choice point are properly suppressed, avoiding duplicate logging.
                    if self.replaying
                        && self.replay_choices_remaining == 0
                        && (self.choice_counter as usize) >= self.baseline_choice_count
                    {
                        eprintln!(
                            "🔍 [REPLAY_CLEAR_BEFORE_CHOICE] choice_counter={}, baseline={}, CLEARING replay mode",
                            self.choice_counter, self.baseline_choice_count
                        );
                        self.replaying = false;
                        if self.verbosity >= VerbosityLevel::Verbose {
                            println!("✅ REPLAY MODE COMPLETE - will present new choice to controller");
                        }
                    } else if self.replaying {
                        eprintln!(
                            "🔍 [REPLAY_STILL_ACTIVE] choice_counter={}, baseline={}, remaining={}",
                            self.choice_counter, self.baseline_choice_count, self.replay_choices_remaining
                        );
                    }

                    // PREAMBLE: Check stop conditions BEFORE printing menu
                    // This ensures snapshots are taken BEFORE presenting the choice to the controller,
                    // and prevents duplicate menu printing when resuming from snapshot.
                    if let Some(result) = self.check_stop_conditions(controller, current_priority)? {
                        return Ok(Some(result));
                    }

                    // Print prompt AFTER checking stop conditions to avoid duplicate output
                    {
                        let view = GameStateView::new(self.game, current_priority);
                        // Print spell ability menu (controlled by show_choice_menu flag)
                        if view.logger().should_show_choice_menu() && !available.is_empty() {
                            print!("{}", format_choice_menu(&view, &available));
                        }
                    } // Drop view before mutable borrow

                    // Ask controller to choose one (or None to pass)
                    // Capture log size BEFORE asking controller (before controller logs its choice)
                    let prior_log_size = self.game.logger.log_count();
                    let view = GameStateView::new(self.game, current_priority);
                    let choice_result = controller.choose_spell_ability_to_play(&view, &available);
                    let choice_value = handle_choice_result!(choice_result, self.game, current_priority);

                    // Log this choice point for snapshot/replay
                    let replay_choice = crate::game::ReplayChoice::SpellAbility(choice_value.clone());
                    self.log_choice_point(current_priority, Some(replay_choice), prior_log_size);

                    break choice_value;
                };

                match choice {
                    None => {
                        // Controller chose to pass priority
                        consecutive_passes += 1;
                        let view = GameStateView::new(self.game, current_priority);
                        controller.on_priority_passed(&view);

                        // Switch priority to other player
                        current_priority = if current_priority == active_player {
                            non_active_player
                        } else {
                            active_player
                        };
                    }
                    Some(ability) => {
                        // Controller chose an ability to play
                        consecutive_passes = 0; // Reset pass counter

                        match ability {
                            crate::core::SpellAbility::PlayLand { card_id } => {
                                // Debug: Log state hash before playing land
                                let card_name = self
                                    .game
                                    .cards
                                    .get(card_id)
                                    .map(|c| c.name.as_str())
                                    .unwrap_or("Unknown");
                                let play_msg = format!(
                                    "{} plays {} ({})",
                                    self.get_player_name(current_priority),
                                    card_name,
                                    card_id
                                );
                                self.game.debug_log_state_hash(&play_msg);

                                // Play land - resolves directly (no stack)
                                if let Err(e) = self.game.play_land(current_priority, card_id) {
                                    if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                        eprintln!("  Error playing land: {e}");
                                    }
                                    // Treat failed land play like passing priority to prevent infinite loops
                                    // This can happen if controller makes invalid choices
                                    consecutive_passes += 1;
                                    current_priority = if current_priority == active_player {
                                        non_active_player
                                    } else {
                                        active_player
                                    };
                                    continue;
                                } else {
                                    let card_name = self
                                        .game
                                        .cards
                                        .get(card_id)
                                        .map(|c| c.name.as_str())
                                        .unwrap_or("Unknown");

                                    if self.verbosity >= VerbosityLevel::Normal {
                                        if !self.replaying {
                                            let message = format!(
                                                "{} plays {} ({})",
                                                self.get_player_name(current_priority),
                                                card_name,
                                                card_id
                                            );
                                            self.game.logger.normal(&message);
                                        } else if self.verbosity >= VerbosityLevel::Verbose {
                                            let message = format!(
                                                "[SUPPRESSED] {} plays {} ({})",
                                                self.get_player_name(current_priority),
                                                card_name,
                                                card_id
                                            );
                                            self.game.logger.verbose(&message);
                                        }
                                    }

                                    // MTG Rules 116.3: "If a player takes a special action, that player receives priority afterward."
                                    // MTG Rules 117.3c: "If a player has priority when they cast a spell, activate an ability,
                                    //                     or take a special action, that player receives priority afterward."
                                    // Playing a land is a special action, so the player retains priority.
                                    // Continue loop with same current_priority to give player another action opportunity.
                                    continue;
                                }
                            }
                            crate::core::SpellAbility::CastSpell { card_id } => {
                                // Cast spell using 8-step process
                                // Mana will be tapped during step 6 (NOT here!)

                                let card_name = self
                                    .game
                                    .cards
                                    .get(card_id)
                                    .map(|c| c.name.to_string())
                                    .unwrap_or_else(|_| "Unknown".to_string());

                                // Debug: Log state hash before casting spell
                                let cast_msg = format!(
                                    "{} casts {} ({}) (putting on stack)",
                                    self.get_player_name(current_priority),
                                    card_name,
                                    card_id
                                );
                                self.game.debug_log_state_hash(&cast_msg);

                                if self.verbosity >= VerbosityLevel::Normal {
                                    if !self.replaying {
                                        let message = format!(
                                            "{} casts {} ({}) (putting on stack)",
                                            self.get_player_name(current_priority),
                                            card_name,
                                            card_id
                                        );
                                        self.game.logger.normal(&message);
                                    } else if self.verbosity >= VerbosityLevel::Verbose {
                                        let message = format!(
                                            "[SUPPRESSED] {} casts {} ({}) (putting on stack)",
                                            self.get_player_name(current_priority),
                                            card_name,
                                            card_id
                                        );
                                        self.game.logger.verbose(&message);
                                    }
                                }

                                // Get valid targets BEFORE calling cast_spell_8_step
                                // (we can't borrow controller inside the closure)
                                let valid_targets = self
                                    .game
                                    .get_valid_targets_for_spell(card_id)
                                    .unwrap_or_else(|_| SmallVec::new());

                                // Ask controller to choose targets (only if there are valid targets)
                                // Use SmallVec for targets - most spells have 0-2 targets (avoids heap allocation)
                                let chosen_targets_vec: SmallVec<[CardId; 2]> = if valid_targets.is_empty() {
                                    // No targets needed - spell has no targeting effects
                                    SmallVec::new()
                                } else if valid_targets.len() == 1 {
                                    // Only one valid target - auto-select without calling controller
                                    // This is not a choice, so don't log ChoicePoint
                                    smallvec::smallvec![valid_targets[0]]
                                } else {
                                    // Multiple valid targets - ask controller to choose
                                    // Capture log size BEFORE asking controller (before controller logs its choice)
                                    let prior_log_size = self.game.logger.log_count();
                                    let view = GameStateView::new(self.game, current_priority);
                                    let choice = controller.choose_targets(&view, card_id, &valid_targets);
                                    let chosen_targets =
                                        handle_choice_result_break!(choice, self.game, current_priority);

                                    // Log this choice point for snapshot/replay
                                    let replay_choice = crate::game::ReplayChoice::Targets(chosen_targets.clone());
                                    self.log_choice_point(current_priority, Some(replay_choice), prior_log_size);

                                    chosen_targets.into_iter().collect()
                                };

                                // Clone for closure (which will move it)
                                // Convert to Vec for callback signature compatibility
                                let targets_for_callback: Vec<CardId> = chosen_targets_vec.iter().copied().collect();

                                // Create targeting callback
                                let targeting_callback = move |_game: &GameState, _spell_id: CardId| {
                                    // Return the pre-selected targets
                                    targets_for_callback.clone()
                                };

                                // Pre-compute ManaEngine for mana payment (step 6)
                                // This avoids allocating a new ManaEngine inside cast_spell_8_step
                                self.mana_engine.update_mut(self.game, current_priority);

                                // Cast using 8-step process
                                if let Err(e) = self.game.cast_spell_8_step(
                                    current_priority,
                                    card_id,
                                    targeting_callback,
                                    &self.mana_engine,
                                ) {
                                    if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                        let message = format!("Error casting spell: {e}");
                                        self.game.logger.normal(&message);
                                    }
                                    // Treat failed spell cast like passing priority to prevent infinite loops
                                    // This can happen if controller makes invalid choices or mana engine has stale state
                                    consecutive_passes += 1;
                                    current_priority = if current_priority == active_player {
                                        non_active_player
                                    } else {
                                        active_player
                                    };
                                    continue;
                                } else {
                                    // Store targets for this spell (will be used when it resolves)
                                    self.spell_targets.push((card_id, chosen_targets_vec));

                                    // Spell is now on the stack - it will resolve later
                                    // when both players pass priority
                                }
                            }
                            crate::core::SpellAbility::ActivateAbility { card_id, ability_index } => {
                                // Activate ability from a permanent
                                // TODO(mtg-70): This should go on the stack for non-mana abilities

                                // Get the card and ability
                                let card_name = self.game.cards.get(card_id).ok().map(|c| c.name.clone());
                                let ability = self
                                    .game
                                    .cards
                                    .get(card_id)
                                    .ok()
                                    .and_then(|c| c.activated_abilities.get(ability_index).cloned());

                                if let Some(ability) = ability {
                                    if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                        let name = card_name.as_ref().map(|n| n.as_str()).unwrap_or("Unknown");
                                        let message = format!("{} activates ability: {}", name, ability.description);
                                        self.game.logger.normal(&message);
                                    }

                                    // Get valid targets for the ability (before paying costs)
                                    let valid_targets = self
                                        .game
                                        .get_valid_targets_for_ability(card_id, ability_index)
                                        .unwrap_or_else(|_| SmallVec::new());

                                    // Ask controller to choose targets (only if there are valid targets)
                                    let chosen_targets_vec: Vec<CardId> = if valid_targets.is_empty() {
                                        // No targets needed - ability has no targeting effects
                                        Vec::new()
                                    } else if valid_targets.len() == 1 {
                                        // Only one valid target - auto-select without calling controller
                                        // This is not a choice, so don't log ChoicePoint
                                        vec![valid_targets[0]]
                                    } else {
                                        // Multiple valid targets - ask controller to choose
                                        // Capture log size BEFORE asking controller (before controller logs its choice)
                                        let prior_log_size = self.game.logger.log_count();
                                        let view = GameStateView::new(self.game, current_priority);
                                        let choice = controller.choose_targets(&view, card_id, &valid_targets);
                                        let chosen_targets =
                                            handle_choice_result_break!(choice, self.game, current_priority);

                                        // Log this choice point for snapshot/replay
                                        let replay_choice = crate::game::ReplayChoice::Targets(chosen_targets.clone());
                                        self.log_choice_point(current_priority, Some(replay_choice), prior_log_size);

                                        chosen_targets.into_iter().collect()
                                    };

                                    // Auto-tap lands for mana costs (if the ability has a mana cost)
                                    // This is the same logic as spell casting (step 6 of cast_spell_8_step)
                                    if let Some(mana_cost) = ability.cost.get_mana_cost() {
                                        // Reuse self.mana_engine to avoid allocation on each activated ability
                                        use crate::game::mana_payment::{GreedyManaResolver, ManaPaymentResolver};

                                        self.mana_engine.update_mut(self.game, current_priority);

                                        // Get ManaSource list from engine (already built with proper production info)
                                        let mana_sources = self.mana_engine.all_sources();

                                        // Use GreedyManaResolver to compute proper tap order
                                        let resolver = GreedyManaResolver::new();
                                        let mut sources_to_tap = Vec::new();
                                        resolver.compute_tap_order(mana_cost, mana_sources, &mut sources_to_tap);

                                        // Tap lands to add mana to pool
                                        for &source_id in &sources_to_tap {
                                            if let Err(e) =
                                                self.game.tap_for_mana_for_cost(current_priority, source_id, mana_cost)
                                            {
                                                if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                                    eprintln!("    Failed to tap land for mana: {e}");
                                                }
                                                // Continue to next source - partial payment might still work
                                            }
                                        }
                                    }

                                    // Pay costs
                                    if let Err(e) = self.game.pay_ability_cost(current_priority, card_id, &ability.cost)
                                    {
                                        if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                            eprintln!("    Failed to pay cost: {e}");
                                        }
                                        // Treat failed ability activation like passing priority to prevent infinite loops
                                        consecutive_passes += 1;
                                        current_priority = if current_priority == active_player {
                                            non_active_player
                                        } else {
                                            active_player
                                        };
                                        continue;
                                    }

                                    // Execute effects immediately (not on the stack)
                                    // TODO(mtg-70): Put non-mana abilities on the stack
                                    for effect in &ability.effects {
                                        // Fix placeholder player IDs and targets for effects
                                        let fixed_effect = match effect {
                                            crate::core::Effect::AddMana { player, mana } if player.as_u32() == 0 => {
                                                // Replace placeholder with current player
                                                crate::core::Effect::AddMana {
                                                    player: current_priority,
                                                    mana: *mana,
                                                }
                                            }
                                            crate::core::Effect::GainLife { player, amount }
                                                if player.as_u32() == 0 =>
                                            {
                                                // Replace placeholder with current player
                                                crate::core::Effect::GainLife {
                                                    player: current_priority,
                                                    amount: *amount,
                                                }
                                            }
                                            crate::core::Effect::DrawCards { player, count }
                                                if player.as_u32() == 0 =>
                                            {
                                                // Replace placeholder with current player
                                                crate::core::Effect::DrawCards {
                                                    player: current_priority,
                                                    count: *count,
                                                }
                                            }
                                            // Replace placeholder targets with chosen targets
                                            crate::core::Effect::DestroyPermanent { target, restriction }
                                                if target.as_u32() == 0 && !chosen_targets_vec.is_empty() =>
                                            {
                                                crate::core::Effect::DestroyPermanent {
                                                    target: chosen_targets_vec[0],
                                                    restriction: restriction.clone(),
                                                }
                                            }
                                            crate::core::Effect::TapPermanent { target }
                                                if target.as_u32() == 0 && !chosen_targets_vec.is_empty() =>
                                            {
                                                crate::core::Effect::TapPermanent {
                                                    target: chosen_targets_vec[0],
                                                }
                                            }
                                            crate::core::Effect::UntapPermanent { target }
                                                if target.as_u32() == 0 && !chosen_targets_vec.is_empty() =>
                                            {
                                                crate::core::Effect::UntapPermanent {
                                                    target: chosen_targets_vec[0],
                                                }
                                            }
                                            crate::core::Effect::PumpCreature {
                                                target,
                                                power_bonus,
                                                toughness_bonus,
                                            } if target.as_u32() == 0 && !chosen_targets_vec.is_empty() => {
                                                crate::core::Effect::PumpCreature {
                                                    target: chosen_targets_vec[0],
                                                    power_bonus: *power_bonus,
                                                    toughness_bonus: *toughness_bonus,
                                                }
                                            }
                                            crate::core::Effect::PutCounter {
                                                target,
                                                counter_type,
                                                amount,
                                            } if target.as_u32() == 0 && !chosen_targets_vec.is_empty() => {
                                                crate::core::Effect::PutCounter {
                                                    target: chosen_targets_vec[0],
                                                    counter_type: *counter_type,
                                                    amount: *amount,
                                                }
                                            }
                                            crate::core::Effect::RemoveCounter {
                                                target,
                                                counter_type,
                                                amount,
                                            } if target.as_u32() == 0 && !chosen_targets_vec.is_empty() => {
                                                crate::core::Effect::RemoveCounter {
                                                    target: chosen_targets_vec[0],
                                                    counter_type: *counter_type,
                                                    amount: *amount,
                                                }
                                            }
                                            crate::core::Effect::AttachEquipment {
                                                source_equipment,
                                                target_creature,
                                            } if target_creature.as_u32() == 0 && !chosen_targets_vec.is_empty() => {
                                                crate::core::Effect::AttachEquipment {
                                                    source_equipment: *source_equipment,
                                                    target_creature: chosen_targets_vec[0],
                                                }
                                            }
                                            // Replace placeholder targets in DealDamage effects
                                            // This is needed for ping abilities like Prodigal Sorcerer
                                            crate::core::Effect::DealDamage {
                                                target: crate::core::TargetRef::None,
                                                amount,
                                            } if !chosen_targets_vec.is_empty() => crate::core::Effect::DealDamage {
                                                target: crate::core::TargetRef::Permanent(chosen_targets_vec[0]),
                                                amount: *amount,
                                            },
                                            // SearchLibrary needs special handling - ask controller to choose card
                                            crate::core::Effect::SearchLibrary {
                                                player,
                                                card_type_filter,
                                                destination,
                                                enters_tapped,
                                                shuffle,
                                            } if player.as_u32() == 0 => {
                                                // Handle library search with controller input
                                                let search_player = current_priority;

                                                // Get library and filter for matching cards
                                                let library_cards = self
                                                    .game
                                                    .player_zones
                                                    .iter()
                                                    .find(|(id, _)| *id == search_player)
                                                    .map(|(_, zones)| zones.library.cards.clone())
                                                    .unwrap_or_default();

                                                // Filter cards by type
                                                let mut valid_cards = Vec::new();
                                                for &card_id in &library_cards {
                                                    if let Ok(card) = self.game.cards.get(card_id) {
                                                        if crate::game::state::GameState::card_matches_search_filter(
                                                            card,
                                                            card_type_filter,
                                                        ) {
                                                            valid_cards.push(card_id);
                                                        }
                                                    }
                                                }

                                                // Ask controller to choose a card (or decline to find)
                                                let prior_log_size = self.game.logger.log_count();
                                                let view = crate::game::controller::GameStateView::new(
                                                    self.game,
                                                    current_priority,
                                                );
                                                let choice = controller.choose_from_library(&view, &valid_cards);
                                                let chosen_card_opt =
                                                    handle_choice_result_break!(choice, self.game, current_priority);

                                                // Log the choice for replay
                                                let replay_choice =
                                                    crate::game::ReplayChoice::LibrarySearch(chosen_card_opt);
                                                self.log_choice_point(
                                                    current_priority,
                                                    Some(replay_choice),
                                                    prior_log_size,
                                                );

                                                // If a card was chosen, move it to destination
                                                if let Some(chosen_card) = chosen_card_opt {
                                                    if let Err(e) = self.game.move_card(
                                                        chosen_card,
                                                        crate::zones::Zone::Library,
                                                        *destination,
                                                        search_player,
                                                    ) {
                                                        if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                                            eprintln!("    Failed to move chosen card: {e}");
                                                        }
                                                    }

                                                    // If destination is battlefield and enters_tapped is true, tap the card
                                                    if *destination == crate::zones::Zone::Battlefield && *enters_tapped
                                                    {
                                                        let _ = self.game.tap_permanent(chosen_card);
                                                    }
                                                }

                                                // Shuffle library if required
                                                if *shuffle {
                                                    self.game.shuffle_library(search_player);
                                                }

                                                // Skip execute_effect for SearchLibrary - we handled it above
                                                continue;
                                            }
                                            _ => effect.clone(),
                                        };

                                        if let Err(e) = self.game.execute_effect(&fixed_effect) {
                                            if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                                eprintln!("    Failed to execute effect: {e}");
                                            }
                                        }
                                    }

                                    // Check state-based actions after effects resolve (MTG Rules 704.3)
                                    // This handles lethal damage from damage-dealing abilities
                                    if let Err(e) = self.game.check_lethal_damage() {
                                        if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                            eprintln!("    Failed to check lethal damage: {e}");
                                        }
                                    }
                                } else if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                    eprintln!("  Ability not found");
                                }
                            }
                        }

                        // After taking an action, switch priority to other player
                        current_priority = if current_priority == active_player {
                            non_active_player
                        } else {
                            active_player
                        };
                    }
                }
            }

            // Both players passed priority
            // Check if there are spells on the stack to resolve
            if self.game.stack.is_empty() {
                // Stack is empty, priority round is complete
                break;
            }

            // Resolve the top spell from the stack (MTG Rules 608: Resolving Spells and Abilities)
            // In MTG, the stack is LIFO (Last In, First Out)
            if let Some(&spell_id) = self.game.stack.cards.last() {
                self.resolve_top_spell_from_stack(spell_id)?;
                // After resolving a spell, players get priority again
                // Loop continues to give priority
            } else {
                // Stack was reported non-empty but has no cards (shouldn't happen)
                break;
            }
        }

        Ok(None)
    }
}
