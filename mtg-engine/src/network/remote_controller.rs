//! Remote controller for network clients
//!
//! This controller represents the opponent from the client's perspective.
//! It receives opponent choices from the server and returns them to the GameLoop,
//! allowing the client to stay in sync with the authoritative server state.
//!
//! ## Architecture
//!
//! ```text
//! Server                     Network                   Client
//! ──────                     ───────                   ──────
//! GameLoop                                             GameLoop
//!   │                                                    │
//!   ├─► NetworkController ──► OpponentChoice ──►  RemoteController
//!   │   (sends choice to                          (receives choice,
//!   │    other client)                             returns to GameLoop)
//! ```
//!
//! The RemoteController blocks waiting for an OpponentChoice message from the server,
//! then returns the choice index. The client's GameLoop then applies this choice
//! to its shadow state, keeping both client and server in sync.

use crate::core::{CardId, ManaCost, PlayerId, SpellAbility};
use crate::game::controller::{ChoiceResult, GameStateView, PlayerController};
use crate::game::snapshot::ControllerType;
use smallvec::SmallVec;
use std::sync::mpsc;

/// A message received from the server for the opponent controller.
///
/// This can be either an actual choice from the opponent, or a signal
/// that the game has ended (allowing graceful shutdown without treating
/// channel close as a disconnect error).
#[derive(Debug, Clone)]
pub enum RemoteMessage {
    /// An actual choice from the opponent
    Choice {
        /// The choice index selected by the opponent
        choice_index: usize,
        /// Human-readable description of the choice
        description: String,
    },
    /// Signal that the game has ended normally
    ///
    /// This allows the RemoteController to exit gracefully without
    /// logging a disconnect warning or treating it as an error.
    GameEnded,
}

/// Legacy type alias for backward compatibility
pub type RemoteChoice = RemoteMessage;

/// A controller that receives opponent choices from the network.
///
/// This is used on the client side to represent the opponent. When the GameLoop
/// asks this controller for a choice, it blocks waiting for the server to send
/// an `OpponentChoice` message via the channel, then returns that choice.
pub struct RemoteController {
    player_id: PlayerId,
    /// Receiver for opponent choices from the WebSocket handler
    choice_rx: mpsc::Receiver<RemoteMessage>,
    /// Whether we've been disconnected from the server
    disconnected: bool,
    /// Whether the game has ended normally (not a disconnect)
    game_ended: bool,
}

impl RemoteController {
    /// Create a new remote controller
    ///
    /// # Arguments
    /// * `player_id` - The player ID this controller represents (the opponent)
    /// * `choice_rx` - Channel to receive opponent choices from the server
    pub fn new(player_id: PlayerId, choice_rx: mpsc::Receiver<RemoteMessage>) -> Self {
        Self {
            player_id,
            choice_rx,
            disconnected: false,
            game_ended: false,
        }
    }

    /// Wait for the next choice from the server
    ///
    /// Returns the choice index, or signals disconnect if channel is closed.
    fn wait_for_choice(&mut self) -> ChoiceResult<usize> {
        if self.disconnected || self.game_ended {
            return ChoiceResult::ExitGame;
        }

        log::trace!("RemoteController {:?}: waiting for opponent choice", self.player_id);
        match self.choice_rx.recv() {
            Ok(RemoteMessage::Choice {
                choice_index,
                description,
            }) => {
                log::debug!(
                    "RemoteController {:?}: Opponent chose index {} ({})",
                    self.player_id,
                    choice_index,
                    description
                );
                ChoiceResult::Ok(choice_index)
            }
            Ok(RemoteMessage::GameEnded) => {
                log::debug!("RemoteController: Received game end signal, exiting gracefully");
                self.game_ended = true;
                ChoiceResult::ExitGame
            }
            Err(_) => {
                // Channel closed without GameEnded signal - this is an unexpected disconnect
                if !self.game_ended {
                    log::warn!("RemoteController: Channel closed unexpectedly, opponent disconnected");
                }
                self.disconnected = true;
                ChoiceResult::ExitGame
            }
        }
    }

