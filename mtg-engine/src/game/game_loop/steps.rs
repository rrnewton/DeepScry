//! Step handler functions for turn progression
//!
//! This module contains the individual step handlers (untap, upkeep, draw, main, end, cleanup)
//! that execute during each turn of the game.

use crate::core::{CardId, Keyword, PlayerId, TriggerEvent};
use crate::game::controller::{format_discard_prompt, ChoiceResult, GameStateView, PlayerController};
use crate::{handle_choice_result_break, Result};
use smallvec::SmallVec;

use super::{GameLoop, GameResult, VerbosityLevel};

impl<'a> GameLoop<'a> {
    /// Untap step - untap all permanents controlled by active player
    ///
    /// If permanents have "You may choose not to untap CARDNAME during your untap step"
    /// (MayNotUntap keyword), the controller is asked which permanents to keep tapped.
    pub(super) fn untap_step(
        &mut self,
        controller1: &mut dyn PlayerController,
        controller2: &mut dyn PlayerController,
    ) -> Result<Option<GameResult>> {
        let active_player = self.game.turn.active_player;

        // Reset draw count for the active player at the start of their turn
        // This tracks "cards drawn this turn" for triggers like Knowledge Seeker
        let prior_log_size = self.game.logger.log_count();
        if let Ok(player) = self.game.get_player_mut(active_player) {
            let old_count = player.cards_drawn_this_turn;
            player.reset_cards_drawn();
            // Log for undo
            self.game.undo_log.log(
                crate::undo::GameAction::SetCardsDrawnThisTurn {
                    player_id: active_player,
                    old_value: old_count,
                    new_value: 0,
                },
                prior_log_size,
            );
        }

        // Collect tapped permanents controlled by active player
        // Separate into: normal permanents and MayNotUntap permanents
        let mut normal_to_untap: SmallVec<[CardId; 8]> = SmallVec::new();
        let mut may_not_untap: SmallVec<[CardId; 8]> = SmallVec::new();

        for &card_id in &self.game.battlefield.cards {
            if let Some(card) = self.game.cards.try_get(card_id) {
                if card.controller == active_player && card.tapped {
                    if card.keywords.contains(Keyword::MayNotUntap) {
                        may_not_untap.push(card_id);
                    } else {
                        normal_to_untap.push(card_id);
                    }
                }
            }
        }

        // Untap all normal permanents
        for card_id in normal_to_untap {
            let _ = self.game.untap_permanent(card_id);
        }

        // If there are MayNotUntap permanents, ask controller which to keep tapped
        if !may_not_untap.is_empty() {
            let controller: &mut dyn PlayerController = if active_player == self.game.players[0].id {
                controller1
            } else {
                controller2
            };

            let choice = self.choose_not_untap_with_hook(controller, active_player, &may_not_untap);

            let stay_tapped: SmallVec<[CardId; 8]> = match choice {
                ChoiceResult::Ok(ids) => ids,
                ChoiceResult::ExitGame => {
                    return Ok(Some(GameResult {
                        winner: self.game.get_other_player_id(active_player),
                        turns_played: self.turns_elapsed,
                        end_reason: super::GameEndReason::Manual,
                        action_count: self.game.action_count(),
                    }));
                }
                ChoiceResult::Error(e) => {
                    log::error!("Error in choose_permanents_to_not_untap: {}", e);
                    SmallVec::new() // Default to untapping everything
                }
                ChoiceResult::UndoRequest(_) => SmallVec::new(), // Can't undo untap step
                ChoiceResult::NeedInput(_) => SmallVec::new(),   // Not supported in untap
            };

            // Untap MayNotUntap permanents that weren't chosen to stay tapped
            for card_id in may_not_untap {
                if !stay_tapped.contains(&card_id) {
                    let _ = self.game.untap_permanent(card_id);
                } else if self.verbosity >= VerbosityLevel::Normal {
                    // Log that this permanent stays tapped
                    if let Some(card) = self.game.cards.try_get(card_id) {
                        let player_name = self.get_player_name(active_player);
                        self.log_normal(&format!("{} chooses not to untap {}", player_name, card.name));
                    }
                }
            }
        }

        Ok(None)
    }

