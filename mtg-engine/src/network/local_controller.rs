//! Network-aware local controller wrapper
//!
//! This wraps any PlayerController and sends choices to the server after each decision.
//! It also receives CardRevealed messages to populate remote libraries before draws.
//!
//! ## Architecture
//!
//! ```text
//! GameLoop calls LocalController methods
//!     │
//!     ▼
//! NetworkLocalController
//!     │
//!     ├─► Delegates to inner controller (InteractiveController, etc.)
//!     │
//!     └─► Sends choice to server via channel
//!         Server validates and broadcasts to opponent
//! ```

use crate::core::{CardId, ManaCost, PlayerId, SpellAbility};
use crate::game::controller::{ChoiceResult, GameStateView, PlayerController};
use crate::game::snapshot::ControllerType;
use smallvec::SmallVec;
use std::sync::mpsc;

/// A choice made by the local player, to be sent to the server
#[derive(Debug, Clone)]
pub struct LocalChoice {
    /// The choice index selected
    pub choice_index: usize,
    /// Human-readable description
    pub description: String,
    /// Action count (undo log position) at the time of choice
    /// This is used for synchronization validation with the server
    pub action_count: u64,
    /// Last N actions from the undo log (for sync debugging)
    /// Only populated when debug mode is enabled
    pub last_actions: Option<String>,
}

/// Message types for the local controller
#[derive(Debug)]
pub enum LocalControllerMessage {
    /// A card was revealed by the server (queue for drawing)
    CardRevealed { owner: PlayerId, card_id: CardId },
    /// Server is requesting a choice - the controller should proceed to make one
    ///
    /// This synchronizes the client's GameLoop with the server's:
    /// 1. Server reaches choice point in its GameLoop
    /// 2. Server sends ChoiceRequest to client via WebSocket
    /// 3. Client's NetworkLocalController receives this message
    /// 4. Client's GameLoop proceeds to make the choice
    /// 5. Client sends SubmitChoice back to server
    ///
    /// This ensures the client never gets ahead of the server.
    ChoiceRequest {
        /// Server's authoritative action count (for sync validation)
        action_count: u64,
        /// Server's choice sequence number
        choice_seq: u32,
    },
    /// Server acknowledged our choice, continue
    ChoiceAcknowledged,
    /// Server reported an error
    Error(String),
    /// Game has ended
    GameEnded,
}

/// A controller that wraps a local controller and sends choices to the server.
///
/// This is used on the client side for our player. When the GameLoop asks for a choice,
/// we drain any pending messages (which may include ChoiceRequest with the server's
/// authoritative action_count), delegate to the inner controller, then send the result
/// to the server and wait for acknowledgment.
///
/// The flow is:
/// 1. GameLoop calls choose_spell_ability_to_play() (or similar)
/// 2. We drain pending messages (ChoiceRequest updates server_action_count if present)
/// 3. We delegate to inner controller
/// 4. We send the choice via choice_tx using server's action_count if available
/// 5. We wait for ChoiceAcknowledged from server
/// 6. We return the result to GameLoop
pub struct NetworkLocalController<C: PlayerController> {
    /// The wrapped local controller
    inner: C,
    /// Channel to send our choices to the WebSocket handler
    choice_tx: mpsc::Sender<LocalChoice>,
    /// Channel to receive messages from the WebSocket handler
    message_rx: mpsc::Receiver<LocalControllerMessage>,
    /// Whether we've been disconnected
    disconnected: bool,
    /// Network debug mode: include action log info in choices for sync validation
    network_debug: bool,
    /// Last received server action count (from ChoiceRequest)
    server_action_count: Option<u64>,
    /// Last received server choice sequence (from ChoiceRequest)
    server_choice_seq: Option<u32>,
}

impl<C: PlayerController> NetworkLocalController<C> {
    /// Create a new network local controller
    ///
    /// # Arguments
    /// * `inner` - The actual controller to delegate choices to
    /// * `choice_tx` - Channel to send choices to WebSocket handler
    /// * `message_rx` - Channel to receive server messages
    pub fn new(
        inner: C,
        choice_tx: mpsc::Sender<LocalChoice>,
        message_rx: mpsc::Receiver<LocalControllerMessage>,
    ) -> Self {
        Self {
            inner,
            choice_tx,
            message_rx,
            disconnected: false,
            network_debug: false,
            server_action_count: None,
            server_choice_seq: None,
        }
    }