    /// Helper to get a single item from a slice based on choice index
    fn select_from_slice<T: Clone>(&mut self, items: &[T]) -> ChoiceResult<Option<T>> {
        match self.wait_for_choice() {
            ChoiceResult::Ok(idx) => {
                if idx < items.len() {
                    ChoiceResult::Ok(Some(items[idx].clone()))
                } else if idx == items.len() {
                    // Index == len typically means "pass" or "none"
                    ChoiceResult::Ok(None)
                } else {
                    log::warn!(
                        "RemoteController: Invalid choice index {} for {} items",
                        idx,
                        items.len()
                    );
                    ChoiceResult::Ok(None)
                }
            }
            ChoiceResult::ExitGame => ChoiceResult::ExitGame,
            ChoiceResult::Error(e) => ChoiceResult::Error(e),
            ChoiceResult::UndoRequest(_) => {
                // Undo is not supported for network games
                ChoiceResult::Error("Undo not supported in network games".to_string())
            }
            ChoiceResult::NeedInput(_) => {
                // NeedInput is not possible from wait_for_choice
                ChoiceResult::Error("NeedInput not supported in network games".to_string())
            }
        }
    }
}

impl PlayerController for RemoteController {
    fn player_id(&self) -> PlayerId {
        self.player_id
    }

    fn choose_spell_ability_to_play(
        &mut self,
        _view: &GameStateView,
        available: &[SpellAbility],
    ) -> ChoiceResult<Option<SpellAbility>> {
        // Server sends: 0 = pass, 1..N = ability indices
        match self.wait_for_choice() {
            ChoiceResult::Ok(0) => ChoiceResult::Ok(None), // Pass
            ChoiceResult::Ok(idx) => {
                let ability_idx = idx - 1;
                if ability_idx < available.len() {
                    ChoiceResult::Ok(Some(available[ability_idx].clone()))
                } else {
                    log::warn!("RemoteController: Invalid ability index {}", ability_idx);
                    ChoiceResult::Ok(None)
                }
            }
            ChoiceResult::ExitGame => ChoiceResult::ExitGame,
            ChoiceResult::Error(e) => ChoiceResult::Error(e),
            ChoiceResult::UndoRequest(_) => ChoiceResult::Error("Undo not supported in network games".to_string()),
            ChoiceResult::NeedInput(_) => ChoiceResult::Error("NeedInput not supported in network games".to_string()),
        }
    }

    fn choose_targets(
        &mut self,
        _view: &GameStateView,
        _spell: CardId,
        valid_targets: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        // For now, single target selection
        // Server sends index into valid_targets, or len() for no target
        match self.wait_for_choice() {
            ChoiceResult::Ok(idx) => {
                if idx < valid_targets.len() {
                    ChoiceResult::Ok(SmallVec::from_slice(&[valid_targets[idx]]))
                } else {
                    ChoiceResult::Ok(SmallVec::new()) // No target
                }
            }
            ChoiceResult::ExitGame => ChoiceResult::ExitGame,
            ChoiceResult::Error(e) => ChoiceResult::Error(e),
            ChoiceResult::UndoRequest(_) => ChoiceResult::Error("Undo not supported in network games".to_string()),
            ChoiceResult::NeedInput(_) => ChoiceResult::Error("NeedInput not supported in network games".to_string()),
        }
    }

    fn choose_mana_sources_to_pay(
        &mut self,
        _view: &GameStateView,
        _cost: &ManaCost,
        available_sources: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        // Simplified: server sends single source index
        // TODO: Multi-select for complex costs
        match self.wait_for_choice() {
            ChoiceResult::Ok(idx) => {
                if idx < available_sources.len() {
                    ChoiceResult::Ok(SmallVec::from_slice(&[available_sources[idx]]))
                } else {
                    ChoiceResult::Ok(SmallVec::new())
                }
            }
            ChoiceResult::ExitGame => ChoiceResult::ExitGame,
            ChoiceResult::Error(e) => ChoiceResult::Error(e),
            ChoiceResult::UndoRequest(_) => ChoiceResult::Error("Undo not supported in network games".to_string()),
            ChoiceResult::NeedInput(_) => ChoiceResult::Error("NeedInput not supported in network games".to_string()),
        }
    }

