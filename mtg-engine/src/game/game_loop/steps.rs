//! Step handler functions for turn progression
//!
//! This module contains the individual step handlers (untap, upkeep, draw, main, end, cleanup)
//! that execute during each turn of the game.

use crate::core::{CardId, TriggerEvent};
use crate::game::controller::{format_discard_prompt, GameStateView, PlayerController};
use crate::{handle_choice_result_break, Result};
use smallvec::SmallVec;

use super::{GameLoop, GameResult, VerbosityLevel};

impl<'a> GameLoop<'a> {
    /// Untap step - untap all permanents controlled by active player
    pub(super) fn untap_step(&mut self) -> Result<()> {
        let active_player = self.game.turn.active_player;

        // Untap all permanents controlled by active player
        // Use SmallVec to avoid heap allocation for typical small counts of tapped cards
        let cards_to_untap: SmallVec<[CardId; 8]> = self
            .game
            .battlefield
            .cards
            .iter()
            .copied()
            .filter(|&card_id| {
                self.game
                    .cards
                    .get(card_id)
                    .map(|c| c.owner == active_player && c.tapped)
                    .unwrap_or(false)
            })
            .collect();

        for card_id in cards_to_untap {
            // Use untap_permanent to ensure mana cache is updated
            let _ = self.game.untap_permanent(card_id);
        }

        Ok(())
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
                if let Ok(card) = self.game.cards.get(card_id) {
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
            self.log_normal("(First turn - no draw)");
            return Ok(None);
        }

        // Drain any pending reveals from network before drawing
        // This ensures revealed cards are queued in the library before draw_card() pops them
        self.drain_reveals();

        // Debug: Log state hash before draw
        #[cfg(feature = "verbose-logging")]
        {
            let player_name = self.get_player_name(active_player);
            let draw_msg = format!("{} draws", player_name);
            self.game.debug_log_state_hash(&draw_msg);
        }

        // Draw a card
        self.game.draw_card(active_player)?;

        // Push reveals immediately for network mode (server-side)
        // This ensures clients receive the draw reveal before their GameLoop needs it
        self.push_reveals(active_player);

        #[cfg(feature = "verbose-logging")]
        {
            // Skip draw logging during replay mode (already logged in previous game segment)
            if !self.replaying {
                let player_name = self.get_player_name(active_player);
                if let Some(zones) = self.game.get_player_zones(active_player) {
                    // If this player's library is remote, we're viewing an opponent's draw
                    // from their hidden deck - don't log specific card names (they'd be wrong)
                    let is_remote_draw = zones.library.is_remote_library();
                    if is_remote_draw {
                        // For opponent draws from remote library, just log "draws a card"
                        log_gamelog!(self, "{} draws a card", player_name);
                    } else if let Some(&card_id) = zones.hand.cards.last() {
                        if let Ok(card) = self.game.cards.get(card_id) {
                            // Use gamelog for official draw action
                            log_gamelog!(self, "{} draws {} ({})", player_name, card.name, card_id);
                        } else {
                            log_gamelog!(self, "{} draws a card", player_name);
                        }
                    } else {
                        log_gamelog!(self, "{} draws a card", player_name);
                    }
                } else {
                    log_gamelog!(self, "{} draws a card", player_name);
                }
            }
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
            let hand_size = self
                .game
                .get_player_zones(player_id)
                .map(|z| z.hand.cards.len())
                .unwrap_or(0);

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
                let view = GameStateView::new(self.game, player_id);
                let hand = view.hand();
                let choice = controller.choose_cards_to_discard(&view, hand, discard_count);
                let cards_to_discard = handle_choice_result_break!(choice, self.game, player_id);

                // Log this choice point for snapshot/replay
                let replay_choice = crate::game::ReplayChoice::Discard(cards_to_discard.clone());
                self.log_choice_point(player_id, Some(replay_choice), prior_log_size);

                // Verify correct number of cards
                if cards_to_discard.len() != discard_count {
                    return Err(crate::MtgError::InvalidAction(format!(
                        "Must discard exactly {discard_count} cards, got {}",
                        cards_to_discard.len()
                    )));
                }

                // Move cards to graveyard
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

        // TODO: Remove damage from creatures

        Ok(None)
    }
}