    /// Enable network debug mode for action log transmission
    ///
    /// When enabled, the last N actions are included with each choice
    /// for sync validation and debugging.
    pub fn with_network_debug(mut self, enabled: bool) -> Self {
        self.network_debug = enabled;
        self
    }

    /// Wait for a ChoiceRequest from the server before making a choice
    ///
    /// This blocks until the server sends a ChoiceRequest, ensuring the client
    /// doesn't run ahead of the server. If a ChoiceRequest was already received,
    /// this returns immediately.
    ///
    /// Returns true if we're ready to proceed, false if we're disconnected.
    fn wait_for_choice_request(&mut self) -> bool {
        // If we already have server state from a previous message, we're ready
        if self.server_action_count.is_some() && self.server_choice_seq.is_some() {
            log::trace!(
                "NetworkLocalController: using pre-received ChoiceRequest seq={:?} action_count={:?}",
                self.server_choice_seq,
                self.server_action_count
            );
            return true;
        }

        // Block waiting for ChoiceRequest
        log::trace!("NetworkLocalController: blocking on message_rx.recv() for ChoiceRequest");
        loop {
            match self.message_rx.recv() {
                Ok(LocalControllerMessage::ChoiceRequest {
                    action_count,
                    choice_seq,
                }) => {
                    log::trace!(
                        "NetworkLocalController: received ChoiceRequest seq={} action_count={}",
                        choice_seq,
                        action_count
                    );
                    self.server_action_count = Some(action_count);
                    self.server_choice_seq = Some(choice_seq);
                    return true;
                }
                Ok(LocalControllerMessage::CardRevealed { owner, card_id }) => {
                    log::debug!(
                        "NetworkLocalController: waiting for ChoiceRequest, got card reveal: {:?} -> {:?}",
                        owner,
                        card_id
                    );
                    // Continue waiting
                }
                Ok(LocalControllerMessage::GameEnded) => {
                    log::debug!("NetworkLocalController: game ended while waiting for ChoiceRequest");
                    self.disconnected = true;
                    return false;
                }
                Ok(LocalControllerMessage::ChoiceAcknowledged) => {
                    log::trace!(
                        "NetworkLocalController: received stale ChoiceAcknowledged while waiting for ChoiceRequest"
                    );
                    // Continue waiting
                }
                Ok(LocalControllerMessage::Error(e)) => {
                    log::error!(
                        "NetworkLocalController: server error while waiting for ChoiceRequest: {}",
                        e
                    );
                    self.disconnected = true;
                    return false;
                }
                Err(_) => {
                    log::error!("NetworkLocalController: channel closed while waiting for ChoiceRequest");
                    self.disconnected = true;
                    return false;
                }
            }
        }
    }