    fn choose_attackers(
        &mut self,
        _view: &GameStateView,
        available_creatures: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        // Server sends: 0 = done/no attackers, 1..N = creature indices
        // TODO: Multi-select for multiple attackers
        match self.wait_for_choice() {
            ChoiceResult::Ok(0) => ChoiceResult::Ok(SmallVec::new()),
            ChoiceResult::Ok(idx) => {
                let creature_idx = idx - 1;
                if creature_idx < available_creatures.len() {
                    ChoiceResult::Ok(SmallVec::from_slice(&[available_creatures[creature_idx]]))
                } else {
                    ChoiceResult::Ok(SmallVec::new())
                }
            }
            ChoiceResult::ExitGame => ChoiceResult::ExitGame,
            ChoiceResult::Error(e) => ChoiceResult::Error(e),
            ChoiceResult::UndoRequest(_) => ChoiceResult::Error("Undo not supported in network games".to_string()),
            ChoiceResult::NeedInput(_) => ChoiceResult::Error("NeedInput not supported in network games".to_string()),
        }
    }

    fn choose_blockers(
        &mut self,
        _view: &GameStateView,
        available_blockers: &[CardId],
        attackers: &[CardId],
    ) -> ChoiceResult<SmallVec<[(CardId, CardId); 8]>> {
        // Server sends: 0 = done/no blockers, 1..N = encoded blocker-attacker pair
        // TODO: Multi-select for multiple blockers
        match self.wait_for_choice() {
            ChoiceResult::Ok(0) => ChoiceResult::Ok(SmallVec::new()),
            ChoiceResult::Ok(idx) => {
                let pair_idx = idx - 1;
                let blocker_idx = pair_idx / attackers.len();
                let attacker_idx = pair_idx % attackers.len();
                if blocker_idx < available_blockers.len() && attacker_idx < attackers.len() {
                    ChoiceResult::Ok(SmallVec::from_slice(&[(
                        available_blockers[blocker_idx],
                        attackers[attacker_idx],
                    )]))
                } else {
                    ChoiceResult::Ok(SmallVec::new())
                }
            }
            ChoiceResult::ExitGame => ChoiceResult::ExitGame,
            ChoiceResult::Error(e) => ChoiceResult::Error(e),
            ChoiceResult::UndoRequest(_) => ChoiceResult::Error("Undo not supported in network games".to_string()),
            ChoiceResult::NeedInput(_) => ChoiceResult::Error("NeedInput not supported in network games".to_string()),
        }
    }

    fn choose_damage_assignment_order(
        &mut self,
        _view: &GameStateView,
        _attacker: CardId,
        blockers: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        // Server sends index of first blocker to assign damage to
        match self.wait_for_choice() {
            ChoiceResult::Ok(idx) => {
                if idx < blockers.len() {
                    // Return all blockers with chosen one first
                    let mut result = SmallVec::new();
                    result.push(blockers[idx]);
                    for (i, &blocker) in blockers.iter().enumerate() {
                        if i != idx {
                            result.push(blocker);
                        }
                    }
                    ChoiceResult::Ok(result)
                } else {
                    ChoiceResult::Ok(blockers.iter().copied().collect())
                }
            }
            ChoiceResult::ExitGame => ChoiceResult::ExitGame,
            ChoiceResult::Error(e) => ChoiceResult::Error(e),
            ChoiceResult::UndoRequest(_) => ChoiceResult::Error("Undo not supported in network games".to_string()),
            ChoiceResult::NeedInput(_) => ChoiceResult::Error("NeedInput not supported in network games".to_string()),
        }
    }