    /// Check and execute phase-triggered abilities
    pub(super) fn check_phase_triggers(&mut self, trigger_event: TriggerEvent) -> Result<()> {
        let active_player = self.game.turn.active_player;

        // Collect all permanents with triggers matching this event
        // Also collect trigger descriptions for logging
        let triggered_info: SmallVec<[(CardId, Vec<String>); 4]> = self
            .game
            .battlefield
            .cards
            .iter()
            .filter_map(|&card_id| {
                if let Some(card) = self.game.cards.try_get(card_id) {
                    // Filter triggers: match event and respect ValidPlayer$ You restriction
                    let matching_descriptions: Vec<String> = card
                        .triggers
                        .iter()
                        .filter(|t| {
                            if t.event != trigger_event {
                                return false;
                            }
                            // Check [controller_only] flag - if present, only fire on controller's turn
                            // This implements ValidPlayer$ You from the card definition
                            if t.description.starts_with("[controller_only]") {
                                return card.controller == active_player;
                            }
                            true
                        })
                        .map(|t| {
                            // Strip the [controller_only] prefix for display
                            let desc = t
                                .description
                                .strip_prefix("[controller_only] ")
                                .unwrap_or(&t.description);
                            format!("Trigger: {} - {}", card.name, desc)
                        })
                        .collect();

                    if !matching_descriptions.is_empty() {
                        Some((card_id, matching_descriptions))
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect();

        // For each card with a matching trigger, log and execute
        // Note: In the future, this will need to handle optional triggers, conditions, etc.
        for (card_id, descriptions) in triggered_info {
            // Log trigger activation if verbose
            if self.verbosity >= VerbosityLevel::Verbose {
                for desc in descriptions {
                    self.log_verbose(&desc);
                }
            }

            // Use the existing check_triggers method to execute effects
            // Pass the card_id as the source for filling in placeholders
            self.game
                .check_triggers_for_controller(trigger_event, card_id, active_player)?;
        }

        // Push reveals after phase triggers for network mode (server-side)
        // Triggered abilities can draw cards, and clients need the card IDs
        self.push_reveals(active_player);
        if let Some(opponent) = self.game.get_other_player_id(active_player) {
            self.push_reveals(opponent);
        }

        Ok(())
    }

    /// Check and fire AttackersDeclared triggers (batch triggers that fire once per declare attackers step)
    /// These differ from Attacks triggers which fire per-creature
    /// Example: "Whenever one or more creatures you control with flying attack"
    pub(super) fn check_attackers_declared_triggers(&mut self, attacking_player: PlayerId) -> Result<()> {
        // Get the list of attacking creatures (need to check keyword filters)
        let attackers: SmallVec<[CardId; 8]> = self.game.combat.attackers.keys().copied().collect();

        if attackers.is_empty() {
            return Ok(()); // No attackers, no triggers
        }

        // Collect all permanents with AttackersDeclared triggers that should fire
        let triggered_cards: SmallVec<[(CardId, String); 4]> = self
            .game
            .battlefield
            .cards
            .iter()
            .filter_map(|&card_id| {
                if let Some(card) = self.game.cards.try_get(card_id) {
                    // Find matching AttackersDeclared triggers
                    for trigger in &card.triggers {
                        if trigger.event != TriggerEvent::AttackersDeclared {
                            continue;
                        }

                        // Check controller_turn_only (AttackingPlayer$ You)
                        if trigger.controller_turn_only && card.controller != attacking_player {
                            continue;
                        }

                        // Check valid_attackers_keyword filter
                        if let Some(required_keyword) = trigger.valid_attackers_keyword {
                            // At least one attacker must have the required keyword
                            let has_matching_attacker = attackers.iter().any(|&attacker_id| {
                                if let Ok(attacker) = self.game.cards.get(attacker_id) {
                                    // Check if attacker is controlled by the triggering player
                                    if attacker.controller != card.controller {
                                        return false;
                                    }
                                    // Check for the required keyword
                                    attacker.keywords.contains(required_keyword)
                                } else {
                                    false
                                }
                            });

                            if !has_matching_attacker {
                                continue;
                            }
                        }

                        // Trigger conditions met!
                        return Some((card_id, trigger.description.clone()));
                    }
                    None
                } else {
                    None
                }
            })
            .collect();

        // Fire each trigger
        for (card_id, description) in triggered_cards {
            if self.verbosity >= VerbosityLevel::Verbose && !self.replaying {
                if let Some(card) = self.game.cards.try_get(card_id) {
                    let message = format!("Trigger: {} - {}", card.name, description);
                    self.log_verbose(&message);
                }
            }

            // Execute the trigger effects
            if let Some(card) = self.game.cards.try_get(card_id) {
                let controller = card.controller;
                self.game
                    .check_triggers_for_controller(TriggerEvent::AttackersDeclared, card_id, controller)?;
            }
        }

        // Push reveals after triggers
        self.push_reveals(attacking_player);
        if let Some(opponent) = self.game.get_other_player_id(attacking_player) {
            self.push_reveals(opponent);
        }

        Ok(())
    }

    /// Upkeep step - priority round for triggers and actions
    pub(super) fn upkeep_step(
        &mut self,
        controller1: &mut dyn PlayerController,
        controller2: &mut dyn PlayerController,
    ) -> Result<Option<GameResult>> {
        // Check for beginning of upkeep triggers
        self.check_phase_triggers(TriggerEvent::BeginningOfUpkeep)?;

        // Pass priority
        if let Some(result) = self.priority_round(controller1, controller2)? {
            return Ok(Some(result));
        }
        Ok(None)
    }

    /// Draw step - active player draws a card
    pub(super) fn draw_step(
        &mut self,
        controller1: &mut dyn PlayerController,
        controller2: &mut dyn PlayerController,
    ) -> Result<Option<GameResult>> {
        let active_player = self.game.turn.active_player;

        // Skip draw on first turn (player going first doesn't draw)
        if self.game.turn.turn_number == 1 {
            // Still print battlefield state even on turn 1 (no draw)
            if !self.replaying && self.verbosity >= VerbosityLevel::Normal {
                self.print_battlefield_state();
            }
            self.log_normal("(First turn - no draw)");
            return Ok(None);
        }

        // Guard against re-entry: WASM harness creates a new GameLoop on each step_harness() call.
        // If priority_round() blocks with NeedInput, current_step stays at Draw (advance_step()
        // is never called), so the next step_harness() call would re-execute draw_card() again.
        // We track which turn we already drew on to skip the draw on re-entry.
        let current_turn = self.game.turn.turn_number;
        let already_drew = self.game.turn.draw_step_executed_turn == Some(current_turn);
        if !already_drew {
            // Sync network state before drawing
            // This ensures revealed cards are queued in the library before draw_card() pops them
            self.sync_to_action();

            // Debug: Log state hash before draw
            #[cfg(feature = "verbose-logging")]
            {
                let player_name = self.get_player_name(active_player);
                let draw_msg = format!("{} draws", player_name);
                self.game.debug_log_state_hash(&draw_msg);
            }

            // Draw a card
            let (_, draw_count) = self.game.draw_card(active_player)?;

            // Mark this turn's mandatory draw as executed
            self.game.turn.draw_step_executed_turn = Some(current_turn);

            // Check for "second card drawn" triggers (e.g., Knowledge Seeker, Otter-Penguin)
            self.game.check_card_drawn_triggers(active_player, draw_count)?;

            // Push reveals immediately for network mode (server-side)
            // This ensures clients receive the draw reveal before their GameLoop needs it
            self.push_reveals(active_player);
        }

        #[cfg(feature = "verbose-logging")]
        {
            // Skip draw logging during replay mode (already logged in previous game segment)
            if !self.replaying {
                let player_name = self.get_player_name(active_player);
                if let Some(zones) = self.game.get_player_zones(active_player) {
                    if let Some(&card_id) = zones.hand.cards.last() {
                        // Late-binding: CardID is known, but card identity may not be
                        if let Some(card) = self.game.cards.try_get(card_id) {
                            log_gamelog!(self, "{} draws {} ({})", player_name, card.name, card_id);
                        } else {
                            // Card not yet revealed - just log the draw
                            log_gamelog!(self, "{} draws a card (id={})", player_name, card_id);
                        }
                    } else {
                        log_gamelog!(self, "{} draws a card", player_name);
                    }
                } else {
                    log_gamelog!(self, "{} draws a card", player_name);
                }
            }
        }

        // Print battlefield state AFTER draw step completes
        // This ensures the active player's hand shows the newly drawn card
        // (Previously this was printed at turn start, before draw - see mtg-p9svf)
        if !self.replaying && self.verbosity >= VerbosityLevel::Normal {
            self.print_battlefield_state();
        }

        // MTG Rules 504.2: After draw, players receive priority
        if let Some(result) = self.priority_round(controller1, controller2)? {
            return Ok(Some(result));
        }

        Ok(None)
    }

    /// Main phase - players can play spells and lands
    pub(super) fn main_phase(
        &mut self,
        controller1: &mut dyn PlayerController,
        controller2: &mut dyn PlayerController,
    ) -> Result<Option<GameResult>> {
        // Priority round where players can take actions
        if let Some(result) = self.priority_round(controller1, controller2)? {
            return Ok(Some(result));
        }
        Ok(None)
    }

    /// End step - handle end of turn triggers and priority
    pub(super) fn end_step(
        &mut self,
        controller1: &mut dyn PlayerController,
        controller2: &mut dyn PlayerController,
    ) -> Result<Option<GameResult>> {
        // Check for beginning of end step triggers
        self.check_phase_triggers(TriggerEvent::BeginningOfEndStep)?;

        if let Some(result) = self.priority_round(controller1, controller2)? {
            return Ok(Some(result));
        }
        Ok(None)
    }

    /// Cleanup step - discard to hand size, remove damage
    pub(super) fn cleanup_step(
        &mut self,
        controller1: &mut dyn PlayerController,
        controller2: &mut dyn PlayerController,
    ) -> Result<Option<GameResult>> {
        let active_player = self.game.turn.active_player;

        // Get non-active player
        let non_active_player = self
            .game
            .get_other_player_id(active_player)
            .expect("Should have non-active player");

        // Process active player first, then non-active player
        for &player_id in &[active_player, non_active_player] {
            let hand_size = self.game.get_player_zones(player_id).map(|z| z.hand.len()).unwrap_or(0);

            let max_hand_size = self.game.get_player(player_id)?.max_hand_size;

            if hand_size > max_hand_size {
                let discard_count = hand_size - max_hand_size;

                log_if_verbose!(
                    self,
                    "{} must discard {} cards (hand size: {}, max: {})",
                    self.get_player_name(player_id),
                    discard_count,
                    hand_size,
                    max_hand_size
                );

                // Get the appropriate controller
                let controller: &mut dyn PlayerController = if player_id == controller1.player_id() {
                    controller1
                } else {
                    controller2
                };

                // Create view and print prompt BEFORE checking stop conditions
                // so users see what choice was about to be made when using --stop-when-fixed-exhausted
                {
                    let view = GameStateView::new(self.game, player_id);
                    let hand = view.hand();
                    // Print discard selection prompt (controlled by show_choice_menu flag)
                    if view.logger().should_show_choice_menu() {
                        print!("{}", format_discard_prompt(&view, hand, discard_count));
                    }
                } // Drop view before mutable borrow

                // PREAMBLE: Check stop conditions before asking for choice
                if let Some(result) = self.check_stop_conditions(controller, player_id)? {
                    return Ok(Some(result));
                }

                // Ask controller which cards to discard
                // Capture log size BEFORE asking controller (before controller logs its choice)
                let prior_log_size = self.game.logger.log_count();
                // Get hand cards before calling helper (which creates view internally)
                let hand: SmallVec<[CardId; 8]> = self
                    .game
                    .get_player_zones(player_id)
                    .map(|zones| zones.hand.cards.iter().copied().collect())
                    .unwrap_or_default();
                let choice = self.choose_discard_with_hook(controller, player_id, &hand, discard_count);
                let cards_to_discard = handle_choice_result_break!(choice, self.game, player_id);

                // Log this choice point for snapshot/replay
                let replay_choice = crate::game::ReplayChoice::Discard(cards_to_discard.clone());
                self.log_choice_point(player_id, Some(replay_choice), prior_log_size);

                // Verify correct number of cards selected
                if cards_to_discard.len() != discard_count {
                    return Err(crate::MtgError::InvalidAction(format!(
                        "Must discard exactly {discard_count} cards, got {}",
                        cards_to_discard.len()
                    )));
                }

                // Move selected cards to graveyard
                for card_id in cards_to_discard {
                    // Verify card is in hand before moving
                    if let Some(zones) = self.game.get_player_zones(player_id) {
                        if !zones.hand.contains(card_id) {
                            return Err(crate::MtgError::InvalidAction(format!(
                                "Card {card_id:?} not in player's hand"
                            )));
                        }
                    }

                    // Use move_card to properly log the action for undo
                    self.game.move_card(
                        card_id,
                        crate::zones::Zone::Hand,
                        crate::zones::Zone::Graveyard,
                        player_id,
                    )?;

                    log_if_verbose!(
                        self,
                        "{} discards {} ({})",
                        self.get_player_name(player_id),
                        self.game
                            .cards
                            .get(card_id)
                            .map(|c| c.name.as_str())
                            .unwrap_or("Unknown"),
                        card_id
                    );
                }
            }
        }

        // Empty mana pools
        for &player_id in &[active_player, non_active_player] {
            if let Ok(player) = self.game.get_player_mut(player_id) {
                player.mana_pool.clear();
            }
        }

        // Clean up persistent effects that expire at end of turn
        let effects_to_remove = self.game.persistent_effects.find_effects_to_cleanup_at_eot();
        if !effects_to_remove.is_empty() {
            log::debug!(target: "persistent_effects", "Cleaning up {} effects at end of turn", effects_to_remove.len());
            self.game.persistent_effects.remove_many(&effects_to_remove);
        }

        // Clean up delayed triggers that expire at end of turn (ThisTurn$ True)
        // Example: Fatal Fissure's "when that creature dies this turn" expires if not triggered
        let expired_triggers = self.game.delayed_triggers.cleanup_end_of_turn();
        if !expired_triggers.is_empty() {
            log::debug!(
                target: "delayed_triggers",
                "Cleaning up {} delayed triggers at end of turn",
                expired_triggers.len()
            );
            for trigger in &expired_triggers {
                log_if_verbose!(self, "Delayed trigger {} expired (end of turn)", trigger.id.as_u32());
            }
        }

        // Remove damage from creatures and clear regeneration shields (CR 514.2)
        for &card_id in &self.game.battlefield.cards {
            if let Ok(card) = self.game.cards.get_mut(card_id) {
                if card.is_creature() {
                    card.damage = 0;
                    card.regeneration_shields = 0;
                }
            }
        }

        Ok(None)
    }
}