    /// Send a choice to the server and wait for acknowledgment
    ///
    /// # Arguments
    /// * `choice_index` - The selected choice index
    /// * `description` - Human-readable description of the choice
    /// * `action_count` - Current action count (undo log position) for sync validation
    /// * `last_actions` - Formatted string of last N actions (debug mode only)
    fn send_choice(
        &mut self,
        choice_index: usize,
        description: String,
        action_count: u64,
        last_actions: Option<String>,
    ) -> Result<(), String> {
        if self.disconnected {
            return Err("Disconnected from server".to_string());
        }

        log::trace!(
            "NetworkLocalController: sending choice {} ({}) at action_count={}",
            choice_index,
            description,
            action_count
        );

        // Send choice
        if self
            .choice_tx
            .send(LocalChoice {
                choice_index,
                description,
                action_count,
                last_actions,
            })
            .is_err()
        {
            self.disconnected = true;
            return Err("Failed to send choice to server".to_string());
        }

        // Wait for acknowledgment
        // Note: We loop in case other messages arrive before the ack
        // Track if we receive an early ChoiceRequest for the next choice
        let mut received_early_request = false;
        loop {
            match self.message_rx.recv() {
                Ok(LocalControllerMessage::ChoiceAcknowledged) => {
                    log::trace!("NetworkLocalController: choice acknowledged");
                    // Only clear server state if we didn't receive an early request for the next choice
                    if !received_early_request {
                        self.server_action_count = None;
                        self.server_choice_seq = None;
                    }
                    return Ok(());
                }
                Ok(LocalControllerMessage::Error(e)) => return Err(e),
                Ok(LocalControllerMessage::GameEnded) => {
                    self.disconnected = true;
                    return Err("Game ended".to_string());
                }
                Ok(LocalControllerMessage::CardRevealed { owner, card_id }) => {
                    // Card reveals can come while waiting for acknowledgment
                    log::debug!(
                        "NetworkLocalController: processing card reveal while waiting for ack: {:?} -> {:?}",
                        owner,
                        card_id
                    );
                    // Continue waiting for ack
                }
                Ok(LocalControllerMessage::ChoiceRequest {
                    action_count,
                    choice_seq,
                }) => {
                    // This can happen when the server sends the next ChoiceRequest before we
                    // receive the ChoiceAcknowledged for the previous choice. Store the request
                    // so wait_for_choice_request() can find it.
                    log::debug!(
                        "NetworkLocalController: received early ChoiceRequest seq={} while waiting for ack, storing",
                        choice_seq
                    );
                    self.server_action_count = Some(action_count);
                    self.server_choice_seq = Some(choice_seq);
                    received_early_request = true;
                    // Continue waiting for ack
                }
                Err(_) => {
                    self.disconnected = true;
                    return Err("Lost connection to server".to_string());
                }
            }
        }
    }

    /// Get formatted last N actions from view for network debug mode
    ///
    /// Returns Some(formatted_string) if network_debug is enabled, None otherwise.
    /// Shows the last 20 actions for context when sync errors occur.
    fn get_debug_actions(&self, view: &GameStateView) -> Option<String> {
        if self.network_debug {
            Some(view.format_last_n_actions(20))
        } else {
            None
        }
    }
}

impl<C: PlayerController> PlayerController for NetworkLocalController<C> {
    fn player_id(&self) -> PlayerId {
        self.inner.player_id()
    }

    fn choose_spell_ability_to_play(
        &mut self,
        view: &GameStateView,
        available: &[SpellAbility],
    ) -> ChoiceResult<Option<SpellAbility>> {
        if self.disconnected {
            return ChoiceResult::ExitGame;
        }

        // Wait for ChoiceRequest from server before proceeding
        if !self.wait_for_choice_request() {
            return ChoiceResult::ExitGame;
        }

        // Delegate to inner controller
        let result = self.inner.choose_spell_ability_to_play(view, available);

        // Send choice to server using server's action_count if available
        let action_count = self.server_action_count.unwrap_or(view.action_count() as u64);
        let last_actions = self.get_debug_actions(view);
        match &result {
            ChoiceResult::Ok(Some(ability)) => {
                let idx = available.iter().position(|a| a == ability).unwrap_or(0) + 1;
                let desc = format!("Play: {:?}", ability);
                if let Err(e) = self.send_choice(idx, desc, action_count, last_actions) {
                    log::error!("Failed to send choice: {}", e);
                    return ChoiceResult::ExitGame;
                }
            }
            ChoiceResult::Ok(None) => {
                if let Err(e) = self.send_choice(0, "Pass".to_string(), action_count, last_actions) {
                    log::error!("Failed to send choice: {}", e);
                    return ChoiceResult::ExitGame;
                }
            }
            _ => {}
        }

        result
    }

    fn choose_targets(
        &mut self,
        view: &GameStateView,
        spell: CardId,
        valid_targets: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        if self.disconnected {
            return ChoiceResult::ExitGame;
        }

        if !self.wait_for_choice_request() {
            return ChoiceResult::ExitGame;
        }

        let result = self.inner.choose_targets(view, spell, valid_targets);

        if let ChoiceResult::Ok(targets) = &result {
            let idx = if targets.is_empty() {
                valid_targets.len() // No target
            } else {
                valid_targets.iter().position(|&t| t == targets[0]).unwrap_or(0)
            };
            let desc = format!("Target: {:?}", targets);
            let action_count = self.server_action_count.unwrap_or(view.action_count() as u64);
            let last_actions = self.get_debug_actions(view);
            if let Err(e) = self.send_choice(idx, desc, action_count, last_actions) {
                log::error!("Failed to send choice: {}", e);
                return ChoiceResult::ExitGame;
            }
        }

        result
    }

