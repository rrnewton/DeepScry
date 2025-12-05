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
}

/// Message types for the local controller
#[derive(Debug)]
pub enum LocalControllerMessage {
    /// A card was revealed by the server (queue for drawing)
    CardRevealed {
        owner: PlayerId,
        card_id: CardId,
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
/// we delegate to the inner controller, then send the result to the server.
pub struct NetworkLocalController<C: PlayerController> {
    /// The wrapped local controller
    inner: C,
    /// Channel to send our choices to the WebSocket handler
    choice_tx: mpsc::Sender<LocalChoice>,
    /// Channel to receive messages from the WebSocket handler
    message_rx: mpsc::Receiver<LocalControllerMessage>,
    /// Whether we've been disconnected
    disconnected: bool,
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
        }
    }

    /// Send a choice to the server and wait for acknowledgment
    fn send_choice(&mut self, choice_index: usize, description: String) -> Result<(), String> {
        if self.disconnected {
            return Err("Disconnected from server".to_string());
        }

        // Send choice
        if self.choice_tx.send(LocalChoice { choice_index, description }).is_err() {
            self.disconnected = true;
            return Err("Failed to send choice to server".to_string());
        }

        // Wait for acknowledgment
        match self.message_rx.recv() {
            Ok(LocalControllerMessage::ChoiceAcknowledged) => Ok(()),
            Ok(LocalControllerMessage::Error(e)) => Err(e),
            Ok(LocalControllerMessage::GameEnded) => {
                self.disconnected = true;
                Err("Game ended".to_string())
            }
            Ok(LocalControllerMessage::CardRevealed { .. }) => {
                // Card reveals should be processed before choice requests
                // If we get one here, something is out of sync
                log::warn!("Unexpected CardRevealed during choice acknowledgment");
                Ok(())
            }
            Err(_) => {
                self.disconnected = true;
                Err("Lost connection to server".to_string())
            }
        }
    }

