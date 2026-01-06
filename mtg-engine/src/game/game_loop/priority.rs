//! Priority system and spell resolution
//!
//! This module handles the priority system where players alternate making choices
//! until both pass in succession, then resolves spells from the stack.

use crate::core::{CardId, Effect};
use crate::game::controller::{format_choice_menu, GameStateView, PlayerController};
use crate::game::snapshot::ControllerType;
use crate::game::GameState;
use crate::{handle_choice_result, handle_choice_result_break, Result};
use smallvec::SmallVec;

use super::{GameLoop, GameResult, VerbosityLevel};

/// Note: Wildcards are intentional throughout - Effect enum has 24+ variants.
/// Priority system handles specific effect types (AddMana, GainLife, etc.) specially
/// and passes through others unchanged. Using exhaustive matching would be verbose.
#[allow(clippy::wildcard_enum_match_arm)]
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

        // Verify spell exists before proceeding
        if self.game.cards.get(spell_id).is_err() {
            return Err(crate::MtgError::EntityNotFound(spell_id.as_u32()));
        }

        // Get spell owner (needed for push_reveals even without logging)
        let spell_owner = self.game.cards.get(spell_id).unwrap().owner; // Safe: checked above

        // Get card info for logging ONLY when logging is enabled
        // This avoids String allocation and effects clone in Silent mode
        let (card_name, card_effects, card_owner) = if should_log {
            let card = self.game.cards.get(spell_id).unwrap(); // Safe: checked above
            (card.name.to_string(), card.effects.clone(), card.owner)
        } else {
            // In Silent mode, use empty placeholders (never accessed)
            (String::new(), Vec::new(), crate::core::PlayerId::new(0))
        };

        if should_log {
            let message = format!("{} ({}) resolves", card_name, spell_id);
            // Use gamelog for official game action
            self.game.logger.gamelog(&message);
        }

        // Resolve the spell (this modifies effects with target replacement)
        self.game.resolve_spell(spell_id, &targets)?;

        // Push reveals after spell resolution for network mode (server-side)
        // Spells can draw cards (e.g., "draw 3 cards", "target player draws 3"),
        // and network clients need the card IDs before their shadow GameLoop draws.
        // Push for both players since spells can target either player for draws.
        self.push_reveals(spell_owner);
        if let Some(opponent) = self.game.get_other_player_id(spell_owner) {
            self.push_reveals(opponent);
        }

        // Log effects for instants/sorceries (only when verbose logging is enabled)
        // Note: We need to manually replace placeholder targets for logging
        if should_log {
            use crate::core::{Effect, TargetRef};
            let mut target_index = 0;
            for effect in &card_effects {
                // Replace placeholder targets with chosen targets for logging
                let effect_to_log = match effect {
                    Effect::DealDamage {
                        target: TargetRef::None,
                        amount,
                    } if target_index < targets.len() => {
                        // Replace placeholder with actual permanent target for logging
                        let replaced = Effect::DealDamage {
                            target: TargetRef::Permanent(targets[target_index]),
                            amount: *amount,
                        };
                        target_index += 1;
                        replaced
                    }
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
                    Effect::AddMana {
                        player,
                        mana,
                        produces_chosen_color,
                    } if player.as_u32() == 0 => Effect::AddMana {
                        player: card_owner,
                        mana: *mana,
                        produces_chosen_color: *produces_chosen_color,
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
                        .unwrap_or_else(|_| i32::from(card.current_power()));
                    let toughness = self
                        .game
                        .get_effective_toughness(spell_id)
                        .unwrap_or_else(|_| i32::from(card.current_toughness()));
                    let message = format!(
                        "{} ({}) enters the battlefield as a {}/{} creature",
                        card_name, spell_id, power, toughness
                    );
                    // Use gamelog for official game action
                    self.game.logger.gamelog(&message);
                }
            }
        }

        // Check state-based actions after spell effects resolve (MTG Rules 704.3)
        // This handles lethal damage from damage-dealing spells like Lightning Bolt
        if let Err(e) = self.game.check_lethal_damage() {
            if should_log {
                eprintln!("    Failed to check lethal damage: {e}");
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
                // Drain any pending reveals from network before processing priority
                // This ensures opponent's played cards are instantiated before we try to act on them
                self.drain_reveals();

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

                    // Log abilities for debugging network sync issues
                    if available_count > 0 && available_count <= 5 {
                        // Log the actual abilities available
                        let abilities: smallvec::SmallVec<[String; 8]> =
                            self.abilities_buffer.iter().map(|a| format!("{:?}", a)).collect();
                        log::debug!(
                            "Priority check: player {:?} has {} available abilities at action_count={}: {:?}",
                            current_priority,
                            available_count,
                            self.game.action_count(),
                            abilities
                        );
                    } else {
                        log::debug!(
                            "Priority check: player {:?} has {} available abilities, action_count={}",
                            current_priority,
                            available_count,
                            self.game.action_count()
                        );
                    }

                    // If no actions available, automatically pass priority without asking controller
                    // Only invoke controller when there's an actual choice to make
                    //
                    // EXCEPTION: Remote/Network controllers MUST always be asked:
                    // - Remote: Client-side opponent controller, we don't know hidden hand contents
                    // - Network: Server-side controller, must notify clients even on 0-ability pass
                    // The server will send the actual ability via OpponentChoice.
                    let ctrl_type = controller.get_controller_type();
                    let is_network_controlled = matches!(ctrl_type, ControllerType::Remote | ControllerType::Network);
                    if available_count == 0 && !is_network_controlled {
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

                    // IMPORTANT: Drain reveals after receiving opponent choice (network mode)
                    // During wait_for_choice(), reveals may have arrived via WebSocket for cards
                    // that the opponent is about to play. We need to process those reveals NOW
                    // before the game tries to act on the choice (e.g., cast a spell from hand).
                    self.drain_reveals();

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
                                // Only format debug message if debug state hash logging is enabled
                                if self.game.logger.debug_state_hash_enabled() {
                                    let play_msg = format!(
                                        "{} plays {} ({})",
                                        self.get_player_name(current_priority),
                                        card_name,
                                        card_id
                                    );
                                    self.game.debug_log_state_hash(&play_msg);
                                }

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
                                            // Use gamelog for official game actions
                                            self.game.logger.gamelog(&message);
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

                                // Debug: Log state hash before casting spell (only if enabled)
                                if self.game.logger.debug_state_hash_enabled() {
                                    let cast_msg = format!(
                                        "{} casts {} ({}) (putting on stack)",
                                        self.get_player_name(current_priority),
                                        card_name,
                                        card_id
                                    );
                                    self.game.debug_log_state_hash(&cast_msg);
                                }

                                if self.verbosity >= VerbosityLevel::Normal {
                                    if !self.replaying {
                                        let message = format!(
                                            "{} casts {} ({}) (putting on stack)",
                                            self.get_player_name(current_priority),
                                            card_name,
                                            card_id
                                        );
                                        // Use gamelog for official game actions
                                        self.game.logger.gamelog(&message);
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

                                // MTG Rule 601.2b: Modal choice happens BEFORE targeting
                                // Check if this is a modal spell and prompt for mode selection
                                if let Ok(Some(Effect::ModalChoice {
                                    modes,
                                    num_to_choose,
                                    min_to_choose,
                                    can_repeat_modes,
                                })) = self.game.get_modal_choice_info(card_id)
                                {
                                    // Get which modes have valid targets
                                    let valid_modes = self
                                        .game
                                        .get_valid_modes_for_spell(card_id, current_priority)
                                        .unwrap_or_default();

                                    // Filter to only modes with valid targets
                                    let valid_mode_indices: Vec<usize> = valid_modes
                                        .iter()
                                        .filter(|(_, has_targets)| *has_targets)
                                        .map(|(idx, _)| *idx)
                                        .collect();

                                    // If no modes have valid targets, the spell can't be cast legally
                                    // (this shouldn't happen if the spell was offered as castable)
                                    if valid_mode_indices.is_empty() {
                                        log::warn!(
                                            target: "priority",
                                            "Modal spell has no modes with valid targets, skipping"
                                        );
                                        // Continue without applying any modes - spell will fizzle
                                    } else {
                                        // Get mode descriptions for valid modes only
                                        let mode_descriptions: Vec<String> = valid_mode_indices
                                            .iter()
                                            .filter_map(|&idx| modes.get(idx).map(|m| m.description.clone()))
                                            .collect();

                                        // Ask controller to choose from valid modes
                                        let prior_log_size = self.game.logger.log_count();
                                        let view = GameStateView::new(self.game, current_priority);
                                        let choice = controller.choose_modes(
                                            &view,
                                            card_id,
                                            &mode_descriptions,
                                            num_to_choose as usize,
                                            min_to_choose as usize,
                                            can_repeat_modes,
                                        );
                                        let selected_modes =
                                            handle_choice_result_break!(choice, self.game, current_priority);

                                        // Map the controller's selection (indices into valid_mode_indices)
                                        // back to the original mode indices
                                        let original_indices: Vec<usize> = selected_modes
                                            .iter()
                                            .filter_map(|&idx| valid_mode_indices.get(idx).copied())
                                            .collect();

                                        // Log the mode choice (using original indices)
                                        let replay_choice = crate::game::ReplayChoice::Modes(
                                            original_indices.iter().copied().collect(),
                                        );
                                        self.log_choice_point(current_priority, Some(replay_choice), prior_log_size);

                                        // Apply selected modes to the spell (replaces ModalChoice with mode effects)
                                        if let Err(e) = self.game.apply_selected_modes(card_id, &original_indices) {
                                            log::warn!(
                                                target: "priority",
                                                "Failed to apply selected modes: {}",
                                                e
                                            );
                                        }

                                        // Log which mode was chosen
                                        if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                            for &mode_idx in &original_indices {
                                                if let Some(mode) = modes.get(mode_idx) {
                                                    let message = format!(
                                                        "{} chooses mode: {}",
                                                        self.get_player_name(current_priority),
                                                        mode.description
                                                    );
                                                    self.game.logger.gamelog(&message);
                                                }
                                            }
                                        }
                                    }
                                }

                                // Get valid targets BEFORE calling cast_spell_8_step
                                // (we can't borrow controller inside the closure)
                                // Note: For modal spells, this runs AFTER mode selection
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
                                        // Use gamelog for official game action
                                        self.game.logger.gamelog(&message);
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
                                        // Reuse sources_to_tap_buffer to avoid allocation
                                        let resolver = GreedyManaResolver::new();
                                        self.sources_to_tap_buffer.clear();
                                        resolver.compute_tap_order(
                                            mana_cost,
                                            mana_sources,
                                            &mut self.sources_to_tap_buffer,
                                        );

                                        // Track remaining cost as hint for each land tap
                                        // This ensures dual lands produce the right color based on what's still needed
                                        let mut remaining_hint = *mana_cost;

                                        // Tap lands to add mana to pool
                                        for &source_id in &self.sources_to_tap_buffer {
                                            if let Err(e) = self.game.tap_for_mana_for_cost(
                                                current_priority,
                                                source_id,
                                                &remaining_hint,
                                            ) {
                                                if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                                    eprintln!("    Failed to tap land for mana: {e}");
                                                }
                                                // Continue to next source - partial payment might still work
                                            }

                                            // Update remaining hint based on what color this source produced
                                            if let Some(card) = self.game.cards.try_get(source_id) {
                                                use crate::core::{ManaColor, ManaProductionKind};
                                                match &card.cache.mana_production.kind {
                                                    ManaProductionKind::Fixed(color) => match color {
                                                        ManaColor::White => {
                                                            remaining_hint.white =
                                                                remaining_hint.white.saturating_sub(1)
                                                        }
                                                        ManaColor::Blue => {
                                                            remaining_hint.blue = remaining_hint.blue.saturating_sub(1)
                                                        }
                                                        ManaColor::Black => {
                                                            remaining_hint.black =
                                                                remaining_hint.black.saturating_sub(1)
                                                        }
                                                        ManaColor::Red => {
                                                            remaining_hint.red = remaining_hint.red.saturating_sub(1)
                                                        }
                                                        ManaColor::Green => {
                                                            remaining_hint.green =
                                                                remaining_hint.green.saturating_sub(1)
                                                        }
                                                    },
                                                    ManaProductionKind::Colorless => {
                                                        if remaining_hint.colorless > 0 {
                                                            remaining_hint.colorless =
                                                                remaining_hint.colorless.saturating_sub(1);
                                                        } else {
                                                            remaining_hint.generic =
                                                                remaining_hint.generic.saturating_sub(1);
                                                        }
                                                    }
                                                    ManaProductionKind::Choice(_) | ManaProductionKind::AnyColor => {
                                                        // Deduct in same priority order as tap_for_mana_for_cost
                                                        if remaining_hint.white > 0 {
                                                            remaining_hint.white =
                                                                remaining_hint.white.saturating_sub(1);
                                                        } else if remaining_hint.blue > 0 {
                                                            remaining_hint.blue = remaining_hint.blue.saturating_sub(1);
                                                        } else if remaining_hint.black > 0 {
                                                            remaining_hint.black =
                                                                remaining_hint.black.saturating_sub(1);
                                                        } else if remaining_hint.red > 0 {
                                                            remaining_hint.red = remaining_hint.red.saturating_sub(1);
                                                        } else if remaining_hint.green > 0 {
                                                            remaining_hint.green =
                                                                remaining_hint.green.saturating_sub(1);
                                                        } else {
                                                            remaining_hint.generic =
                                                                remaining_hint.generic.saturating_sub(1);
                                                        }
                                                    }
                                                }
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
                                            crate::core::Effect::AddMana {
                                                player,
                                                mana,
                                                produces_chosen_color,
                                            } if player.as_u32() == 0 => {
                                                // Replace placeholder with current player
                                                crate::core::Effect::AddMana {
                                                    player: current_priority,
                                                    mana: *mana,
                                                    produces_chosen_color: *produces_chosen_color,
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
                                            // Self-targeting pump: "This creature gets +X/+Y"
                                            // When no targets were chosen (ability doesn't target), use source card
                                            crate::core::Effect::PumpCreature {
                                                target,
                                                power_bonus,
                                                toughness_bonus,
                                            } if target.as_u32() == 0 && chosen_targets_vec.is_empty() => {
                                                crate::core::Effect::PumpCreature {
                                                    target: card_id, // Target self (the source of the ability)
                                                    power_bonus: *power_bonus,
                                                    toughness_bonus: *toughness_bonus,
                                                }
                                            }
                                            // Self-targeting PutCounter: "Put counter on this creature"
                                            // When Defined$ Self is used, no targets are chosen - use source card
                                            crate::core::Effect::PutCounter {
                                                target,
                                                counter_type,
                                                amount,
                                            } if target.as_u32() == 0 && chosen_targets_vec.is_empty() => {
                                                crate::core::Effect::PutCounter {
                                                    target: card_id, // Target self (the source of the ability)
                                                    counter_type: *counter_type,
                                                    amount: *amount,
                                                }
                                            }
                                            // Targeted PutCounter: "Put counter on target creature"
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
                                            // Self-targeting SetBasePowerToughness (Animate): "This creature has base P/T X/Y"
                                            // When Defined$ Self is used, no targets are chosen - use source card
                                            crate::core::Effect::SetBasePowerToughness {
                                                target,
                                                power,
                                                toughness,
                                            } if target.as_u32() == 0 && chosen_targets_vec.is_empty() => {
                                                crate::core::Effect::SetBasePowerToughness {
                                                    target: card_id, // Target self (the source of the ability)
                                                    power: *power,
                                                    toughness: *toughness,
                                                }
                                            }
                                            // Targeted SetBasePowerToughness: "Target creature has base P/T X/Y"
                                            crate::core::Effect::SetBasePowerToughness {
                                                target,
                                                power,
                                                toughness,
                                            } if target.as_u32() == 0 && !chosen_targets_vec.is_empty() => {
                                                crate::core::Effect::SetBasePowerToughness {
                                                    target: chosen_targets_vec[0],
                                                    power: *power,
                                                    toughness: *toughness,
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
                                            // GrantCantBeBlocked: "Target creature can't be blocked this turn"
                                            // Used by Deserter's Disciple
                                            crate::core::Effect::GrantCantBeBlocked { target }
                                                if target.as_u32() == 0 && !chosen_targets_vec.is_empty() =>
                                            {
                                                crate::core::Effect::GrantCantBeBlocked {
                                                    target: chosen_targets_vec[0],
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

                                    // Push reveals after ability effects for network mode (server-side)
                                    // Abilities can draw cards, and clients need the card IDs before drawing
                                    self.push_reveals(current_priority);
                                    if let Some(opponent) = self.game.get_other_player_id(current_priority) {
                                        self.push_reveals(opponent);
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
                            crate::core::SpellAbility::CastFromExile {
                                card_id,
                                alternative_cost,
                                effect_id,
                            } => {
                                // Cast from exile using alternative cost (Airbend, Suspend, etc.)
                                // Similar to CastSpell but card comes from exile instead of hand

                                let card_name = self
                                    .game
                                    .cards
                                    .get(card_id)
                                    .map(|c| c.name.to_string())
                                    .unwrap_or_else(|_| "Unknown".to_string());

                                if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                    let message = format!(
                                        "{} casts {} from exile for {} (was airbended)",
                                        self.get_player_name(current_priority),
                                        card_name,
                                        alternative_cost
                                    );
                                    self.game.logger.gamelog(&message);
                                }

                                // Move card from exile to stack
                                // First, find which player's exile zone has this card
                                let owner = self
                                    .game
                                    .cards
                                    .get(card_id)
                                    .map(|c| c.owner)
                                    .unwrap_or(current_priority);

                                if let Err(e) = self.game.move_card(
                                    card_id,
                                    crate::zones::Zone::Exile,
                                    crate::zones::Zone::Stack,
                                    owner,
                                ) {
                                    if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                        self.game.logger.normal(&format!("Error moving from exile: {e}"));
                                    }
                                    consecutive_passes += 1;
                                    current_priority = if current_priority == active_player {
                                        non_active_player
                                    } else {
                                        active_player
                                    };
                                    continue;
                                }

                                // Pay the alternative cost (not the card's mana cost)
                                self.mana_engine.update_mut(self.game, current_priority);
                                use crate::game::mana_payment::{GreedyManaResolver, ManaPaymentResolver};

                                let mana_sources = self.mana_engine.all_sources();
                                // Reuse sources_to_tap_buffer to avoid allocation
                                let resolver = GreedyManaResolver::new();
                                self.sources_to_tap_buffer.clear();
                                resolver.compute_tap_order(
                                    &alternative_cost,
                                    mana_sources,
                                    &mut self.sources_to_tap_buffer,
                                );

                                let mut remaining_hint = alternative_cost;
                                for &source_id in &self.sources_to_tap_buffer {
                                    if let Err(e) =
                                        self.game
                                            .tap_for_mana_for_cost(current_priority, source_id, &remaining_hint)
                                    {
                                        if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                            self.game.logger.normal(&format!("Failed to tap: {e}"));
                                        }
                                    }
                                    // Deduct from remaining hint (simplified - assume 1 generic each)
                                    remaining_hint.generic = remaining_hint.generic.saturating_sub(1);
                                }

                                // Remove the persistent effect that granted this cast permission
                                self.game.persistent_effects.remove(effect_id);

                                // Spell is now on the stack - will resolve when both players pass
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
                self.resolve_top_spell_from_stack_interactive(spell_id, controller1, controller2)?;
                // After resolving a spell, players get priority again
                // Loop continues to give priority
            } else {
                // Stack was reported non-empty but has no cards (shouldn't happen)
                break;
            }
        }

        Ok(None)
    }

    /// Resolve a spell from the stack with interactive effect handling
    ///
    /// This wraps `resolve_top_spell_from_stack` and handles interactive effects
    /// like Balance that require player choices.
    fn resolve_top_spell_from_stack_interactive(
        &mut self,
        spell_id: CardId,
        controller1: &mut dyn PlayerController,
        controller2: &mut dyn PlayerController,
    ) -> Result<()> {
        // Check if this spell has any Balance effects before resolving
        // Also capture SVars for SubAbility resolution
        let (balance_effects, svars): (Vec<_>, std::collections::HashMap<String, String>) =
            if let Ok(card) = self.game.cards.get(spell_id) {
                let effects = card
                    .effects
                    .iter()
                    .filter_map(|e| {
                        if let crate::core::Effect::Balance {
                            card_type,
                            zone,
                            sub_ability,
                        } = e
                        {
                            Some((card_type.clone(), zone.clone(), sub_ability.clone()))
                        } else {
                            None
                        }
                    })
                    .collect();
                (effects, card.svars.clone())
            } else {
                (Vec::new(), std::collections::HashMap::new())
            };

        // Resolve the spell normally (Balance effects are no-ops in execute_effect)
        self.resolve_top_spell_from_stack(spell_id)?;

        // Now handle any Balance effects interactively, including SubAbility chains
        for (card_type, zone, sub_ability) in balance_effects {
            self.resolve_balance_effect_chain(
                &card_type,
                &zone,
                sub_ability.as_deref(),
                &svars,
                controller1,
                controller2,
            )?;
        }

        Ok(())
    }

    /// Resolve a Balance effect interactively, asking each player to choose sacrifices
    ///
    /// Balance equalizes permanents/cards of a specified type across all players.
    /// Each player with more than the minimum must choose which permanents to sacrifice.
    fn resolve_balance_effect_interactive(
        &mut self,
        card_type: &str,
        zone: &str,
        controller1: &mut dyn PlayerController,
        controller2: &mut dyn PlayerController,
    ) -> Result<()> {
        use crate::zones::Zone;

        let player_ids: Vec<_> = self.game.players.iter().map(|p| p.id).collect();

        if zone == "Hand" {
            // Hand balancing - use existing non-interactive implementation for now
            // TODO: Make hand discard interactive too
            self.game.execute_balance_effect(card_type, zone)?;
        } else {
            // Battlefield - interactive sacrifice
            // First, compute what each player needs to sacrifice
            let counts_and_permanents: Vec<_> = player_ids
                .iter()
                .map(|&pid| {
                    let matching_permanents: Vec<CardId> = self
                        .game
                        .battlefield
                        .cards
                        .iter()
                        .filter(|&&card_id| {
                            if let Ok(card) = self.game.cards.get(card_id) {
                                if card.controller != pid {
                                    return false;
                                }
                                match card_type {
                                    "Creature" => card.is_creature(),
                                    "Land" => card.is_land(),
                                    "Artifact" => card.is_artifact(),
                                    "Enchantment" => card.is_enchantment(),
                                    "" => true,
                                    _ => true,
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

            let min_count = counts_and_permanents.iter().map(|(_, c, _)| *c).min().unwrap_or(0);

            // Log the balance action
            let type_str = if card_type.is_empty() { "permanents" } else { card_type };
            self.game
                .logger
                .gamelog(&format!("Balance: {} equalize to {}", type_str, min_count));

            // Process each player who needs to sacrifice
            for (player_id, current_count, valid_permanents) in counts_and_permanents {
                if current_count <= min_count {
                    continue; // This player doesn't need to sacrifice
                }

                let sacrifice_count = current_count - min_count;

                // Get the appropriate controller for this player
                let controller: &mut dyn PlayerController = if player_id == controller1.player_id() {
                    controller1
                } else {
                    controller2
                };

                // Ask player to choose which permanents to sacrifice
                let view = GameStateView::new(self.game, player_id);
                let prior_log_size = self.game.logger.log_count();

                let choice_result =
                    controller.choose_permanents_to_sacrifice(&view, &valid_permanents, sacrifice_count, type_str);

                let to_sacrifice = handle_choice_result!(choice_result, self.game, player_id);

                // Log this choice point for snapshot/replay
                let replay_choice = crate::game::ReplayChoice::Sacrifice(to_sacrifice.clone());
                self.log_choice_point(player_id, Some(replay_choice), prior_log_size);

                // Verify correct count
                if to_sacrifice.len() != sacrifice_count {
                    return Err(crate::MtgError::InvalidAction(format!(
                        "Must sacrifice exactly {} {}, selected {}",
                        sacrifice_count,
                        type_str,
                        to_sacrifice.len()
                    )));
                }

                // Verify all selected permanents are valid
                for &perm_id in &to_sacrifice {
                    if !valid_permanents.contains(&perm_id) {
                        return Err(crate::MtgError::InvalidAction(format!(
                            "Selected permanent {:?} is not a valid {} to sacrifice",
                            perm_id, type_str
                        )));
                    }
                }

                // Sacrifice the selected permanents
                for card_id in to_sacrifice {
                    let owner = self.game.cards.get(card_id)?.owner;

                    // Log before moving
                    if let Ok(card) = self.game.cards.get(card_id) {
                        let player_name = self
                            .game
                            .get_player(player_id)
                            .map(|p| p.name.to_string())
                            .unwrap_or_else(|_| "Player".to_string());
                        self.game
                            .logger
                            .gamelog(&format!("{} sacrifices {} to Balance", player_name, card.name));
                    }

                    // Check death triggers
                    let _ = self.game.check_death_triggers(card_id);

                    // Move to graveyard
                    self.game
                        .move_card(card_id, Zone::Battlefield, Zone::Graveyard, owner)?;
                }
            }
        }

        Ok(())
    }

    /// Resolve a Balance effect with SubAbility chaining
    ///
    /// This executes the Balance effect for the specified card_type/zone, then
    /// looks up and executes any chained SubAbility from the card's SVars.
    ///
    /// Example chain: Land → Hand → Creature
    /// - First Balance effect: Land on Battlefield (sub_ability: "BalanceHands")
    /// - Look up SVar "BalanceHands": "DB$ Balance | Zone$ Hand | SubAbility$ BalanceCreatures"
    /// - Execute: Hand balancing (sub_ability: "BalanceCreatures")
    /// - Look up SVar "BalanceCreatures": "DB$ Balance | Valid$ Creature"
    /// - Execute: Creature balancing (no sub_ability)
    fn resolve_balance_effect_chain(
        &mut self,
        card_type: &str,
        zone: &str,
        sub_ability: Option<&str>,
        svars: &std::collections::HashMap<String, String>,
        controller1: &mut dyn PlayerController,
        controller2: &mut dyn PlayerController,
    ) -> Result<()> {
        // Execute this Balance effect
        self.resolve_balance_effect_interactive(card_type, zone, controller1, controller2)?;

        // If there's a SubAbility reference, look it up and execute it
        if let Some(sub_ability_name) = sub_ability {
            if let Some(svar_body) = svars.get(sub_ability_name) {
                // Parse the SVar body to get the next Balance effect parameters
                // Format: "DB$ Balance | Zone$ Hand | SubAbility$ BalanceCreatures"
                // or "DB$ Balance | Valid$ Creature"
                if let Some((next_card_type, next_zone, next_sub_ability)) = Self::parse_balance_svar(svar_body) {
                    // Recursively execute the chained effect
                    self.resolve_balance_effect_chain(
                        &next_card_type,
                        &next_zone,
                        next_sub_ability.as_deref(),
                        svars,
                        controller1,
                        controller2,
                    )?;
                }
            }
        }

        Ok(())
    }

    /// Parse a Balance SVar body to extract card_type, zone, and sub_ability
    ///
    /// Input: "DB$ Balance | Zone$ Hand | SubAbility$ BalanceCreatures"
    /// Output: Some(("Land", "Hand", Some("BalanceCreatures")))
    ///
    /// Input: "DB$ Balance | Valid$ Creature"
    /// Output: Some(("Creature", "Battlefield", None))
    fn parse_balance_svar(svar_body: &str) -> Option<(String, String, Option<String>)> {
        // Only process Balance SVars
        if !svar_body.contains("DB$ Balance") && !svar_body.contains("DB$Balance") {
            return None;
        }

        // Parse parameters by splitting on |
        let mut card_type = "Land".to_string(); // Default for Balance
        let mut zone = "Battlefield".to_string(); // Default for Balance
        let mut sub_ability = None;

        for param in svar_body.split('|') {
            let param = param.trim();
            if let Some((key, value)) = param.split_once('$') {
                match key.trim() {
                    "Valid" => card_type = value.trim().to_string(),
                    "Zone" => zone = value.trim().to_string(),
                    "SubAbility" => sub_ability = Some(value.trim().to_string()),
                    _ => {}
                }
            }
        }

        Some((card_type, zone, sub_ability))
    }
}