    fn choose_mana_sources_to_pay(
        &mut self,
        view: &GameStateView,
        cost: &ManaCost,
        available_sources: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        if self.disconnected {
            return ChoiceResult::ExitGame;
        }

        if !self.wait_for_choice_request() {
            return ChoiceResult::ExitGame;
        }

        let result = self.inner.choose_mana_sources_to_pay(view, cost, available_sources);

        if let ChoiceResult::Ok(sources) = &result {
            // For now, just send first source index
            let idx = if sources.is_empty() {
                available_sources.len()
            } else {
                available_sources.iter().position(|&s| s == sources[0]).unwrap_or(0)
            };
            let desc = format!("Mana source: {:?}", sources);
            let action_count = self.server_action_count.unwrap_or(view.action_count() as u64);
            let last_actions = self.get_debug_actions(view);
            if let Err(e) = self.send_choice(idx, desc, action_count, last_actions) {
                log::error!("Failed to send choice: {}", e);
                return ChoiceResult::ExitGame;
            }
        }

        result
    }

    fn choose_attackers(
        &mut self,
        view: &GameStateView,
        available_creatures: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        if self.disconnected {
            return ChoiceResult::ExitGame;
        }

        if !self.wait_for_choice_request() {
            return ChoiceResult::ExitGame;
        }

        let result = self.inner.choose_attackers(view, available_creatures);

        if let ChoiceResult::Ok(attackers) = &result {
            let idx = if attackers.is_empty() {
                0 // No attackers
            } else {
                available_creatures.iter().position(|&a| a == attackers[0]).unwrap_or(0) + 1
            };
            let desc = format!("Attackers: {:?}", attackers);
            let action_count = self.server_action_count.unwrap_or(view.action_count() as u64);
            let last_actions = self.get_debug_actions(view);
            if let Err(e) = self.send_choice(idx, desc, action_count, last_actions) {
                log::error!("Failed to send choice: {}", e);
                return ChoiceResult::ExitGame;
            }
        }

        result
    }

    fn choose_blockers(
        &mut self,
        view: &GameStateView,
        available_blockers: &[CardId],
        attackers: &[CardId],
    ) -> ChoiceResult<SmallVec<[(CardId, CardId); 8]>> {
        if self.disconnected {
            return ChoiceResult::ExitGame;
        }

        if !self.wait_for_choice_request() {
            return ChoiceResult::ExitGame;
        }

        let result = self.inner.choose_blockers(view, available_blockers, attackers);

        if let ChoiceResult::Ok(blocks) = &result {
            let idx = if blocks.is_empty() {
                0 // No blockers
            } else {
                // Encode as blocker_idx * num_attackers + attacker_idx + 1
                let (blocker, attacker) = blocks[0];
                let blocker_idx = available_blockers.iter().position(|&b| b == blocker).unwrap_or(0);
                let attacker_idx = attackers.iter().position(|&a| a == attacker).unwrap_or(0);
                blocker_idx * attackers.len() + attacker_idx + 1
            };
            let desc = format!("Blocks: {:?}", blocks);
            let action_count = self.server_action_count.unwrap_or(view.action_count() as u64);
            let last_actions = self.get_debug_actions(view);
            if let Err(e) = self.send_choice(idx, desc, action_count, last_actions) {
                log::error!("Failed to send choice: {}", e);
                return ChoiceResult::ExitGame;
            }
        }

        result
    }

    fn choose_damage_assignment_order(
        &mut self,
        view: &GameStateView,
        attacker: CardId,
        blockers: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        if self.disconnected {
            return ChoiceResult::ExitGame;
        }

        if !self.wait_for_choice_request() {
            return ChoiceResult::ExitGame;
        }

        let result = self.inner.choose_damage_assignment_order(view, attacker, blockers);

        if let ChoiceResult::Ok(order) = &result {
            let idx = if order.is_empty() {
                0
            } else {
                blockers.iter().position(|&b| b == order[0]).unwrap_or(0)
            };
            let desc = format!("Damage order: {:?}", order);
            let action_count = self.server_action_count.unwrap_or(view.action_count() as u64);
            let last_actions = self.get_debug_actions(view);
            if let Err(e) = self.send_choice(idx, desc, action_count, last_actions) {
                log::error!("Failed to send choice: {}", e);
                return ChoiceResult::ExitGame;
            }
        }

        result
    }