    /// Process any pending CardRevealed messages before a choice
    ///
    /// This is called before delegating to the inner controller to ensure
    /// the game state is up-to-date with revealed cards.
    fn process_pending_reveals(&mut self) {
        // Non-blocking check for any pending reveals
        while let Ok(msg) = self.message_rx.try_recv() {
            match msg {
                LocalControllerMessage::CardRevealed { owner, card_id } => {
                    log::debug!("Processing card reveal for {:?}: {:?}", owner, card_id);
                    // The reveal should have been processed by the game state already
                    // This is just for logging/debugging
                }
                LocalControllerMessage::GameEnded => {
                    self.disconnected = true;
                    break;
                }
                _ => {
                    // Other messages will be handled later
                    log::warn!("Unexpected message while processing reveals: {:?}", msg);
                }
            }
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
        self.process_pending_reveals();

        if self.disconnected {
            return ChoiceResult::ExitGame;
        }

        // Delegate to inner controller
        let result = self.inner.choose_spell_ability_to_play(view, available);

        // Send choice to server
        match &result {
            ChoiceResult::Ok(Some(ability)) => {
                let idx = available.iter().position(|a| a == ability).unwrap_or(0) + 1;
                let desc = format!("Play: {:?}", ability);
                if let Err(e) = self.send_choice(idx, desc) {
                    log::error!("Failed to send choice: {}", e);
                    return ChoiceResult::ExitGame;
                }
            }
            ChoiceResult::Ok(None) => {
                if let Err(e) = self.send_choice(0, "Pass".to_string()) {
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
        self.process_pending_reveals();

        if self.disconnected {
            return ChoiceResult::ExitGame;
        }

        let result = self.inner.choose_targets(view, spell, valid_targets);

        match &result {
            ChoiceResult::Ok(targets) => {
                let idx = if targets.is_empty() {
                    valid_targets.len() // No target
                } else {
                    valid_targets.iter().position(|&t| t == targets[0]).unwrap_or(0)
                };
                let desc = format!("Target: {:?}", targets);
                if let Err(e) = self.send_choice(idx, desc) {
                    log::error!("Failed to send choice: {}", e);
                    return ChoiceResult::ExitGame;
                }
            }
            _ => {}
        }

        result
    }

    fn choose_mana_sources_to_pay(
        &mut self,
        view: &GameStateView,
        cost: &ManaCost,
        available_sources: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        self.process_pending_reveals();

        if self.disconnected {
            return ChoiceResult::ExitGame;
        }

        let result = self.inner.choose_mana_sources_to_pay(view, cost, available_sources);

        match &result {
            ChoiceResult::Ok(sources) => {
                // For now, just send first source index
                let idx = if sources.is_empty() {
                    available_sources.len()
                } else {
                    available_sources.iter().position(|&s| s == sources[0]).unwrap_or(0)
                };
                let desc = format!("Mana source: {:?}", sources);
                if let Err(e) = self.send_choice(idx, desc) {
                    log::error!("Failed to send choice: {}", e);
                    return ChoiceResult::ExitGame;
                }
            }
            _ => {}
        }

        result
    }

    fn choose_attackers(
        &mut self,
        view: &GameStateView,
        available_creatures: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        self.process_pending_reveals();

        if self.disconnected {
            return ChoiceResult::ExitGame;
        }

        let result = self.inner.choose_attackers(view, available_creatures);

        match &result {
            ChoiceResult::Ok(attackers) => {
                let idx = if attackers.is_empty() {
                    0 // No attackers
                } else {
                    available_creatures.iter().position(|&a| a == attackers[0]).unwrap_or(0) + 1
                };
                let desc = format!("Attackers: {:?}", attackers);
                if let Err(e) = self.send_choice(idx, desc) {
                    log::error!("Failed to send choice: {}", e);
                    return ChoiceResult::ExitGame;
                }
            }
            _ => {}
        }

        result
    }

    fn choose_blockers(
        &mut self,
        view: &GameStateView,
        available_blockers: &[CardId],
        attackers: &[CardId],
    ) -> ChoiceResult<SmallVec<[(CardId, CardId); 8]>> {
        self.process_pending_reveals();

        if self.disconnected {
            return ChoiceResult::ExitGame;
        }

        let result = self.inner.choose_blockers(view, available_blockers, attackers);

        match &result {
            ChoiceResult::Ok(blocks) => {
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
                if let Err(e) = self.send_choice(idx, desc) {
                    log::error!("Failed to send choice: {}", e);
                    return ChoiceResult::ExitGame;
                }
            }
            _ => {}
        }

        result
    }

    fn choose_damage_assignment_order(
        &mut self,
        view: &GameStateView,
        attacker: CardId,
        blockers: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        self.process_pending_reveals();

        if self.disconnected {
            return ChoiceResult::ExitGame;
        }

        let result = self.inner.choose_damage_assignment_order(view, attacker, blockers);

        match &result {
            ChoiceResult::Ok(order) => {
                let idx = if order.is_empty() {
                    0
                } else {
                    blockers.iter().position(|&b| b == order[0]).unwrap_or(0)
                };
                let desc = format!("Damage order: {:?}", order);
                if let Err(e) = self.send_choice(idx, desc) {
                    log::error!("Failed to send choice: {}", e);
                    return ChoiceResult::ExitGame;
                }
            }
            _ => {}
        }

        result
    }

    fn choose_cards_to_discard(
        &mut self,
        view: &GameStateView,
        hand: &[CardId],
        count: usize,
    ) -> ChoiceResult<SmallVec<[CardId; 7]>> {
        self.process_pending_reveals();

        if self.disconnected {
            return ChoiceResult::ExitGame;
        }

        let result = self.inner.choose_cards_to_discard(view, hand, count);

        match &result {
            ChoiceResult::Ok(discards) => {
                let idx = if discards.is_empty() {
                    hand.len()
                } else {
                    hand.iter().position(|&c| c == discards[0]).unwrap_or(0)
                };
                let desc = format!("Discard: {:?}", discards);
                if let Err(e) = self.send_choice(idx, desc) {
                    log::error!("Failed to send choice: {}", e);
                    return ChoiceResult::ExitGame;
                }
            }
            _ => {}
        }

        result
    }

    fn choose_from_library(
        &mut self,
        view: &GameStateView,
        valid_cards: &[CardId],
    ) -> ChoiceResult<Option<CardId>> {
        self.process_pending_reveals();

        if self.disconnected {
            return ChoiceResult::ExitGame;
        }

        let result = self.inner.choose_from_library(view, valid_cards);

        match &result {
            ChoiceResult::Ok(card) => {
                let idx = match card {
                    Some(c) => valid_cards.iter().position(|&v| v == *c).unwrap_or(valid_cards.len()),
                    None => valid_cards.len(),
                };
                let desc = format!("From library: {:?}", card);
                if let Err(e) = self.send_choice(idx, desc) {
                    log::error!("Failed to send choice: {}", e);
                    return ChoiceResult::ExitGame;
                }
            }
            _ => {}
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
