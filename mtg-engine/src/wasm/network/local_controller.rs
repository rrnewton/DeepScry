//! WASM Network Local Controller
//!
//! Wraps the local player's controller and coordinates with the server.
//! Returns `NeedInput` when waiting for server synchronization.
//!
//! ## Flow
//!
//! 1. Wait for ChoiceRequest from server (or NeedInput)
//! 2. Delegate to inner controller for actual choice
//! 3. Queue SubmitChoice message
//! 4. Wait for ChoiceAccepted (or NeedInput)
//! 5. Return choice to GameLoop

use super::client::SharedNetworkClient;
use crate::core::{CardId, ManaCost, PlayerId, SpellAbility};
use crate::game::controller::{sort_spell_abilities, ChoiceContext, ChoiceResult, GameStateView, PlayerController};
use crate::game::snapshot::ControllerType;
use crate::wasm::human_controller::{PendingChoice, WasmHumanController};
use smallvec::SmallVec;

/// Extended choice context variants for network waiting
///
/// These are returned when we need to wait for the network.
/// They use the existing ChoiceContext enum but with empty data
/// to signal a "waiting" state to the UI layer.
fn waiting_for_server_context() -> ChoiceContext {
    // Use SpellAbility context with empty data to signal waiting
    ChoiceContext::SpellAbility {
        available: vec![],
        formatted_choices: vec!["Waiting for server...".to_string()],
    }
}

fn waiting_for_ack_context() -> ChoiceContext {
    ChoiceContext::SpellAbility {
        available: vec![],
        formatted_choices: vec!["Waiting for acknowledgment...".to_string()],
    }
}

/// WASM Network Local Controller
///
/// Wraps the local player's controller (typically WasmHumanController) and
/// ensures synchronization with the server before and after each choice.
pub struct WasmNetworkLocalController {
    player_id: PlayerId,
    /// The inner controller that makes actual decisions
    inner: WasmHumanController,
    /// Shared reference to the network client
    network_client: SharedNetworkClient,
}

impl WasmNetworkLocalController {
    /// Create a new network local controller
    pub fn new(player_id: PlayerId, network_client: SharedNetworkClient) -> Self {
        Self {
            player_id,
            inner: WasmHumanController::new(player_id),
            network_client,
        }
    }

    /// Create with an existing inner controller
    pub fn with_inner(player_id: PlayerId, inner: WasmHumanController, network_client: SharedNetworkClient) -> Self {
        Self {
            player_id,
            inner,
            network_client,
        }
    }

    /// Get a mutable reference to the inner controller
    pub fn inner_mut(&mut self) -> &mut WasmHumanController {
        &mut self.inner
    }

    /// Set a pending choice on the inner controller
    pub fn set_pending_choice(&mut self, choice: PendingChoice) {
        self.inner.set_pending_choice(choice);
    }

    /// Check if waiting for server
    fn wait_for_choice_request(&self) -> bool {
        self.network_client.borrow().has_choice_request()
    }

    /// Check if choice was acknowledged
    fn wait_for_choice_ack(&self) -> bool {
        self.network_client.borrow().is_choice_acknowledged()
    }

    /// Submit a choice to the server
    fn submit_choice(&self, choice_index: usize, view: &GameStateView) {
        let action_count = view.action_count() as u64;
        let state_hash = if self.network_client.borrow().is_network_debug() {
            Some(crate::game::compute_view_hash(view))
        } else {
            None
        };
        self.network_client
            .borrow_mut()
            .submit_choice(choice_index, action_count, state_hash);
    }
}

impl PlayerController for WasmNetworkLocalController {
    fn player_id(&self) -> PlayerId {
        self.player_id
    }

    fn choose_spell_ability_to_play(
        &mut self,
        view: &GameStateView,
        available: &[SpellAbility],
    ) -> ChoiceResult<Option<SpellAbility>> {
        // 1. Check for ChoiceRequest from server
        if !self.wait_for_choice_request() {
            return ChoiceResult::NeedInput(waiting_for_server_context());
        }

        // 2. Delegate to inner controller
        let sorted = sort_spell_abilities(available);
        match self.inner.choose_spell_ability_to_play(view, available) {
            ChoiceResult::Ok(choice) => {
                // 3. Submit choice to server
                let choice_index = match &choice {
                    None => 0, // Pass
                    Some(ability) => sorted.iter().position(|a| a == ability).map(|i| i + 1).unwrap_or(0),
                };
                self.submit_choice(choice_index, view);

                // 4. Check for acknowledgment
                if !self.wait_for_choice_ack() {
                    // Store choice to replay after ack
                    return ChoiceResult::NeedInput(waiting_for_ack_context());
                }

                ChoiceResult::Ok(choice)
            }
            ChoiceResult::NeedInput(ctx) => ChoiceResult::NeedInput(ctx),
            other => other,
        }
    }

    fn choose_targets(
        &mut self,
        view: &GameStateView,
        spell: CardId,
        valid_targets: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        if !self.wait_for_choice_request() {
            return ChoiceResult::NeedInput(waiting_for_server_context());
        }

        match self.inner.choose_targets(view, spell, valid_targets) {
            ChoiceResult::Ok(targets) => {
                // For targets, choice_index encodes the selection
                // For simplicity, we send the first target's index
                let choice_index = if targets.is_empty() {
                    valid_targets.len() // "none" option
                } else {
                    valid_targets.iter().position(|&t| t == targets[0]).unwrap_or(0)
                };
                self.submit_choice(choice_index, view);

                if !self.wait_for_choice_ack() {
                    return ChoiceResult::NeedInput(waiting_for_ack_context());
                }

                ChoiceResult::Ok(targets)
            }
            other => other,
        }
    }