    fn choose_cards_to_discard(
        &mut self,
        view: &GameStateView,
        hand: &[CardId],
        count: usize,
    ) -> ChoiceResult<SmallVec<[CardId; 7]>> {
        if self.disconnected {
            return ChoiceResult::ExitGame;
        }

        if !self.wait_for_choice_request() {
            return ChoiceResult::ExitGame;
        }

        let result = self.inner.choose_cards_to_discard(view, hand, count);

        if let ChoiceResult::Ok(discards) = &result {
            let idx = if discards.is_empty() {
                hand.len()
            } else {
                hand.iter().position(|&c| c == discards[0]).unwrap_or(0)
            };
            let desc = format!("Discard: {:?}", discards);
            let action_count = self.server_action_count.unwrap_or(view.action_count() as u64);
            let last_actions = self.get_debug_actions(view);
            if let Err(e) = self.send_choice(idx, desc, action_count, last_actions) {
                log::error!("Failed to send choice: {}", e);
                return ChoiceResult::ExitGame;
            }
        }

        result
    }

    fn choose_from_library(&mut self, view: &GameStateView, valid_cards: &[CardId]) -> ChoiceResult<Option<CardId>> {
        if self.disconnected {
            return ChoiceResult::ExitGame;
        }

        if !self.wait_for_choice_request() {
            return ChoiceResult::ExitGame;
        }

        let result = self.inner.choose_from_library(view, valid_cards);

        if let ChoiceResult::Ok(card) = &result {
            let idx = match card {
                Some(c) => valid_cards.iter().position(|&v| v == *c).unwrap_or(valid_cards.len()),
                None => valid_cards.len(),
            };
            let desc = format!("From library: {:?}", card);
            let action_count = self.server_action_count.unwrap_or(view.action_count() as u64);
            let last_actions = self.get_debug_actions(view);
            if let Err(e) = self.send_choice(idx, desc, action_count, last_actions) {
                log::error!("Failed to send choice: {}", e);
                return ChoiceResult::ExitGame;
            }
        }

        result
    }

    fn choose_permanents_to_sacrifice(
        &mut self,
        view: &GameStateView,
        valid_permanents: &[CardId],
        count: usize,
        card_type_description: &str,
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        // Note: Reveal processing is handled by drain_reveals callback in GameLoop
        if self.disconnected {
            return ChoiceResult::ExitGame;
        }

        let result = self
            .inner
            .choose_permanents_to_sacrifice(view, valid_permanents, count, card_type_description);

        if let ChoiceResult::Ok(permanents) = &result {
            // Encode as first permanent index for now
            // TODO: Multi-select encoding for multiple permanents
            let idx = if permanents.is_empty() {
                valid_permanents.len()
            } else {
                valid_permanents.iter().position(|&p| p == permanents[0]).unwrap_or(0)
            };
            let desc = format!("Sacrifice: {:?}", permanents);
            let action_count = view.action_count() as u64;
            let last_actions = self.get_debug_actions(view);
            if let Err(e) = self.send_choice(idx, desc, action_count, last_actions) {
                log::error!("Failed to send choice: {}", e);
                return ChoiceResult::ExitGame;
            }
        }

        result
    }

    fn on_priority_passed(&mut self, view: &GameStateView) {
        self.inner.on_priority_passed(view);
    }

    fn on_game_end(&mut self, view: &GameStateView, won: bool) {
        self.inner.on_game_end(view, won);
    }

    fn has_more_choices(&self) -> bool {
        self.inner.has_more_choices()
    }

    fn choose_from_options(&mut self, options: &[String]) -> usize {
        self.inner.choose_from_options(options)
    }

    fn get_controller_type(&self) -> ControllerType {
        self.inner.get_controller_type()
    }
}