    fn choose_cards_to_discard(
        &mut self,
        _view: &GameStateView,
        hand: &[CardId],
        _count: usize,
    ) -> ChoiceResult<SmallVec<[CardId; 7]>> {
        // Server sends index of card to discard
        // TODO: Multi-select for discarding multiple cards
        match self.wait_for_choice() {
            ChoiceResult::Ok(idx) => {
                if idx < hand.len() {
                    ChoiceResult::Ok(SmallVec::from_slice(&[hand[idx]]))
                } else {
                    ChoiceResult::Ok(SmallVec::new())
                }
            }
            ChoiceResult::ExitGame => ChoiceResult::ExitGame,
            ChoiceResult::Error(e) => ChoiceResult::Error(e),
            ChoiceResult::UndoRequest(_) => ChoiceResult::Error("Undo not supported in network games".to_string()),
            ChoiceResult::NeedInput(_) => ChoiceResult::Error("NeedInput not supported in network games".to_string()),
        }
    }

    fn choose_from_library(&mut self, _view: &GameStateView, valid_cards: &[CardId]) -> ChoiceResult<Option<CardId>> {
        self.select_from_slice(valid_cards)
    }

    fn choose_permanents_to_sacrifice(
        &mut self,
        _view: &GameStateView,
        valid_permanents: &[CardId],
        _count: usize,
        _card_type_description: &str,
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        // Server sends index of permanent to sacrifice
        // TODO: Multi-select for sacrificing multiple permanents
        match self.wait_for_choice() {
            ChoiceResult::Ok(idx) => {
                if idx < valid_permanents.len() {
                    ChoiceResult::Ok(SmallVec::from_slice(&[valid_permanents[idx]]))
                } else {
                    ChoiceResult::Ok(SmallVec::new())
                }
            }
            ChoiceResult::ExitGame => ChoiceResult::ExitGame,
            ChoiceResult::Error(e) => ChoiceResult::Error(e),
            ChoiceResult::UndoRequest(_) => ChoiceResult::Error("Undo not supported in network games".to_string()),
            ChoiceResult::NeedInput(_) => ChoiceResult::Error("NeedInput not supported in network games".to_string()),
        }
    }

    fn on_priority_passed(&mut self, _view: &GameStateView) {
        // Nothing to do
    }

    fn on_game_end(&mut self, _view: &GameStateView, _won: bool) {
        // Nothing to do
    }

    fn get_controller_type(&self) -> ControllerType {
        // FIXME-UNFINISHED: Add ControllerType::Remote variant
        ControllerType::Zero
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::EntityId;

    #[test]
    fn test_remote_controller_creation() {
        let (tx, rx) = mpsc::channel();
        let controller = RemoteController::new(EntityId::new(1), rx);
        assert_eq!(controller.player_id(), EntityId::new(1));
        drop(tx); // Avoid unused warning
    }

    #[test]
    fn test_remote_controller_receives_choice() {
        let (tx, rx) = mpsc::channel();
        let mut controller = RemoteController::new(EntityId::new(1), rx);

        // Send a choice
        tx.send(RemoteMessage::Choice {
            choice_index: 2,
            description: "Cast Lightning Bolt".to_string(),
        })
        .unwrap();

        // Controller should receive it
        let result = controller.wait_for_choice();
        assert!(matches!(result, ChoiceResult::Ok(2)));
    }

    #[test]
    fn test_remote_controller_game_ended() {
        let (tx, rx) = mpsc::channel();
        let mut controller = RemoteController::new(EntityId::new(1), rx);

        // Send game ended signal
        tx.send(RemoteMessage::GameEnded).unwrap();

        // Controller should exit gracefully
        let result = controller.wait_for_choice();
        assert!(matches!(result, ChoiceResult::ExitGame));
        assert!(controller.game_ended);
        assert!(!controller.disconnected);
    }

    #[test]
    fn test_remote_controller_disconnect() {
        let (tx, rx) = mpsc::channel();
        let mut controller = RemoteController::new(EntityId::new(1), rx);

        // Drop the sender to simulate disconnect
        drop(tx);

        // Controller should detect disconnect
        let result = controller.wait_for_choice();
        assert!(matches!(result, ChoiceResult::ExitGame));
    }
}