    fn choose_mana_sources_to_pay(
        &mut self,
        view: &GameStateView,
        cost: &ManaCost,
        available_sources: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        if !self.wait_for_choice_request() {
            return ChoiceResult::NeedInput(waiting_for_server_context());
        }

        match self.inner.choose_mana_sources_to_pay(view, cost, available_sources) {
            ChoiceResult::Ok(sources) => {
                // Encode as bitmask or first source index
                let choice_index = if sources.is_empty() {
                    0
                } else {
                    available_sources.iter().position(|&s| s == sources[0]).unwrap_or(0)
                };
                self.submit_choice(choice_index, view);

                if !self.wait_for_choice_ack() {
                    return ChoiceResult::NeedInput(waiting_for_ack_context());
                }

                ChoiceResult::Ok(sources)
            }
            other => other,
        }
    }

    fn choose_attackers(
        &mut self,
        view: &GameStateView,
        available_creatures: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        if !self.wait_for_choice_request() {
            return ChoiceResult::NeedInput(waiting_for_server_context());
        }

        match self.inner.choose_attackers(view, available_creatures) {
            ChoiceResult::Ok(attackers) => {
                // Encode attackers - for now just send count as a simple encoding
                let choice_index = attackers.len();
                self.submit_choice(choice_index, view);

                if !self.wait_for_choice_ack() {
                    return ChoiceResult::NeedInput(waiting_for_ack_context());
                }

                ChoiceResult::Ok(attackers)
            }
            other => other,
        }
    }

    fn choose_blockers(
        &mut self,
        view: &GameStateView,
        available_blockers: &[CardId],
        attackers: &[CardId],
    ) -> ChoiceResult<SmallVec<[(CardId, CardId); 8]>> {
        if !self.wait_for_choice_request() {
            return ChoiceResult::NeedInput(waiting_for_server_context());
        }

        match self.inner.choose_blockers(view, available_blockers, attackers) {
            ChoiceResult::Ok(blocks) => {
                // Encode blockers
                let choice_index = blocks.len();
                self.submit_choice(choice_index, view);

                if !self.wait_for_choice_ack() {
                    return ChoiceResult::NeedInput(waiting_for_ack_context());
                }

                ChoiceResult::Ok(blocks)
            }
            other => other,
        }
    }

    fn choose_damage_assignment_order(
        &mut self,
        view: &GameStateView,
        attacker: CardId,
        blockers: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        if !self.wait_for_choice_request() {
            return ChoiceResult::NeedInput(waiting_for_server_context());
        }

        match self.inner.choose_damage_assignment_order(view, attacker, blockers) {
            ChoiceResult::Ok(order) => {
                let choice_index = 0; // Default order
                self.submit_choice(choice_index, view);

                if !self.wait_for_choice_ack() {
                    return ChoiceResult::NeedInput(waiting_for_ack_context());
                }

                ChoiceResult::Ok(order)
            }
            other => other,
        }
    }

    fn choose_cards_to_discard(
        &mut self,
        view: &GameStateView,
        hand: &[CardId],
        count: usize,
    ) -> ChoiceResult<SmallVec<[CardId; 7]>> {
        if !self.wait_for_choice_request() {
            return ChoiceResult::NeedInput(waiting_for_server_context());
        }

        match self.inner.choose_cards_to_discard(view, hand, count) {
            ChoiceResult::Ok(discards) => {
                let choice_index = if discards.is_empty() {
                    0
                } else {
                    hand.iter().position(|&c| c == discards[0]).unwrap_or(0)
                };
                self.submit_choice(choice_index, view);

                if !self.wait_for_choice_ack() {
                    return ChoiceResult::NeedInput(waiting_for_ack_context());
                }

                ChoiceResult::Ok(discards)
            }
            other => other,
        }
    }

    fn choose_from_library(&mut self, view: &GameStateView, valid_cards: &[CardId]) -> ChoiceResult<Option<CardId>> {
        if !self.wait_for_choice_request() {
            return ChoiceResult::NeedInput(waiting_for_server_context());
        }

        match self.inner.choose_from_library(view, valid_cards) {
            ChoiceResult::Ok(choice) => {
                let choice_index = match choice {
                    None => valid_cards.len(),
                    Some(card) => valid_cards.iter().position(|&c| c == card).unwrap_or(0),
                };
                self.submit_choice(choice_index, view);

                if !self.wait_for_choice_ack() {
                    return ChoiceResult::NeedInput(waiting_for_ack_context());
                }

                ChoiceResult::Ok(choice)
            }
            other => other,
        }
    }

    fn choose_permanents_to_sacrifice(
        &mut self,
        view: &GameStateView,
        valid_permanents: &[CardId],
        count: usize,
        card_type_description: &str,
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        if !self.wait_for_choice_request() {
            return ChoiceResult::NeedInput(waiting_for_server_context());
        }

        match self
            .inner
            .choose_permanents_to_sacrifice(view, valid_permanents, count, card_type_description)
        {
            ChoiceResult::Ok(sacrifices) => {
                let choice_index = if sacrifices.is_empty() {
                    0
                } else {
                    valid_permanents.iter().position(|&c| c == sacrifices[0]).unwrap_or(0)
                };
                self.submit_choice(choice_index, view);

                if !self.wait_for_choice_ack() {
                    return ChoiceResult::NeedInput(waiting_for_ack_context());
                }

                ChoiceResult::Ok(sacrifices)
            }
            other => other,
        }
    }

    fn on_priority_passed(&mut self, view: &GameStateView) {
        self.inner.on_priority_passed(view);
    }

    fn on_game_end(&mut self, view: &GameStateView, won: bool) {
        self.inner.on_game_end(view, won);
    }

    fn get_controller_type(&self) -> ControllerType {
        ControllerType::Network
    }
}
