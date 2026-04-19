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
            .game
            .spell_targets
            .iter()
            .find(|(id, _)| *id == spell_id)
            .map(|(_, t)| t.clone())
            .unwrap_or_default();

        log::debug!(
            target: "priority",
            "[RESOLVE] spell_id={}, targets from spell_targets: {:?}",
            spell_id.as_u32(),
            targets.iter().map(|c| c.as_u32()).collect::<Vec<_>>()
        );

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
            // Track last resolved target for SubAbility chains using Defined$ Targeted
            let mut last_resolved_target: Option<CardId> = None;
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
                    Effect::CounterSpell { target } if target.is_placeholder() && target_index < targets.len() => {
                        let replaced = Effect::CounterSpell {
                            target: targets[target_index],
                        };
                        target_index += 1;
                        replaced
                    }
                    Effect::DestroyPermanent { target, restriction }
                        if target.is_placeholder() && target_index < targets.len() =>
                    {
                        let replaced = Effect::DestroyPermanent {
                            target: targets[target_index],
                            restriction: restriction.clone(),
                        };
                        target_index += 1;
                        replaced
                    }
                    Effect::DestroyPermanent { target, restriction }
                        if target.is_self_target() =>
                    {
                        Effect::DestroyPermanent {
                            target: spell_id,
                            restriction: restriction.clone(),
                        }
                    }
                    Effect::TapPermanent { target } if target.is_placeholder() && target_index < targets.len() => {
                        let replaced = Effect::TapPermanent {
                            target: targets[target_index],
                        };
                        target_index += 1;
                        replaced
                    }
                    Effect::UntapPermanent { target } if target.is_placeholder() && target_index < targets.len() => {
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
                        keywords_granted,
                    } if target.is_placeholder() && target_index < targets.len() => {
                        let resolved_target = targets[target_index];
                        last_resolved_target = Some(resolved_target);
                        let replaced = Effect::PumpCreature {
                            target: resolved_target,
                            power_bonus: *power_bonus,
                            toughness_bonus: *toughness_bonus,
                            keywords_granted: keywords_granted.clone(),
                        };
                        target_index += 1;
                        replaced
                    }
                    // Handle UntapPermanent with reuse_previous sentinel (from Defined$ Targeted in SubAbility)
                    Effect::UntapPermanent { target } if target.is_reuse_previous() => {
                        if let Some(prev_target) = last_resolved_target {
                            Effect::UntapPermanent { target: prev_target }
                        } else {
                            // No previous target available, use as-is (will show Unknown in log)
                            effect.clone()
                        }
                    }
                    Effect::ExilePermanent { target } if target.is_placeholder() && target_index < targets.len() => {
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
                        amount_var,
                    } if player.is_placeholder() => Effect::AddMana {
                        player: card_owner,
                        mana: *mana,
                        produces_chosen_color: *produces_chosen_color,
                        amount_var: amount_var.clone(),
                    },
                    Effect::DrawCards { player, count } if player.is_placeholder() => Effect::DrawCards {
                        player: card_owner,
                        count: *count,
                    },
                    Effect::GainLife { player, amount } if player.is_placeholder() => Effect::GainLife {
                        player: card_owner,
                        amount: *amount,
                    },
                    Effect::Mill { player, count } if player.is_placeholder() => Effect::Mill {
                        player: card_owner,
                        count: *count,
                    },
                    Effect::SearchLibrary {
                        player,
                        card_type_filter,
                        destination,
                        enters_tapped,
                        shuffle,
                    } if player.is_placeholder() => Effect::SearchLibrary {
                        player: card_owner,
                        card_type_filter: card_type_filter.clone(),
                        destination: *destination,
                        enters_tapped: *enters_tapped,
                        shuffle: *shuffle,
                    },
                    Effect::CreateToken {
                        controller,
                        token_script,
                        amount,
                        for_each_player,
                    } if *controller == crate::core::PlayerId::new(0) => Effect::CreateToken {
                        controller: card_owner,
                        token_script: token_script.clone(),
                        amount: *amount,
                        for_each_player: *for_each_player,
                    },
                    _ => effect.clone(),
                };

                self.log_effect_execution(&card_name, spell_id, &effect_to_log, card_owner);
            }

            // Check if it's a permanent entering battlefield
            if let Some(card) = self.game.cards.try_get(spell_id) {
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

            // Set starting loyalty counters for planeswalkers (MTG CR 306.5b)
            if let Some(card) = self.game.cards.try_get(spell_id) {
                if let Some(loyalty) = card.definition.loyalty {
                    let card_name_str = card.name.to_string();
                    if let Ok(card_mut) = self.game.cards.get_mut(spell_id) {
                        card_mut.add_counter(crate::core::CounterType::Loyalty, loyalty);
                    }
                    if should_log {
                        self.game.logger.gamelog(&format!(
                            "{} ({}) enters with {} loyalty",
                            card_name_str, spell_id, loyalty
                        ));
                    }
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
        // Check legendary rule (MTG CR 704.5j)
        if let Err(e) = self.game.check_legendary_rule() {
            if should_log {
                eprintln!("    Failed to check legendary rule: {e}");
            }
        }
        // Check aura attachment (MTG CR 704.5d)
        if let Err(e) = self.game.check_aura_attachment() {
            if should_log {
                eprintln!("    Failed to check aura attachment: {e}");
            }
        }
        // Check equipment attachment (MTG CR 704.5n)
        if let Err(e) = self.game.check_equipment_attachment() {
            if should_log {
                eprintln!("    Failed to check equipment attachment: {e}");
            }
        }

        // Remove the spell from our targets tracking
        self.game.spell_targets.retain(|(id, _)| *id != spell_id);

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
            // Use persistent priority state from TurnStructure (for WASM NeedInput resumption)
            // If priority_player is None, this is a fresh priority round - start with active player
            let mut current_priority = self.game.turn.priority_player.unwrap_or(active_player);
            let mut consecutive_passes = self.game.turn.consecutive_passes;
            let mut action_count = 0;
            const MAX_ACTIONS_PER_PRIORITY: usize = 1000;

            // Persist initial state
            self.game.turn.priority_player = Some(current_priority);

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

                // NETWORK SYNC PROTOCOL:
                // For network controllers (Remote, Network), we need to:
                // 1. Call prepare_for_priority_choice() to block on MVar and receive ChoiceRequest/OpponentChoice
                // 2. Then call sync_to_action() to process CardRevealed messages that are now guaranteed buffered
                // 3. Then compute abilities (now correct, includes newly drawn cards)
                //
                // This solves a race condition where sync_to_action() might run before the WS reader
                // has buffered the CardRevealed messages, causing abilities to be computed with stale data.
                let is_network_controlled = matches!(
                    controller.get_controller_type(),
                    ControllerType::Remote | ControllerType::Network
                );

                if is_network_controlled {
                    // For network controllers: prepare first (blocks until network data received)
                    if !controller.prepare_for_priority_choice() {
                        // Game ended or error - return ExitGame
                        return Ok(None);
                    }
                }

                // Sync network state now that we know reveals are buffered
                // For network controllers, the prepare call above guarantees ChoiceRequest/OpponentChoice
                // has been received, which means all preceding CardRevealed are buffered
                self.sync_to_action();

                // WASM RESUMPTION: Complete a pending typecycling library search.
                //
                // When the WASM game loop is interrupted (NeedInput) during the library
                // search phase of typecycling, `pending_cycling_search` is set. On the
                // next game loop invocation we bypass `choose_spell_ability_to_play` and
                // call `choose_from_library_with_hook` directly. Without this, the queued
                // LibrarySearchByName OpponentChoice would be mistakenly consumed by
                // `choose_spell_ability_to_play`, corrupting the RNG and causing a desync.
                if let Some((search_player, ref land_type)) = self.game.pending_cycling_search.clone() {
                    if search_player == current_priority {
                        let land_type = land_type.clone();
                        log::debug!(
                            "[WASM RESUME] Resuming cycling library search for player {:?}, land_type={}",
                            search_player,
                            land_type.as_str()
                        );
                        // Rebuild valid_cards with the same filter used in the cycling handler.
                        let filter = format!("Land.{}", land_type.as_str());
                        let library_cards = self
                            .game
                            .player_zones
                            .iter()
                            .find(|(id, _)| *id == search_player)
                            .map(|(_, zones)| zones.library.cards.clone())
                            .unwrap_or_default();
                        let valid_cards: Vec<CardId> = library_cards
                            .iter()
                            .copied()
                            .filter(|&card_id| {
                                self.game
                                    .cards
                                    .get(card_id)
                                    .map(|card| {
                                        crate::game::state::GameState::card_matches_search_filter(card, &filter)
                                    })
                                    .unwrap_or(false)
                            })
                            .collect();
                        // Capture log size BEFORE the library search choice (for log_choice_point).
                        let prior_log_size = self.game.logger.log_count();

                        // Ask controller to complete the library search.
                        let lib_choice = self.choose_from_library_with_hook(controller, search_player, &valid_cards);
                        let chosen_card_opt = handle_choice_result_break!(lib_choice, self.game, search_player);

                        // Library search succeeded — clear the pending state.
                        self.game.pending_cycling_search = None;

                        // Log this choice point for undo/replay (mirrors the cycling handler).
                        let chosen_index = chosen_card_opt.and_then(|c| valid_cards.iter().position(|&v| v == c));
                        let replay_choice = crate::game::ReplayChoice::LibrarySearch(chosen_index);
                        self.log_choice_point(search_player, Some(replay_choice), prior_log_size);

                        // Move chosen card from library to hand.
                        // The card may already be in hand (moved via CardRevealed); if so,
                        // move_card fails gracefully — the shadow game tolerates this.
                        if let Some(chosen_card) = chosen_card_opt {
                            let _ = self.game.move_card(
                                chosen_card,
                                crate::zones::Zone::Library,
                                crate::zones::Zone::Hand,
                                search_player,
                            );
                        }

                        // Shuffle library after searching (MTG CR 702.29).
                        self.game.shuffle_library(search_player);

                        // Cycling is now fully complete — switch priority to the other player.
                        consecutive_passes = 0;
                        self.game.turn.consecutive_passes = 0;
                        current_priority = if current_priority == active_player {
                            non_active_player
                        } else {
                            active_player
                        };
                        self.game.turn.priority_player = Some(current_priority);

                        continue; // Re-enter while loop with new current_priority.
                    }
                }

                // WASM RESUMPTION: Complete a pending spell cast.
                //
                // When the WASM game loop is interrupted (NeedInput) during mode selection
                // or target selection of a spell cast, `pending_cast` is set to the card_id.
                // On the next game loop invocation, we bypass `choose_spell_ability_to_play`
                // and resume the cast from where it was interrupted. Without this, the queued
                // mode or target ChoiceRequest would be mistakenly consumed by
                // `choose_spell_ability_to_play`, causing a desync.
                if let Some((cast_player, card_id)) = self.game.pending_cast {
                    if cast_player == current_priority {
                        log::debug!(
                            "[WASM RESUME] Resuming pending cast of {:?} for player {:?}",
                            card_id,
                            cast_player
                        );

                        // Step 1: Mode selection (only if ModalChoice effect still present).
                        // If the previous iteration already applied modes (via apply_selected_modes),
                        // get_modal_choice_info returns Ok(None) and we skip directly to targeting.
                        if let Ok(Some(Effect::ModalChoice {
                            modes,
                            num_to_choose,
                            min_to_choose,
                            can_repeat_modes,
                        })) = self.game.get_modal_choice_info(card_id)
                        {
                            let valid_modes = self
                                .game
                                .get_valid_modes_for_spell(card_id, cast_player)
                                .unwrap_or_default();
                            let valid_mode_indices: Vec<usize> = valid_modes
                                .iter()
                                .filter(|(_, has_targets)| *has_targets)
                                .map(|(idx, _)| *idx)
                                .collect();

                            if !valid_mode_indices.is_empty() {
                                let mode_descriptions: Vec<String> = valid_mode_indices
                                    .iter()
                                    .filter_map(|&idx| modes.get(idx).map(|m| m.description.clone()))
                                    .collect();

                                let prior_log_size = self.game.logger.log_count();
                                let choice = self.choose_modes_with_hook(
                                    controller,
                                    cast_player,
                                    card_id,
                                    &mode_descriptions,
                                    num_to_choose as usize,
                                    min_to_choose as usize,
                                    can_repeat_modes,
                                );
                                let selected_modes = handle_choice_result_break!(choice, self.game, cast_player);

                                let original_indices: Vec<usize> = selected_modes
                                    .iter()
                                    .filter_map(|&idx| valid_mode_indices.get(idx).copied())
                                    .collect();

                                let replay_choice =
                                    crate::game::ReplayChoice::Modes(original_indices.iter().copied().collect());
                                self.log_choice_point(cast_player, Some(replay_choice), prior_log_size);

                                if let Err(e) = self.game.apply_selected_modes(card_id, &original_indices) {
                                    log::warn!("[WASM RESUME] Failed to apply selected modes: {}", e);
                                }

                                if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                    for &mode_idx in &original_indices {
                                        if let Some(mode) = modes.get(mode_idx) {
                                            let message = format!(
                                                "{} chooses mode: {}",
                                                self.get_player_name(cast_player),
                                                mode.description
                                            );
                                            self.game.logger.gamelog(&message);
                                        }
                                    }
                                }
                            }
                        }

                        // Step 2: Target selection (runs after mode selection, or directly for non-modal spells).
                        let valid_targets = self
                            .game
                            .get_valid_targets_for_spell(card_id)
                            .unwrap_or_else(|_| SmallVec::new());

                        log::debug!(
                            target: "priority",
                            "[WASM RESUME] Target selection: card_id={}, valid_targets={:?}",
                            card_id.as_u32(),
                            valid_targets.iter().map(|c| c.as_u32()).collect::<Vec<_>>()
                        );

                        let chosen_targets_vec: SmallVec<[CardId; 2]> = if valid_targets.is_empty() {
                            log::debug!(target: "priority", "[WASM RESUME] No valid targets, using empty vec");
                            SmallVec::new()
                        } else if valid_targets.len() == 1 {
                            log::debug!(
                                target: "priority",
                                "[WASM RESUME] Auto-selecting single target: {:?}",
                                valid_targets[0].as_u32()
                            );
                            smallvec::smallvec![valid_targets[0]]
                        } else {
                            let prior_log_size = self.game.logger.log_count();
                            let choice =
                                self.choose_targets_with_hook(controller, cast_player, card_id, &valid_targets);
                            let chosen_targets = handle_choice_result_break!(choice, self.game, cast_player);
                            log::debug!(
                                target: "priority",
                                "[WASM RESUME] User chose targets: {:?}",
                                chosen_targets.iter().map(|c| c.as_u32()).collect::<Vec<_>>()
                            );
                            let replay_choice = crate::game::ReplayChoice::Targets(chosen_targets.clone());
                            self.log_choice_point(cast_player, Some(replay_choice), prior_log_size);
                            chosen_targets.into_iter().collect()
                        };

                        log::debug!(
                            target: "priority",
                            "[WASM RESUME] Final chosen_targets_vec: {:?}",
                            chosen_targets_vec.iter().map(|c| c.as_u32()).collect::<Vec<_>>()
                        );

                        // Step 3: Cast the spell — mana is paid automatically (GreedyManaResolver).
                        let targets_for_callback = chosen_targets_vec.clone();
                        let targeting_callback = move |_game: &GameState, _spell_id: CardId| targets_for_callback;

                        self.mana_engine.update_mut(self.game, cast_player);

                        match self
                            .game
                            .cast_spell_8_step(cast_player, card_id, targeting_callback, &self.mana_engine)
                        {
                            Ok(()) => {
                                self.game.spell_targets.push((card_id, chosen_targets_vec));
                                self.game.pending_cast = None;
                                consecutive_passes = 0;
                                self.game.turn.consecutive_passes = 0;
                                // Mirror the main-path priority switch at the end of Some(ability).
                                // Without this, the recovery block would keep current_priority on the
                                // caster, causing the WASM to skip the opponent's priority pass and
                                // creating a 1-action desync (log_choice_point missing for opponent).
                                current_priority = if current_priority == active_player {
                                    non_active_player
                                } else {
                                    active_player
                                };
                                self.game.turn.priority_player = Some(current_priority);
                                continue;
                            }
                            Err(e) => {
                                log::error!("[WASM RESUME] Failed to cast spell {:?}: {}", card_id, e);
                                self.game.pending_cast = None;
                                consecutive_passes += 1;
                                self.game.turn.consecutive_passes = consecutive_passes;
                                current_priority = if current_priority == active_player {
                                    non_active_player
                                } else {
                                    active_player
                                };
                                self.game.turn.priority_player = Some(current_priority);
                                continue;
                            }
                        }
                    }
                }

                // WASM RESUMPTION: Bypass spell ability selection when resuming pending activation.
                //
                // When the WASM game loop is interrupted (NeedInput) during target selection
                // of an activated ability, `pending_activation` is set. On the next
                // step_harness() call, we skip `choose_spell_ability_to_play` (which would
                // misroute the queued target ChoiceRequest) and resume directly in the
                // ActivateAbility arm where the target choice is completed.
                let choice = 'ability_choice: {
                    if let Some((act_player, act_card, act_idx)) = self.game.pending_activation {
                        if act_player == current_priority {
                            log::debug!(
                                "[WASM RESUME] Resuming pending activation of {:?} ability {} for player {:?}",
                                act_card,
                                act_idx,
                                act_player
                            );
                            // Sync reveals before resuming (mirrors sync_to_action() in normal loop)
                            self.sync_to_action();
                            break 'ability_choice Some(crate::core::SpellAbility::ActivateAbility {
                                card_id: act_card,
                                ability_index: act_idx,
                            });
                        }
                    }

                    // Loop to allow undo/retry for spell ability choices
                    loop {
                        // Get all available spell abilities for this player.
                        //
                        // OPTIMIZATION: get_available_spell_abilities now returns &[SpellAbility] from a
                        // reused internal buffer, eliminating repeated Vec allocations. We check emptiness
                        // first (no copy needed), then copy into SmallVec only when there are abilities
                        // (avoiding heap allocation for typical hand sizes up to 16 cards).
                        let available_count = self.get_available_spell_abilities(current_priority).len();

                        // Log abilities for debugging network sync issues
                        // OPTIMIZATION: Only format abilities when debug logging is enabled
                        // The format!("{:?}", a) calls were allocating ~2% of CPU time even when
                        // debug logging was disabled at runtime.
                        if log::log_enabled!(log::Level::Debug) {
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
                        }

                        // If no actions available, automatically pass priority without asking controller
                        // Only invoke controller when there's an actual choice to make
                        //
                        // EXCEPTION: Remote/Network controllers MUST always be asked:
                        // - Remote: Client-side opponent controller, we don't know hidden hand contents
                        // - Network: Server-side controller, must notify clients even on 0-ability pass
                        // The server will send the actual ability via OpponentChoice.
                        let ctrl_type = controller.get_controller_type();
                        let is_network_controlled =
                            matches!(ctrl_type, ControllerType::Remote | ControllerType::Network);
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

                        // Print choice menu BEFORE checking stop conditions so
                        // the available actions for the current choice point always
                        // appear in the text output.  External agents (agentplay)
                        // parse the LAST "available actions:" block to learn what
                        // options the choosing player has.
                        {
                            let view = GameStateView::new(self.game, current_priority);
                            if view.logger().should_show_choice_menu() && !available.is_empty() {
                                print!("{}", format_choice_menu(&view, &available));
                            }
                        } // Drop view before mutable borrow

                        // Check stop conditions AFTER printing the menu.
                        // Snapshots are still taken before the controller acts.
                        if let Some(result) = self.check_stop_conditions(controller, current_priority)? {
                            return Ok(Some(result));
                        }

                        // Ask controller to choose one (or None to pass)
                        // Capture log size BEFORE asking controller (before controller logs its choice)
                        let prior_log_size = self.game.logger.log_count();
                        // Use network-aware helper (creates view internally, handles pre-choice hook)
                        let choice_result =
                            self.choose_spell_ability_with_hook(controller, current_priority, &available);
                        let choice_value = handle_choice_result!(choice_result, self.game, current_priority);

                        // IMPORTANT: Sync network state after receiving opponent choice
                        // During wait_for_choice(), reveals may have arrived via WebSocket for cards
                        // that the opponent is about to play. We need to process those reveals NOW
                        // before the game tries to act on the choice (e.g., cast a spell from hand).
                        self.sync_to_action();

                        // Log this choice point for snapshot/replay
                        let replay_choice = crate::game::ReplayChoice::SpellAbility(choice_value.clone());
                        self.log_choice_point(current_priority, Some(replay_choice), prior_log_size);

                        break choice_value;
                    } // close inner loop
                }; // close 'ability_choice labeled block

                match choice {
                    None => {
                        // Controller chose to pass priority
                        consecutive_passes += 1;
                        self.game.turn.consecutive_passes = consecutive_passes;
                        let view = GameStateView::new(self.game, current_priority);
                        controller.on_priority_passed(&view);

                        // Switch priority to other player
                        current_priority = if current_priority == active_player {
                            non_active_player
                        } else {
                            active_player
                        };
                        self.game.turn.priority_player = Some(current_priority);
                    }
                    Some(ability) => {
                        // Controller chose an ability to play
                        consecutive_passes = 0; // Reset pass counter
                        self.game.turn.consecutive_passes = 0;

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
                                    self.game.turn.consecutive_passes = consecutive_passes;
                                    current_priority = if current_priority == active_player {
                                        non_active_player
                                    } else {
                                        active_player
                                    };
                                    self.game.turn.priority_player = Some(current_priority);
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

                                // Mark this cast as in-progress for WASM resumption.
                                // If mode selection or target selection below returns NeedInput,
                                // the next game loop invocation will resume via `pending_cast`.
                                self.game.pending_cast = Some((current_priority, card_id));

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
                                        let choice = self.choose_modes_with_hook(
                                            controller,
                                            current_priority,
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

                                // Step 2b: X value selection (MTG CR 601.2b)
                                // If the spell has X in its mana cost, ask controller to choose X
                                {
                                    let has_x = self
                                        .game
                                        .cards
                                        .get(card_id)
                                        .map(|c| c.mana_cost.has_x())
                                        .unwrap_or(false);
                                    if has_x {
                                        // Calculate maximum X the player could pay
                                        // max_x = (total untapped sources + pool - non-X cost) / x_count
                                        self.mana_engine.update_mut(self.game, current_priority);
                                        let untapped_sources =
                                            self.mana_engine
                                                .all_sources()
                                                .iter()
                                                .filter(|s| !s.is_tapped && !s.has_summoning_sickness)
                                                .count() as u8;
                                        let pool_mana = self
                                            .game
                                            .get_player(current_priority)
                                            .map(|p| p.total_available_mana().total())
                                            .unwrap_or(0);
                                        let max_mana = untapped_sources.saturating_add(pool_mana);
                                        let card = self.game.cards.get(card_id).unwrap();
                                        let colored_cost = card.mana_cost.cmc(); // colored + generic (excluding X)
                                        let x_count = card.mana_cost.x_count;
                                        let max_x = if x_count > 0 && max_mana > colored_cost {
                                            (max_mana - colored_cost) / x_count
                                        } else {
                                            0
                                        };

                                        let prior_log_size = self.game.logger.log_count();
                                        let view = GameStateView::new(self.game, current_priority);
                                        let x_choice = controller.choose_x_value(&view, card_id, max_x);
                                        let x_value =
                                            handle_choice_result_break!(x_choice, self.game, current_priority);

                                        // Clamp to max
                                        let x_value = x_value.min(max_x);

                                        // Store X paid on the card
                                        if let Ok(card) = self.game.cards.get_mut(card_id) {
                                            card.x_paid = x_value;
                                        }

                                        // Log X value choice
                                        let replay_choice = crate::game::ReplayChoice::XValue(x_value);
                                        self.log_choice_point(current_priority, Some(replay_choice), prior_log_size);

                                        if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                            let message = format!("  → X = {}", x_value);
                                            self.game.logger.gamelog(&message);
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
                                    let choice = self.choose_targets_with_hook(
                                        controller,
                                        current_priority,
                                        card_id,
                                        &valid_targets,
                                    );
                                    let chosen_targets =
                                        handle_choice_result_break!(choice, self.game, current_priority);

                                    // Log this choice point for snapshot/replay
                                    let replay_choice = crate::game::ReplayChoice::Targets(chosen_targets.clone());
                                    self.log_choice_point(current_priority, Some(replay_choice), prior_log_size);

                                    chosen_targets.into_iter().collect()
                                };

                                // Log target selection to gamelog (if any targets were chosen)
                                if !chosen_targets_vec.is_empty()
                                    && self.verbosity >= VerbosityLevel::Normal
                                    && !self.replaying
                                {
                                    // Get target names for display
                                    let target_names: Vec<String> = chosen_targets_vec
                                        .iter()
                                        .filter_map(|&tid| {
                                            self.game.cards.try_get(tid).map(|c| format!("{} ({})", c.name, tid))
                                        })
                                        .collect();
                                    if !target_names.is_empty() {
                                        let message = format!("  → targeting {}", target_names.join(", "));
                                        self.game.logger.gamelog(&message);
                                    }
                                }

                                // Clone SmallVec for closure (which will move it)
                                let targets_for_callback = chosen_targets_vec.clone();

                                // Create targeting callback (FnOnce — no clone needed)
                                let targeting_callback =
                                    move |_game: &GameState, _spell_id: CardId| targets_for_callback;

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
                                    self.game.turn.consecutive_passes = consecutive_passes;
                                    current_priority = if current_priority == active_player {
                                        non_active_player
                                    } else {
                                        active_player
                                    };
                                    self.game.turn.priority_player = Some(current_priority);
                                    continue;
                                } else {
                                    // Store targets for this spell (will be used when it resolves)
                                    self.game.spell_targets.push((card_id, chosen_targets_vec));

                                    // Cast fully committed — clear WASM resumption flag.
                                    self.game.pending_cast = None;

                                    // Spell is now on the stack - it will resolve later
                                    // when both players pass priority
                                }
                            }
                            crate::core::SpellAbility::ActivateAbility { card_id, ability_index } => {
                                // Activate ability from a permanent
                                // TODO(mtg-70): This should go on the stack for non-mana abilities

                                // Get the card and ability
                                let card_name = self.game.cards.try_get(card_id).map(|c| c.name.clone());
                                let ability = self
                                    .game
                                    .cards
                                    .get(card_id)
                                    .ok()
                                    .and_then(|c| c.activated_abilities.get(ability_index).cloned());

                                // On WASM resumption, pending_activation is already set.
                                // Don't log the "activates ability" message again on re-entry.
                                let is_activation_resumption = self.game.pending_activation.is_some();

                                // Check if we're resuming mid-effects (e.g., after NeedInput from
                                // DiscardCards routing). If so, skip target selection, cost payment,
                                // and already-executed effects. This prevents double-draws when
                                // abilities like Bazaar of Baghdad (draw 2, discard 3) have their
                                // DrawCards effects executed before DiscardCards returns NeedInput.
                                let effect_resume = self.game.pending_activation_effect_idx.take();

                                if let Some(ability) = ability {
                                    // Log "activates ability" only on first entry (not WASM resumption)
                                    if !is_activation_resumption
                                        && self.verbosity >= VerbosityLevel::Normal
                                        && !self.replaying
                                    {
                                        let name = card_name.as_ref().map(|n| n.as_str()).unwrap_or("Unknown");
                                        let desc = ability.description.replace("CARDNAME", name);
                                        let message = format!("{name} activates ability: {desc}");
                                        // Use gamelog for official game action
                                        self.game.logger.gamelog(&message);
                                    }

                                    // Guard against WASM re-entry at spell ability selection.
                                    // If target selection below returns NeedInput, the next
                                    // step_harness() call will see this flag and bypass
                                    // choose_spell_ability_to_play, resuming here instead.
                                    self.game.pending_activation = Some((current_priority, card_id, ability_index));

                                    // When resuming mid-effects, use saved targets and skip
                                    // target selection + cost payment (already done on first entry).
                                    let chosen_targets_vec: Vec<CardId>;
                                    let effect_start_idx: usize;

                                    if let Some((resume_idx, saved_targets)) = effect_resume {
                                        // WASM effect resumption: skip targets + costs, resume effects
                                        log::debug!(
                                            "[WASM EFFECT RESUME] Resuming ability effects from index {} (saved {} targets)",
                                            resume_idx,
                                            saved_targets.len()
                                        );
                                        chosen_targets_vec = saved_targets;
                                        effect_start_idx = resume_idx;
                                    } else {
                                        effect_start_idx = 0;
                                        // Get valid targets for the ability (before paying costs)
                                        let valid_targets = self
                                            .game
                                            .get_valid_targets_for_ability(card_id, ability_index)
                                            .unwrap_or_else(|_| SmallVec::new());

                                        // Ask controller to choose targets (only if there are valid targets)
                                        chosen_targets_vec = if valid_targets.is_empty() {
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
                                            let choice = self.choose_targets_with_hook(
                                                controller,
                                                current_priority,
                                                card_id,
                                                &valid_targets,
                                            );
                                            let chosen_targets =
                                                handle_choice_result_break!(choice, self.game, current_priority);

                                            // Log this choice point for snapshot/replay
                                            let replay_choice =
                                                crate::game::ReplayChoice::Targets(chosen_targets.clone());
                                            self.log_choice_point(
                                                current_priority,
                                                Some(replay_choice),
                                                prior_log_size,
                                            );

                                            chosen_targets.into_iter().collect()
                                        };

                                        // Log target selection to gamelog (if any targets were chosen)
                                        if !chosen_targets_vec.is_empty()
                                            && self.verbosity >= VerbosityLevel::Normal
                                            && !self.replaying
                                        {
                                            // Get target names for display
                                            let target_names: Vec<String> = chosen_targets_vec
                                                .iter()
                                                .filter_map(|&tid| {
                                                    self.game
                                                        .cards
                                                        .try_get(tid)
                                                        .map(|c| format!("{} ({})", c.name, tid))
                                                })
                                                .collect();
                                            if !target_names.is_empty() {
                                                let message = format!("  -> targeting {}", target_names.join(", "));
                                                self.game.logger.gamelog(&message);
                                            }
                                        }

                                        // Auto-tap lands for mana costs (if the ability has a mana cost)
                                        // This is the same logic as spell casting (step 6 of cast_spell_8_step)
                                        // SKIP when resuming mid-effects: costs were already paid on first entry.
                                        if effect_start_idx == 0 {
                                            if let Some(mana_cost) = ability.cost.get_mana_cost() {
                                                // Reuse self.mana_engine to avoid allocation on each activated ability
                                                use crate::game::mana_payment::{
                                                    GreedyManaResolver, ManaPaymentResolver, ManaSource,
                                                };

                                                self.mana_engine.update_mut(self.game, current_priority);

                                                // Get ManaSource list from engine (already built with proper production info)
                                                let all_sources = self.mana_engine.all_sources();

                                                // If the ability cost includes tapping this card, we can't use it for mana
                                                // because it will already be tapped as part of paying the activation cost.
                                                // Filter out the source card in this case.
                                                let filtered_sources: smallvec::SmallVec<[ManaSource; 8]>;
                                                let mana_sources: &[ManaSource] = if ability.cost.includes_tap() {
                                                    filtered_sources = all_sources
                                                        .iter()
                                                        .filter(|s| s.card_id != card_id)
                                                        .cloned()
                                                        .collect();
                                                    &filtered_sources
                                                } else {
                                                    all_sources
                                                };

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
                                                        match &card.definition.cache.mana_production.kind {
                                                            ManaProductionKind::Fixed(color) => match color {
                                                                ManaColor::White => {
                                                                    remaining_hint.white =
                                                                        remaining_hint.white.saturating_sub(1)
                                                                }
                                                                ManaColor::Blue => {
                                                                    remaining_hint.blue =
                                                                        remaining_hint.blue.saturating_sub(1)
                                                                }
                                                                ManaColor::Black => {
                                                                    remaining_hint.black =
                                                                        remaining_hint.black.saturating_sub(1)
                                                                }
                                                                ManaColor::Red => {
                                                                    remaining_hint.red =
                                                                        remaining_hint.red.saturating_sub(1)
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
                                                            ManaProductionKind::Choice(_)
                                                            | ManaProductionKind::AnyColor => {
                                                                // Deduct in same priority order as tap_for_mana_for_cost
                                                                if remaining_hint.white > 0 {
                                                                    remaining_hint.white =
                                                                        remaining_hint.white.saturating_sub(1);
                                                                } else if remaining_hint.blue > 0 {
                                                                    remaining_hint.blue =
                                                                        remaining_hint.blue.saturating_sub(1);
                                                                } else if remaining_hint.black > 0 {
                                                                    remaining_hint.black =
                                                                        remaining_hint.black.saturating_sub(1);
                                                                } else if remaining_hint.red > 0 {
                                                                    remaining_hint.red =
                                                                        remaining_hint.red.saturating_sub(1);
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
                                            if let Err(e) =
                                                self.game.pay_ability_cost(current_priority, card_id, &ability.cost)
                                            {
                                                if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                                    eprintln!("    Failed to pay cost: {e}");
                                                }
                                                log::warn!(
                                                    "pay_ability_cost failed for {:?} ability {} (player {:?}): {}",
                                                    card_id,
                                                    ability.description,
                                                    current_priority,
                                                    e
                                                );
                                                // Clear pending_activation — ability failed, no resumption needed
                                                self.game.pending_activation = None;
                                                self.game.pending_activation_effect_idx = None;
                                                // Treat failed ability activation like passing priority to prevent infinite loops
                                                consecutive_passes += 1;
                                                self.game.turn.consecutive_passes = consecutive_passes;
                                                current_priority = if current_priority == active_player {
                                                    non_active_player
                                                } else {
                                                    active_player
                                                };
                                                self.game.turn.priority_player = Some(current_priority);
                                                continue;
                                            }
                                        } // end if effect_start_idx == 0 (skip costs on effect resumption)
                                    } // end else (not resuming from effect_resume)

                                    // Execute effects immediately (not on the stack)
                                    // TODO(mtg-70): Put non-mana abilities on the stack
                                    // Use enumerate to track index for WASM effect resumption.
                                    // When resuming from a NeedInput mid-effects, skip effects
                                    // that were already executed on the first entry.
                                    for (effect_idx, effect) in ability.effects.iter().enumerate() {
                                        if effect_idx < effect_start_idx {
                                            continue; // Skip already-executed effects on resumption
                                        }
                                        // Fix placeholder player IDs and targets for effects
                                        let fixed_effect = match effect {
                                            crate::core::Effect::AddMana {
                                                player,
                                                mana,
                                                produces_chosen_color,
                                                amount_var,
                                            } if player.is_placeholder() => {
                                                // Replace placeholder with current player
                                                crate::core::Effect::AddMana {
                                                    player: current_priority,
                                                    mana: *mana,
                                                    produces_chosen_color: *produces_chosen_color,
                                                    amount_var: amount_var.clone(),
                                                }
                                            }
                                            crate::core::Effect::GainLife { player, amount }
                                                if player.is_placeholder() =>
                                            {
                                                // Replace placeholder with current player
                                                crate::core::Effect::GainLife {
                                                    player: current_priority,
                                                    amount: *amount,
                                                }
                                            }
                                            crate::core::Effect::DrawCards { player, count }
                                                if player.is_placeholder() =>
                                            {
                                                // Replace placeholder with current player
                                                crate::core::Effect::DrawCards {
                                                    player: current_priority,
                                                    count: *count,
                                                }
                                            }
                                            // Replace placeholder targets with chosen targets
                                            crate::core::Effect::DestroyPermanent { target, restriction }
                                                if target.is_placeholder() && !chosen_targets_vec.is_empty() =>
                                            {
                                                crate::core::Effect::DestroyPermanent {
                                                    target: chosen_targets_vec[0],
                                                    restriction: restriction.clone(),
                                                }
                                            }
                                            // Defined$ Self: destroy the source card itself
                                            crate::core::Effect::DestroyPermanent { target, restriction }
                                                if target.is_self_target() =>
                                            {
                                                crate::core::Effect::DestroyPermanent {
                                                    target: card_id,
                                                    restriction: restriction.clone(),
                                                }
                                            }
                                            crate::core::Effect::TapPermanent { target }
                                                if target.is_placeholder() && !chosen_targets_vec.is_empty() =>
                                            {
                                                crate::core::Effect::TapPermanent {
                                                    target: chosen_targets_vec[0],
                                                }
                                            }
                                            crate::core::Effect::UntapPermanent { target }
                                                if target.is_placeholder() && !chosen_targets_vec.is_empty() =>
                                            {
                                                crate::core::Effect::UntapPermanent {
                                                    target: chosen_targets_vec[0],
                                                }
                                            }
                                            crate::core::Effect::PumpCreature {
                                                target,
                                                power_bonus,
                                                toughness_bonus,
                                                keywords_granted,
                                            } if target.is_placeholder() && !chosen_targets_vec.is_empty() => {
                                                crate::core::Effect::PumpCreature {
                                                    target: chosen_targets_vec[0],
                                                    power_bonus: *power_bonus,
                                                    toughness_bonus: *toughness_bonus,
                                                    keywords_granted: keywords_granted.clone(),
                                                }
                                            }
                                            // Self-targeting pump: "This creature gets +X/+Y"
                                            // When no targets were chosen (ability doesn't target), use source card
                                            crate::core::Effect::PumpCreature {
                                                target,
                                                power_bonus,
                                                toughness_bonus,
                                                keywords_granted,
                                            } if target.is_placeholder() && chosen_targets_vec.is_empty() => {
                                                crate::core::Effect::PumpCreature {
                                                    target: card_id, // Target self (the source of the ability)
                                                    power_bonus: *power_bonus,
                                                    toughness_bonus: *toughness_bonus,
                                                    keywords_granted: keywords_granted.clone(),
                                                }
                                            }
                                            // Self-targeting PutCounter: "Put counter on this creature"
                                            // When Defined$ Self is used, no targets are chosen - use source card
                                            crate::core::Effect::PutCounter {
                                                target,
                                                counter_type,
                                                amount,
                                            } if target.is_placeholder() && chosen_targets_vec.is_empty() => {
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
                                            } if target.is_placeholder() && !chosen_targets_vec.is_empty() => {
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
                                            } if target.is_placeholder() && !chosen_targets_vec.is_empty() => {
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
                                                keywords_granted,
                                            } if target.is_placeholder() && chosen_targets_vec.is_empty() => {
                                                crate::core::Effect::SetBasePowerToughness {
                                                    target: card_id, // Target self (the source of the ability)
                                                    power: *power,
                                                    toughness: *toughness,
                                                    keywords_granted: keywords_granted.clone(),
                                                }
                                            }
                                            // Targeted SetBasePowerToughness: "Target creature has base P/T X/Y"
                                            crate::core::Effect::SetBasePowerToughness {
                                                target,
                                                power,
                                                toughness,
                                                keywords_granted,
                                            } if target.is_placeholder() && !chosen_targets_vec.is_empty() => {
                                                crate::core::Effect::SetBasePowerToughness {
                                                    target: chosen_targets_vec[0],
                                                    power: *power,
                                                    toughness: *toughness,
                                                    keywords_granted: keywords_granted.clone(),
                                                }
                                            }
                                            crate::core::Effect::AttachEquipment {
                                                source_equipment,
                                                target_creature,
                                            } if target_creature.is_placeholder() && !chosen_targets_vec.is_empty() => {
                                                crate::core::Effect::AttachEquipment {
                                                    source_equipment: *source_equipment,
                                                    target_creature: chosen_targets_vec[0],
                                                }
                                            }
                                            // GrantCantBeBlocked: "Target creature can't be blocked this turn"
                                            // Used by Deserter's Disciple
                                            crate::core::Effect::GrantCantBeBlocked { target }
                                                if target.is_placeholder() && !chosen_targets_vec.is_empty() =>
                                            {
                                                crate::core::Effect::GrantCantBeBlocked {
                                                    target: chosen_targets_vec[0],
                                                }
                                            }
                                            // Self-targeting Regenerate: "Regenerate CARDNAME"
                                            crate::core::Effect::Regenerate { target }
                                                if target.is_placeholder() && chosen_targets_vec.is_empty() =>
                                            {
                                                crate::core::Effect::Regenerate {
                                                    target: card_id, // Target self
                                                }
                                            }
                                            // Targeted Regenerate: "Regenerate target creature"
                                            crate::core::Effect::Regenerate { target }
                                                if target.is_placeholder() && !chosen_targets_vec.is_empty() =>
                                            {
                                                crate::core::Effect::Regenerate {
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
                                            } if player.is_placeholder() => {
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

                                                // Sync network state to process pending CardRevealed messages
                                                // before filtering library cards (mtg-ondgo fix)
                                                self.sync_to_action();

                                                // Filter cards by type
                                                let mut valid_cards = Vec::new();
                                                for &card_id in &library_cards {
                                                    if let Some(card) = self.game.cards.try_get(card_id) {
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
                                                let choice = self.choose_from_library_with_hook(
                                                    controller,
                                                    current_priority,
                                                    &valid_cards,
                                                );
                                                // Handle NeedInput: save effect index for resumption
                                                let chosen_card_opt = match choice {
                                                    crate::game::controller::ChoiceResult::NeedInput(ctx) => {
                                                        self.game.pending_activation_effect_idx =
                                                            Some((effect_idx, chosen_targets_vec));
                                                        return Err(crate::MtgError::NeedInput(Box::new(ctx)));
                                                    }
                                                    other => {
                                                        handle_choice_result_break!(other, self.game, current_priority)
                                                    }
                                                };

                                                // Log the choice for replay - convert CardId to index
                                                let chosen_index = chosen_card_opt
                                                    .and_then(|card_id| valid_cards.iter().position(|&c| c == card_id));
                                                let replay_choice =
                                                    crate::game::ReplayChoice::LibrarySearch(chosen_index);
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
                                            // Earthbend: Target land becomes 0/0 creature with haste and N +1/+1 counters
                                            crate::core::Effect::Earthbend { target, num_counters }
                                                if target.is_placeholder() && !chosen_targets_vec.is_empty() =>
                                            {
                                                crate::core::Effect::Earthbend {
                                                    target: chosen_targets_vec[0],
                                                    num_counters: *num_counters,
                                                }
                                            }
                                            // DiscardCards with choice (count != u8::MAX): Route through
                                            // controller for network-safe discard decisions (mtg-xomxx).
                                            //
                                            // Without this, choose_card_to_discard() runs independently
                                            // on both server and shadow. The shadow can't see cards drawn
                                            // in the same ability (e.g., Bazaar of Baghdad: draw 2, discard 3)
                                            // because CardRevealed messages haven't arrived yet.
                                            crate::core::Effect::DiscardCards {
                                                player,
                                                count,
                                                remember_discarded,
                                                ..
                                            } if *count != u8::MAX => {
                                                let discard_player = if player.is_placeholder() {
                                                    current_priority
                                                } else {
                                                    *player
                                                };
                                                let discard_count = *count as usize;
                                                let remember = *remember_discarded;

                                                // Sync reveals so shadow sees newly drawn cards
                                                self.push_reveals(discard_player);
                                                if let Some(opp) = self.game.get_other_player_id(discard_player) {
                                                    self.push_reveals(opp);
                                                }
                                                self.sync_to_action();

                                                // Get current hand
                                                let hand: SmallVec<[CardId; 8]> = self
                                                    .game
                                                    .get_player_zones(discard_player)
                                                    .map(|zones| zones.hand.cards.iter().copied().collect())
                                                    .unwrap_or_default();

                                                // Clamp count to actual hand size
                                                let actual_count = discard_count.min(hand.len());
                                                if actual_count == 0 {
                                                    continue;
                                                }

                                                // Route through controller protocol
                                                let choice = self.choose_discard_with_hook(
                                                    controller,
                                                    discard_player,
                                                    &hand,
                                                    actual_count,
                                                );
                                                // Handle NeedInput specially: save effect index for
                                                // WASM resumption so we don't re-execute prior effects
                                                // (especially DrawCards) on re-entry.
                                                let cards_to_discard = match choice {
                                                    crate::game::controller::ChoiceResult::NeedInput(ctx) => {
                                                        // Save effect index and targets for resumption
                                                        self.game.pending_activation_effect_idx =
                                                            Some((effect_idx, chosen_targets_vec));
                                                        return Err(crate::MtgError::NeedInput(Box::new(ctx)));
                                                    }
                                                    other => {
                                                        handle_choice_result_break!(other, self.game, current_priority)
                                                    }
                                                };

                                                for card_id in cards_to_discard {
                                                    if remember {
                                                        self.game.remembered_cards.push(card_id);
                                                    }
                                                    if let Err(e) = self.game.discard_card(discard_player, card_id) {
                                                        if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                                            eprintln!("    Failed to discard: {e}");
                                                        }
                                                    }
                                                }

                                                // Skip execute_effect — handled above
                                                continue;
                                            }
                                            // DiscardCards with placeholder player (full hand, u8::MAX):
                                            // Just fix the placeholder, let execute_effect handle it
                                            // (no choice needed — discards everything deterministically)
                                            crate::core::Effect::DiscardCards {
                                                player,
                                                count,
                                                remember_discarded,
                                                ..
                                            } if player.is_placeholder() => crate::core::Effect::DiscardCards {
                                                player: current_priority,
                                                count: *count,
                                                remember_discarded: *remember_discarded,
                                                optional: false,
                                                remember_discarding_players: false,
                                            },
                                            // Loot (discard then draw): Route discard through controller
                                            // for network-safe decisions (mtg-xomxx).
                                            crate::core::Effect::Loot {
                                                player,
                                                discard_count,
                                                draw_count,
                                            } => {
                                                let loot_player = if player.is_placeholder() {
                                                    current_priority
                                                } else {
                                                    *player
                                                };
                                                let d_count = *discard_count as usize;
                                                let dr_count = *draw_count;

                                                // Sync reveals before looting
                                                self.sync_to_action();

                                                // Discard phase: route through controller
                                                if d_count > 0 {
                                                    let hand: SmallVec<[CardId; 8]> = self
                                                        .game
                                                        .get_player_zones(loot_player)
                                                        .map(|zones| zones.hand.cards.iter().copied().collect())
                                                        .unwrap_or_default();

                                                    let actual_count = d_count.min(hand.len());
                                                    if actual_count > 0 {
                                                        let choice = self.choose_discard_with_hook(
                                                            controller,
                                                            loot_player,
                                                            &hand,
                                                            actual_count,
                                                        );
                                                        // Handle NeedInput: save effect index for resumption
                                                        let cards_to_discard = match choice {
                                                            crate::game::controller::ChoiceResult::NeedInput(ctx) => {
                                                                self.game.pending_activation_effect_idx =
                                                                    Some((effect_idx, chosen_targets_vec));
                                                                return Err(crate::MtgError::NeedInput(Box::new(ctx)));
                                                            }
                                                            other => {
                                                                handle_choice_result_break!(
                                                                    other,
                                                                    self.game,
                                                                    current_priority
                                                                )
                                                            }
                                                        };
                                                        for card_id in cards_to_discard {
                                                            if let Err(e) = self.game.discard_card(loot_player, card_id)
                                                            {
                                                                if self.verbosity >= VerbosityLevel::Normal
                                                                    && !self.replaying
                                                                {
                                                                    eprintln!("    Failed to discard: {e}");
                                                                }
                                                            }
                                                        }
                                                    }
                                                }

                                                // Draw phase: execute directly
                                                for _ in 0..dr_count {
                                                    match self.game.draw_card(loot_player) {
                                                        Ok((_, draw_num)) => {
                                                            if let Err(e) = self
                                                                .game
                                                                .check_card_drawn_triggers(loot_player, draw_num)
                                                            {
                                                                if self.verbosity >= VerbosityLevel::Normal
                                                                    && !self.replaying
                                                                {
                                                                    eprintln!("    Failed draw trigger: {e}");
                                                                }
                                                            }
                                                        }
                                                        Err(e) => {
                                                            if self.verbosity >= VerbosityLevel::Normal
                                                                && !self.replaying
                                                            {
                                                                eprintln!("    Failed to draw: {e}");
                                                            }
                                                            break;
                                                        }
                                                    }
                                                }

                                                // Skip execute_effect — handled above
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

                                    // Mark exhaust ability as exhausted (can only be activated once per game)
                                    if ability.exhaust {
                                        if let Ok(card_mut) = self.game.cards.get_mut(card_id) {
                                            if !card_mut.exhausted_abilities.contains(&ability_index) {
                                                card_mut.exhausted_abilities.push(ability_index);
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
                                    // Check legendary rule (MTG CR 704.5j)
                                    if let Err(e) = self.game.check_legendary_rule() {
                                        if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                            eprintln!("    Failed to check legendary rule: {e}");
                                        }
                                    }
                                    // Check aura attachment (MTG CR 704.5d)
                                    if let Err(e) = self.game.check_aura_attachment() {
                                        if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                            eprintln!("    Failed to check aura attachment: {e}");
                                        }
                                    }
                                    // Check equipment attachment (MTG CR 704.5n)
                                    if let Err(e) = self.game.check_equipment_attachment() {
                                        if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                            eprintln!("    Failed to check equipment attachment: {e}");
                                        }
                                    }

                                    // Clear pending_activation — ability executed successfully
                                    self.game.pending_activation = None;
                                    self.game.pending_activation_effect_idx = None;
                                } else {
                                    log::warn!(
                                        "ActivateAbility: ability not found for card {:?} '{}' ability_index={} (player {:?})",
                                        card_id, card_name.as_ref().map(|n| n.as_str()).unwrap_or("MISSING"), ability_index, current_priority
                                    );
                                    if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                        eprintln!("  Ability not found");
                                    }
                                    // Treat as pass to avoid infinite loop
                                    consecutive_passes += 1;
                                    self.game.turn.consecutive_passes = consecutive_passes;
                                    current_priority = if current_priority == active_player {
                                        non_active_player
                                    } else {
                                        active_player
                                    };
                                    self.game.turn.priority_player = Some(current_priority);
                                    continue;
                                }
                            }
                            crate::core::SpellAbility::CastFromExile {
                                card_id,
                                alternative_cost,
                                effect_id,
                            } => {
                                // Cast from exile using alternative cost (Airbend, Suspend, etc.)
                                // Uses the generalized 8-step casting process from exile zone.

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

                                // Mode selection for modal spells cast from exile (Charm spells)
                                // MTG Rule 601.2b: Modal choice happens BEFORE targeting/payment
                                if let Ok(Some(crate::core::Effect::ModalChoice {
                                    modes,
                                    num_to_choose,
                                    min_to_choose,
                                    can_repeat_modes,
                                })) = self.game.get_modal_choice_info(card_id)
                                {
                                    let valid_modes = self
                                        .game
                                        .get_valid_modes_for_spell(card_id, current_priority)
                                        .unwrap_or_default();
                                    let valid_mode_indices: Vec<usize> = valid_modes
                                        .iter()
                                        .filter(|(_, has_targets)| *has_targets)
                                        .map(|(idx, _)| *idx)
                                        .collect();

                                    if !valid_mode_indices.is_empty() {
                                        let mode_descriptions: Vec<String> = valid_mode_indices
                                            .iter()
                                            .filter_map(|&idx| modes.get(idx).map(|m| m.description.clone()))
                                            .collect();

                                        let prior_log_size = self.game.logger.log_count();
                                        let choice = self.choose_modes_with_hook(
                                            controller,
                                            current_priority,
                                            card_id,
                                            &mode_descriptions,
                                            num_to_choose as usize,
                                            min_to_choose as usize,
                                            can_repeat_modes,
                                        );
                                        let selected_modes =
                                            handle_choice_result_break!(choice, self.game, current_priority);

                                        let original_indices: Vec<usize> = selected_modes
                                            .iter()
                                            .filter_map(|&idx| valid_mode_indices.get(idx).copied())
                                            .collect();

                                        let replay_choice = crate::game::ReplayChoice::Modes(
                                            original_indices.iter().copied().collect(),
                                        );
                                        self.log_choice_point(current_priority, Some(replay_choice), prior_log_size);

                                        if let Err(e) = self.game.apply_selected_modes(card_id, &original_indices) {
                                            log::warn!(target: "priority", "Failed to apply modes: {}", e);
                                        }

                                        if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                            for &mode_idx in &original_indices {
                                                if let Some(mode) = modes.get(mode_idx) {
                                                    self.game.logger.gamelog(&format!(
                                                        "{} chooses mode: {}",
                                                        self.get_player_name(current_priority),
                                                        mode.description
                                                    ));
                                                }
                                            }
                                        }
                                    }
                                }

                                // Target selection (same as CastSpell path)
                                let valid_targets = self
                                    .game
                                    .get_valid_targets_for_spell(card_id)
                                    .unwrap_or_else(|_| SmallVec::new());

                                let chosen_targets_vec: SmallVec<[CardId; 2]> = if valid_targets.is_empty() {
                                    SmallVec::new()
                                } else if valid_targets.len() == 1 {
                                    smallvec::smallvec![valid_targets[0]]
                                } else {
                                    let prior_log_size = self.game.logger.log_count();
                                    let choice = self.choose_targets_with_hook(
                                        controller,
                                        current_priority,
                                        card_id,
                                        &valid_targets,
                                    );
                                    let chosen_targets =
                                        handle_choice_result_break!(choice, self.game, current_priority);
                                    let replay_choice = crate::game::ReplayChoice::Targets(chosen_targets.clone());
                                    self.log_choice_point(current_priority, Some(replay_choice), prior_log_size);
                                    chosen_targets.into_iter().collect()
                                };

                                let targets_for_callback = chosen_targets_vec.clone();
                                let targeting_callback =
                                    move |_game: &GameState, _spell_id: CardId| targets_for_callback;

                                // Cast using generalized 8-step process from exile with alternative cost
                                self.mana_engine.update_mut(self.game, current_priority);

                                if let Err(e) = self.game.cast_spell_8_step_from(
                                    current_priority,
                                    card_id,
                                    targeting_callback,
                                    &self.mana_engine,
                                    crate::zones::Zone::Exile,
                                    Some(alternative_cost),
                                ) {
                                    if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                        let message = format!("Error casting from exile: {e}");
                                        self.game.logger.normal(&message);
                                    }
                                    consecutive_passes += 1;
                                    self.game.turn.consecutive_passes = consecutive_passes;
                                    current_priority = if current_priority == active_player {
                                        non_active_player
                                    } else {
                                        active_player
                                    };
                                    self.game.turn.priority_player = Some(current_priority);
                                    continue;
                                }

                                // Store targets for resolution
                                if !chosen_targets_vec.is_empty() {
                                    self.game.spell_targets.push((card_id, chosen_targets_vec));
                                }

                                // Remove the persistent effect that granted this cast permission
                                self.game.persistent_effects.remove(effect_id);

                                // Spell is now on the stack - will resolve when both players pass
                            }

                            crate::core::SpellAbility::CastFromCommand { card_id, total_cost } => {
                                // Cast commander from command zone (MTG CR 903.8)
                                // Uses the generalized 8-step casting process from command zone.

                                let card_name = self
                                    .game
                                    .cards
                                    .get(card_id)
                                    .map(|c| c.name.to_string())
                                    .unwrap_or_else(|_| "Unknown".to_string());

                                if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                    let player_idx = self
                                        .game
                                        .players
                                        .iter()
                                        .position(|p| p.id == current_priority)
                                        .unwrap_or(0);
                                    let tax = self.game.players[player_idx].commander_tax();
                                    let message = if tax > 0 {
                                        format!(
                                            "{} casts {} from command zone for {} (includes {{{}}} commander tax)",
                                            self.get_player_name(current_priority),
                                            card_name,
                                            total_cost,
                                            tax
                                        )
                                    } else {
                                        format!(
                                            "{} casts {} from command zone for {}",
                                            self.get_player_name(current_priority),
                                            card_name,
                                            total_cost
                                        )
                                    };
                                    self.game.logger.gamelog(&message);
                                }

                                // Target selection (same as CastSpell path)
                                let valid_targets = self
                                    .game
                                    .get_valid_targets_for_spell(card_id)
                                    .unwrap_or_else(|_| SmallVec::new());

                                let chosen_targets_vec: SmallVec<[CardId; 2]> = if valid_targets.is_empty() {
                                    SmallVec::new()
                                } else if valid_targets.len() == 1 {
                                    smallvec::smallvec![valid_targets[0]]
                                } else {
                                    let prior_log_size = self.game.logger.log_count();
                                    let choice = self.choose_targets_with_hook(
                                        controller,
                                        current_priority,
                                        card_id,
                                        &valid_targets,
                                    );
                                    let chosen_targets =
                                        handle_choice_result_break!(choice, self.game, current_priority);
                                    let replay_choice = crate::game::ReplayChoice::Targets(chosen_targets.clone());
                                    self.log_choice_point(current_priority, Some(replay_choice), prior_log_size);
                                    chosen_targets.into_iter().collect()
                                };

                                let targets_for_callback = chosen_targets_vec.clone();
                                let targeting_callback =
                                    move |_game: &GameState, _spell_id: CardId| targets_for_callback;

                                // Cast using generalized 8-step process from command zone
                                // total_cost already includes commander tax (computed at ability generation time)
                                self.mana_engine.update_mut(self.game, current_priority);

                                if let Err(e) = self.game.cast_spell_8_step_from(
                                    current_priority,
                                    card_id,
                                    targeting_callback,
                                    &self.mana_engine,
                                    crate::zones::Zone::Command,
                                    Some(total_cost),
                                ) {
                                    if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                        let message = format!("Error casting from command zone: {e}");
                                        self.game.logger.normal(&message);
                                    }
                                    consecutive_passes += 1;
                                    self.game.turn.consecutive_passes = consecutive_passes;
                                    current_priority = if current_priority == active_player {
                                        non_active_player
                                    } else {
                                        active_player
                                    };
                                    self.game.turn.priority_player = Some(current_priority);
                                    continue;
                                }

                                // Store targets for resolution
                                if !chosen_targets_vec.is_empty() {
                                    self.game.spell_targets.push((card_id, chosen_targets_vec));
                                }

                                // Record commander cast for commander tax tracking
                                if let Some(player) = self.game.players.iter_mut().find(|p| p.id == current_priority) {
                                    let old_count = player.commander_cast_count;
                                    player.record_commander_cast();
                                    let prior_log_size = self.game.logger.log_count();
                                    self.game.undo_log.log(
                                        crate::undo::GameAction::SetCommanderCastCount {
                                            player_id: current_priority,
                                            old_value: old_count,
                                            new_value: player.commander_cast_count,
                                        },
                                        prior_log_size,
                                    );
                                }

                                // Spell is now on the stack - will resolve when both players pass
                            }

                            crate::core::SpellAbility::Cycle {
                                card_id,
                                cost,
                                search_type,
                            } => {
                                // Cycling ability from hand
                                // MTG CR 702.29: "Cycling is an activated ability that functions only
                                // while the card with cycling is in a player's hand."

                                let card_name = self
                                    .game
                                    .cards
                                    .get(card_id)
                                    .map(|c| c.name.to_string())
                                    .unwrap_or_else(|_| "Unknown".to_string());

                                let type_str = match &search_type {
                                    Some(st) => format!("{}cycling", st.as_str()),
                                    None => "Cycling".to_string(),
                                };

                                if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                    let message = format!(
                                        "{} uses {} on {} (cost: {})",
                                        self.get_player_name(current_priority),
                                        type_str,
                                        card_name,
                                        cost
                                    );
                                    self.game.logger.gamelog(&message);
                                }

                                // 1. Pay the cycling cost
                                self.mana_engine.update_mut(self.game, current_priority);
                                use crate::game::mana_payment::{GreedyManaResolver, ManaPaymentResolver};

                                let mana_sources = self.mana_engine.all_sources();
                                let resolver = GreedyManaResolver::new();
                                self.sources_to_tap_buffer.clear();
                                resolver.compute_tap_order(&cost, mana_sources, &mut self.sources_to_tap_buffer);

                                let mut remaining_hint = cost;
                                for &source_id in &self.sources_to_tap_buffer {
                                    if let Err(e) =
                                        self.game
                                            .tap_for_mana_for_cost(current_priority, source_id, &remaining_hint)
                                    {
                                        if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                            self.game.logger.normal(&format!("Failed to tap for cycling: {e}"));
                                        }
                                    }
                                    remaining_hint.generic = remaining_hint.generic.saturating_sub(1);
                                }

                                // 2. Discard the card (move from hand to graveyard)
                                let owner = self
                                    .game
                                    .cards
                                    .get(card_id)
                                    .map(|c| c.owner)
                                    .unwrap_or(current_priority);

                                if let Err(e) = self.game.move_card(
                                    card_id,
                                    crate::zones::Zone::Hand,
                                    crate::zones::Zone::Graveyard,
                                    owner,
                                ) {
                                    if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                        self.game.logger.normal(&format!("Failed to discard for cycling: {e}"));
                                    }
                                }

                                // 3. Perform the cycling effect
                                match search_type {
                                    Some(land_type) => {
                                        // Typecycling: Search library for card with matching type
                                        // For Mountaincycling, search for Mountain
                                        // For Swampcycling, search for Swamp
                                        log::debug!(
                                            "[TYPECYCLING] Entered typecycling branch for land_type={:?}",
                                            land_type
                                        );

                                        // Build search filter for matching land type
                                        let filter = format!("Land.{}", land_type.as_str());
                                        log::debug!("[TYPECYCLING] Filter = '{}'", filter);

                                        // Get library and filter for matching cards
                                        let library_cards = self
                                            .game
                                            .player_zones
                                            .iter()
                                            .find(|(id, _)| *id == current_priority)
                                            .map(|(_, zones)| zones.library.cards.clone())
                                            .unwrap_or_default();
                                        log::debug!("[TYPECYCLING] Library has {} cards", library_cards.len());

                                        // Sync network state to process pending CardRevealed messages
                                        // before filtering library cards (mtg-ondgo fix)
                                        self.sync_to_action();

                                        // Filter cards by type
                                        let mut valid_cards = Vec::new();
                                        for &lib_card_id in &library_cards {
                                            if let Some(card) = self.game.cards.try_get(lib_card_id) {
                                                if crate::game::state::GameState::card_matches_search_filter(
                                                    card, &filter,
                                                ) {
                                                    valid_cards.push(lib_card_id);
                                                }
                                            }
                                        }
                                        log::debug!(
                                            "[TYPECYCLING] Found {} valid cards matching filter",
                                            valid_cards.len()
                                        );

                                        // Ask controller to choose a card (or decline to find).
                                        // Set pending_cycling_search BEFORE calling, so that if
                                        // NeedInput is returned and the WASM game loop restarts,
                                        // priority_round() will resume the search directly instead
                                        // of routing the LibrarySearchByName OpponentChoice through
                                        // choose_spell_ability_to_play (which would misroute it).
                                        self.game.pending_cycling_search = Some((current_priority, land_type.clone()));

                                        let prior_log_size = self.game.logger.log_count();
                                        log::debug!("[TYPECYCLING] About to call choose_from_library_with_hook");
                                        let choice = self.choose_from_library_with_hook(
                                            controller,
                                            current_priority,
                                            &valid_cards,
                                        );
                                        log::debug!(
                                            "[TYPECYCLING] choose_from_library_with_hook returned: {:?}",
                                            choice
                                        );
                                        let chosen_card_opt =
                                            handle_choice_result_break!(choice, self.game, current_priority);

                                        // Library search succeeded — clear the pending state.
                                        self.game.pending_cycling_search = None;

                                        // Log the choice for replay - convert CardId to index
                                        let chosen_index = chosen_card_opt
                                            .and_then(|card_id| valid_cards.iter().position(|&c| c == card_id));
                                        let replay_choice = crate::game::ReplayChoice::LibrarySearch(chosen_index);
                                        self.log_choice_point(current_priority, Some(replay_choice), prior_log_size);

                                        // If a card was chosen, move it to hand
                                        if let Some(chosen_card) = chosen_card_opt {
                                            if let Err(e) = self.game.move_card(
                                                chosen_card,
                                                crate::zones::Zone::Library,
                                                crate::zones::Zone::Hand,
                                                current_priority,
                                            ) {
                                                if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                                    self.game
                                                        .logger
                                                        .normal(&format!("Failed to move card to hand: {e}"));
                                                }
                                            }
                                        }

                                        // Shuffle library after searching (MTG CR 702.29)
                                        self.game.shuffle_library(current_priority);
                                    }
                                    None => {
                                        // Regular cycling: Draw a card
                                        match self.game.draw_card(current_priority) {
                                            Ok((_, draw_count)) => {
                                                // Check for "second card drawn" triggers
                                                let _ =
                                                    self.game.check_card_drawn_triggers(current_priority, draw_count);
                                            }
                                            Err(e) => {
                                                if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                                    self.game
                                                        .logger
                                                        .normal(&format!("Failed to draw from cycling: {e}"));
                                                }
                                            }
                                        }
                                    }
                                }

                                // Cycling is complete (doesn't use the stack)
                            }

                            crate::core::SpellAbility::CastFromGraveyard {
                                card_id,
                                effect_id: _,
                                add_finality_counter,
                            } => {
                                // Cast creature from graveyard (Leonardo, Sewer Samurai)
                                let card_name = self
                                    .game
                                    .cards
                                    .get(card_id)
                                    .map(|c| c.name.to_string())
                                    .unwrap_or_else(|_| "Unknown".to_string());

                                if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                    let message = format!(
                                        "{} casts {} from graveyard (with finality counter)",
                                        self.get_player_name(current_priority),
                                        card_name,
                                    );
                                    self.game.logger.gamelog(&message);
                                }

                                // Move card from graveyard to stack
                                let owner = self
                                    .game
                                    .cards
                                    .get(card_id)
                                    .map(|c| c.owner)
                                    .unwrap_or(current_priority);

                                if let Err(e) = self.game.move_card(
                                    card_id,
                                    crate::zones::Zone::Graveyard,
                                    crate::zones::Zone::Stack,
                                    owner,
                                ) {
                                    if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                        self.game.logger.normal(&format!("Error moving from graveyard: {e}"));
                                    }
                                    consecutive_passes += 1;
                                    self.game.turn.consecutive_passes = consecutive_passes;
                                    current_priority = if current_priority == active_player {
                                        non_active_player
                                    } else {
                                        active_player
                                    };
                                    self.game.turn.priority_player = Some(current_priority);
                                    continue;
                                }

                                // Pay the card's mana cost normally
                                let mana_cost = self.game.cards.get(card_id).map(|c| c.mana_cost).unwrap_or_default();

                                self.mana_engine.update_mut(self.game, current_priority);
                                use crate::game::mana_payment::{GreedyManaResolver, ManaPaymentResolver};

                                let mana_sources = self.mana_engine.all_sources();
                                self.sources_to_tap_buffer.clear();
                                let resolver = GreedyManaResolver::new();
                                resolver.compute_tap_order(&mana_cost, mana_sources, &mut self.sources_to_tap_buffer);

                                let mut remaining_hint = mana_cost;
                                for &source_id in &self.sources_to_tap_buffer {
                                    if let Err(e) =
                                        self.game
                                            .tap_for_mana_for_cost(current_priority, source_id, &remaining_hint)
                                    {
                                        if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                            self.game.logger.normal(&format!("Failed to tap: {e}"));
                                        }
                                    }
                                    remaining_hint.generic = remaining_hint.generic.saturating_sub(1);
                                }

                                // If add_finality_counter, mark the card so it gets a finality counter on ETB
                                if add_finality_counter {
                                    if let Ok(card) = self.game.cards.get_mut(card_id) {
                                        card.add_counter(crate::core::CounterType::Finality, 1);
                                        log::debug!(
                                            "Added finality counter to {} ({}) cast from graveyard",
                                            card_name,
                                            card_id
                                        );
                                    }
                                }

                                // Spell is now on the stack - will resolve when both players pass
                            }
                        }

                        // After taking an action, switch priority to other player
                        current_priority = if current_priority == active_player {
                            non_active_player
                        } else {
                            active_player
                        };
                        self.game.turn.priority_player = Some(current_priority);
                    }
                }
            }

            // Both players passed priority - reset priority state for the next round
            // (either after stack resolution or for next phase)
            self.game.turn.priority_player = None;
            self.game.turn.consecutive_passes = 0;

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
    /// like Balance and DiscardCards that require player choices routed through
    /// the controller protocol (mtg-xomxx network desync fix).
    fn resolve_top_spell_from_stack_interactive(
        &mut self,
        spell_id: CardId,
        controller1: &mut dyn PlayerController,
        controller2: &mut dyn PlayerController,
    ) -> Result<()> {
        // Check if this spell has any Balance or choice-based discard effects before resolving
        // Also capture SVars for SubAbility resolution
        let (balance_effects, has_choice_discard, svars): (Vec<_>, bool, std::collections::HashMap<String, String>) =
            if let Some(card) = self.game.cards.try_get(spell_id) {
                let balances = card
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
                let has_discard = card.effects.iter().any(|e| {
                    matches!(
                        e,
                        crate::core::Effect::DiscardCards { count, .. } if *count != u8::MAX
                    ) || matches!(e, crate::core::Effect::Loot { .. })
                });
                (balances, has_discard, card.svars.clone())
            } else {
                (Vec::new(), false, std::collections::HashMap::new())
            };

        if has_choice_discard {
            // Use effect-by-effect resolution so we can intercept discard choices
            // and route them through the controller protocol (mtg-xomxx).
            self.resolve_top_spell_with_discard_hook(spell_id, controller1, controller2)?;
        } else {
            // Resolve the spell normally (Balance effects are no-ops in execute_effect)
            self.resolve_top_spell_from_stack(spell_id)?;
        }

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

    /// Resolve a spell effect-by-effect, intercepting discard choices through
    /// the controller protocol for network-safe operation (mtg-xomxx).
    ///
    /// This is used instead of `resolve_top_spell_from_stack` when a spell
    /// contains choice-based DiscardCards or Loot effects.
    fn resolve_top_spell_with_discard_hook(
        &mut self,
        spell_id: CardId,
        controller1: &mut dyn PlayerController,
        controller2: &mut dyn PlayerController,
    ) -> Result<()> {
        // Look up targets
        let targets: SmallVec<[CardId; 2]> = self
            .game
            .spell_targets
            .iter()
            .find(|(id, _)| *id == spell_id)
            .map(|(_, t)| t.clone())
            .unwrap_or_default();

        let should_log = self.verbosity >= VerbosityLevel::Normal && !self.replaying;

        if self.game.cards.get(spell_id).is_err() {
            return Err(crate::MtgError::EntityNotFound(spell_id.as_u32()));
        }

        let spell_owner = self.game.cards.get(spell_id).unwrap().owner;

        // Get card info for logging
        let (card_name, card_effects, card_owner) = if should_log {
            let card = self.game.cards.get(spell_id).unwrap();
            (card.name.to_string(), card.effects.clone(), card.owner)
        } else {
            (String::new(), Vec::new(), crate::core::PlayerId::new(0))
        };

        if should_log {
            let message = format!("{} ({}) resolves", card_name, spell_id);
            self.game.logger.gamelog(&message);
        }

        // Collect resolved effects without executing
        let effects = self.game.resolve_spell_collect_effects(spell_id, &targets)?;

        if let Some(effects) = effects {
            for effect in &effects {
                match effect {
                    // Choice-based discard: route through controller
                    crate::core::Effect::DiscardCards {
                        player,
                        count,
                        remember_discarded,
                        ..
                    } if *count != u8::MAX => {
                        let discard_count = *count as usize;
                        let remember = *remember_discarded;

                        // Push reveals so shadow sees any cards drawn earlier
                        self.push_reveals(*player);
                        if let Some(opp) = self.game.get_other_player_id(*player) {
                            self.push_reveals(opp);
                        }
                        self.sync_to_action();

                        let hand: SmallVec<[CardId; 8]> = self
                            .game
                            .get_player_zones(*player)
                            .map(|zones| zones.hand.cards.iter().copied().collect())
                            .unwrap_or_default();

                        let actual_count = discard_count.min(hand.len());
                        if actual_count > 0 {
                            let controller: &mut dyn PlayerController = if *player == controller1.player_id() {
                                controller1
                            } else {
                                controller2
                            };
                            let choice = self.choose_discard_with_hook(controller, *player, &hand, actual_count);
                            let cards_to_discard = match choice {
                                crate::game::controller::ChoiceResult::Ok(v) => v,
                                crate::game::controller::ChoiceResult::NeedInput(ctx) => {
                                    return Err(crate::MtgError::NeedInput(Box::new(ctx)));
                                }
                                crate::game::controller::ChoiceResult::ExitGame => {
                                    return Err(crate::MtgError::InvalidAction(
                                        "Game exit requested during spell discard".into(),
                                    ));
                                }
                                crate::game::controller::ChoiceResult::Error(e) => {
                                    return Err(crate::MtgError::InvalidAction(format!(
                                        "Controller error during spell discard: {e}"
                                    )));
                                }
                                crate::game::controller::ChoiceResult::UndoRequest(_) => {
                                    // Undo during spell resolution: skip this discard
                                    SmallVec::new()
                                }
                            };
                            for card_id in cards_to_discard {
                                if remember {
                                    self.game.remembered_cards.push(card_id);
                                }
                                if let Err(e) = self.game.discard_card(*player, card_id) {
                                    if should_log {
                                        eprintln!("    Failed to discard: {e}");
                                    }
                                }
                            }
                        }
                    }
                    // Loot: discard through controller, then draw
                    crate::core::Effect::Loot {
                        player,
                        discard_count,
                        draw_count,
                    } => {
                        let d_count = *discard_count as usize;

                        // Sync reveals before looting
                        self.sync_to_action();

                        if d_count > 0 {
                            let hand: SmallVec<[CardId; 8]> = self
                                .game
                                .get_player_zones(*player)
                                .map(|zones| zones.hand.cards.iter().copied().collect())
                                .unwrap_or_default();

                            let actual_count = d_count.min(hand.len());
                            if actual_count > 0 {
                                let controller: &mut dyn PlayerController = if *player == controller1.player_id() {
                                    controller1
                                } else {
                                    controller2
                                };
                                let choice = self.choose_discard_with_hook(controller, *player, &hand, actual_count);
                                let cards_to_discard = choice.into_result().map_err(|e| {
                                    crate::MtgError::InvalidAction(format!(
                                        "Loot discard choice failed during spell resolution: {e}"
                                    ))
                                })?;
                                for card_id in cards_to_discard {
                                    if let Err(e) = self.game.discard_card(*player, card_id) {
                                        if should_log {
                                            eprintln!("    Failed to discard: {e}");
                                        }
                                    }
                                }
                            }
                        }

                        for _ in 0..*draw_count {
                            match self.game.draw_card(*player) {
                                Ok((_, draw_num)) => {
                                    let _ = self.game.check_card_drawn_triggers(*player, draw_num);
                                }
                                Err(_) => break,
                            }
                        }
                    }
                    // All other effects: execute normally
                    _ => {
                        if let Err(e) = self.game.execute_effect(effect) {
                            if should_log {
                                eprintln!("    Failed to execute effect: {e}");
                            }
                        }
                    }
                }
            }
        }

        // Finalize the spell (move from stack to destination, ETB, etc.)
        self.game.resolve_spell_finalize(spell_id, &targets)?;

        // Push reveals after spell resolution
        self.push_reveals(spell_owner);
        if let Some(opponent) = self.game.get_other_player_id(spell_owner) {
            self.push_reveals(opponent);
        }

        // Log effects for display
        if should_log {
            use crate::core::{Effect, TargetRef};
            let mut target_index = 0;
            for effect in &card_effects {
                let effect_to_log = match effect {
                    Effect::DealDamage {
                        target: TargetRef::None,
                        amount,
                    } if target_index < targets.len() => {
                        let replaced = Effect::DealDamage {
                            target: TargetRef::Permanent(targets[target_index]),
                            amount: *amount,
                        };
                        target_index += 1;
                        replaced
                    }
                    Effect::CounterSpell { target } if target.is_placeholder() && target_index < targets.len() => {
                        let replaced = Effect::CounterSpell {
                            target: targets[target_index],
                        };
                        target_index += 1;
                        replaced
                    }
                    Effect::DestroyPermanent { target, restriction }
                        if target.is_placeholder() && target_index < targets.len() =>
                    {
                        let replaced = Effect::DestroyPermanent {
                            target: targets[target_index],
                            restriction: restriction.clone(),
                        };
                        target_index += 1;
                        replaced
                    }
                    Effect::DrawCards { player, count } if player.is_placeholder() => Effect::DrawCards {
                        player: card_owner,
                        count: *count,
                    },
                    Effect::DiscardCards {
                        player,
                        count,
                        remember_discarded,
                        ..
                    } if player.is_placeholder() => Effect::DiscardCards {
                        player: card_owner,
                        count: *count,
                        remember_discarded: *remember_discarded,
                        optional: false,
                        remember_discarding_players: false,
                    },
                    Effect::CreateToken {
                        controller,
                        token_script,
                        amount,
                        for_each_player,
                    } if *controller == crate::core::PlayerId::new(0) => Effect::CreateToken {
                        controller: card_owner,
                        token_script: token_script.clone(),
                        amount: *amount,
                        for_each_player: *for_each_player,
                    },
                    _ => effect.clone(),
                };
                self.log_effect_execution(&card_name, spell_id, &effect_to_log, card_owner);
            }

            if let Some(card) = self.game.cards.try_get(spell_id) {
                if card.is_creature() {
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
                    self.game.logger.gamelog(&message);
                }
            }

            // Set starting loyalty counters for planeswalkers (MTG CR 306.5b)
            if let Some(card) = self.game.cards.try_get(spell_id) {
                if let Some(loyalty) = card.definition.loyalty {
                    let card_name_str = card.name.to_string();
                    if let Ok(card_mut) = self.game.cards.get_mut(spell_id) {
                        card_mut.add_counter(crate::core::CounterType::Loyalty, loyalty);
                    }
                    if should_log {
                        self.game.logger.gamelog(&format!(
                            "{} ({}) enters with {} loyalty",
                            card_name_str, spell_id, loyalty
                        ));
                    }
                }
            }
        }

        // Check state-based actions
        if let Err(e) = self.game.check_lethal_damage() {
            if should_log {
                eprintln!("    Failed to check lethal damage: {e}");
            }
        }
        if let Err(e) = self.game.check_legendary_rule() {
            if should_log {
                eprintln!("    Failed to check legendary rule: {e}");
            }
        }
        if let Err(e) = self.game.check_aura_attachment() {
            if should_log {
                eprintln!("    Failed to check aura attachment: {e}");
            }
        }
        if let Err(e) = self.game.check_equipment_attachment() {
            if should_log {
                eprintln!("    Failed to check equipment attachment: {e}");
            }
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
                            if let Some(card) = self.game.cards.try_get(card_id) {
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
                let prior_log_size = self.game.logger.log_count();

                let choice_result = self.choose_sacrifice_with_hook(
                    controller,
                    player_id,
                    &valid_permanents,
                    sacrifice_count,
                    type_str,
                );

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
                    if let Some(card) = self.game.cards.try_get(card_id) {
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

                    // Move to graveyard (or exile if finality counter)
                    let dest = self.game.death_destination_for_card(card_id);
                    self.game.move_card(card_id, Zone::Battlefield, dest, owner)?;
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
