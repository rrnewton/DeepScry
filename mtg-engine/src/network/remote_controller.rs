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
use crate::network::local_controller::BundledReveal;
use crate::network::protocol::CardReveal;
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
        /// The choice indices selected by the opponent (multiple for attackers/blockers)
        choice_indices: Vec<usize>,
        /// Human-readable description of the choice
        description: String,
        /// The actual spell ability (for Priority choices)
        /// When present, RemoteController can return this directly instead
        /// of looking up by index in the local available list
        spell_ability: Option<SpellAbility>,
        /// Card reveal for the spell being cast (for hidden hand cards)
        /// When opponent casts a spell from their hand that we don't know about,
        /// this contains the card reveal info so we can instantiate it.
        card_reveal: Option<(PlayerId, CardReveal)>,
        /// Bundled reveals that arrived before this choice from the server.
        /// These MUST be processed before the choice is applied to ensure
        /// cards are instantiated in time. This eliminates race conditions
        /// from having separate reveal/choice channels.
        reveals: Vec<BundledReveal>,
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
///
/// RevealMsg is imported from local_controller to avoid duplicate type alias.
use crate::network::local_controller::RevealMsg;

pub struct RemoteController {
    player_id: PlayerId,
    /// Receiver for opponent choices from the WebSocket handler
    choice_rx: mpsc::Receiver<RemoteMessage>,
    /// Channel to send bundled reveals to the game thread for processing
    /// This is used to push reveals received with Choice messages so drain_reveals() can process them
    reveal_tx: Option<mpsc::Sender<RevealMsg>>,
    /// Whether we've been disconnected from the server
    disconnected: bool,
    /// Whether the game has ended normally (not a disconnect)
    game_ended: bool,
    /// Last received spell ability (from Priority choices)
    last_spell_ability: Option<SpellAbility>,
    /// Last received card reveal (for instantiating hidden hand cards)
    last_card_reveal: Option<(PlayerId, CardReveal)>,
    /// Bundled reveals from the last received Choice message.
    /// These should be processed by the caller before proceeding.
    pending_reveals: Vec<BundledReveal>,
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
            reveal_tx: None,
            disconnected: false,
            game_ended: false,
            last_spell_ability: None,
            last_card_reveal: None,
            pending_reveals: Vec::new(),
        }
    }

    /// Set the reveal channel for pushing bundled reveals to the game thread.
    /// This channel is used to send reveals that arrive with Choice messages
    /// so that drain_reveals() can process them before evaluating available options.
    pub fn with_reveal_tx(mut self, reveal_tx: mpsc::Sender<RevealMsg>) -> Self {
        self.reveal_tx = Some(reveal_tx);
        self
    }

    /// Take and clear pending reveals that arrived with the last Choice message.
    /// The caller should process these to instantiate cards before proceeding.
    pub fn take_pending_reveals(&mut self) -> Vec<BundledReveal> {
        std::mem::take(&mut self.pending_reveals)
    }

    /// Push bundled reveals to the reveal channel for processing by drain_reveals().
    /// This is called after receiving a Choice message with bundled reveals.
    fn push_pending_reveals(&mut self) {
        if let Some(ref reveal_tx) = self.reveal_tx {
            for reveal in self.pending_reveals.drain(..) {
                let card_reveal = CardReveal {
                    card_id: reveal.card_id,
                    name: reveal.card_name,
                };
                if let Err(e) = reveal_tx.send((reveal.owner, card_reveal, reveal.reason)) {
                    log::error!("Failed to send bundled reveal to game thread: {:?}", e);
                }
            }
        } else {
            // No reveal channel - just clear the reveals (they won't be processed)
            if !self.pending_reveals.is_empty() {
                log::warn!(
                    "RemoteController: {} bundled reveals dropped (no reveal_tx channel)",
                    self.pending_reveals.len()
                );
                self.pending_reveals.clear();
            }
        }
    }

    /// Get and clear the last card reveal (for instantiating hidden hand cards)
    pub fn take_card_reveal(&mut self) -> Option<(PlayerId, CardReveal)> {
        self.last_card_reveal.take()
    }

    /// Wait for the next choice from the server
    ///
    /// Returns the choice indices, or signals disconnect if channel is closed.
    /// Also stores any spell_ability and card_reveal for use by choose_spell_ability_to_play.
    fn wait_for_choice(&mut self) -> ChoiceResult<Vec<usize>> {
        if self.disconnected || self.game_ended {
            return ChoiceResult::ExitGame;
        }

        log::trace!("RemoteController {:?}: waiting for opponent choice", self.player_id);
        match self.choice_rx.recv() {
            Ok(RemoteMessage::Choice {
                choice_indices,
                description,
                spell_ability,
                card_reveal,
                reveals,
            }) => {
                log::debug!(
                    "RemoteController {:?}: Opponent chose indices {:?} ({}) spell_ability={:?} card_reveal={:?} bundled_reveals={}",
                    self.player_id,
                    choice_indices,
                    description,
                    spell_ability,
                    card_reveal.as_ref().map(|(owner, reveal)| (owner, &reveal.name)),
                    reveals.len()
                );
                // Store spell_ability for choose_spell_ability_to_play to use
                self.last_spell_ability = spell_ability;
                // Store card_reveal for instantiating hidden hand cards
                self.last_card_reveal = card_reveal;
                // Store bundled reveals and push to channel for drain_reveals() to process
                self.pending_reveals = reveals;
                self.push_pending_reveals();
                ChoiceResult::Ok(choice_indices)
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

    /// Helper to get a single item from a slice based on choice indices (uses first index)
    fn select_from_slice<T: Clone>(&mut self, items: &[T]) -> ChoiceResult<Option<T>> {
        match self.wait_for_choice() {
            ChoiceResult::Ok(indices) => {
                let idx = indices.first().copied().unwrap_or(items.len());
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
        // Server sends: [0] = pass, [N] = ability index (1-based)
        // For remote controllers, we may receive the actual ability directly
        match self.wait_for_choice() {
            ChoiceResult::Ok(indices) => {
                let idx = indices.first().copied().unwrap_or(0);
                if idx == 0 {
                    return ChoiceResult::Ok(None); // Pass
                }

                // If server sent the actual spell ability, use it directly
                // This handles the case where client doesn't know opponent's hand
                if let Some(ability) = self.last_spell_ability.take() {
                    log::debug!("RemoteController: Using server-provided spell ability: {:?}", ability);
                    return ChoiceResult::Ok(Some(ability));
                }

                // Fall back to index-based lookup
                let ability_idx = idx - 1;
                if ability_idx < available.len() {
                    ChoiceResult::Ok(Some(available[ability_idx].clone()))
                } else {
                    log::warn!(
                        "RemoteController: Invalid ability index {} (available={}, spell_ability was None)",
                        ability_idx,
                        available.len()
                    );
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
        // Server sends indices into valid_targets, or [len()] for no target
        match self.wait_for_choice() {
            ChoiceResult::Ok(indices) => {
                let mut targets = SmallVec::new();
                for idx in indices {
                    if idx < valid_targets.len() {
                        targets.push(valid_targets[idx]);
                    }
                }
                ChoiceResult::Ok(targets)
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
        // Server sends indices of mana sources to use
        match self.wait_for_choice() {
            ChoiceResult::Ok(indices) => {
                let mut sources = SmallVec::new();
                for idx in indices {
                    if idx < available_sources.len() {
                        sources.push(available_sources[idx]);
                    }
                }
                ChoiceResult::Ok(sources)
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
        // Server sends indices: [0] = no attackers, [N, M, ...] = creature indices (1-based)
        match self.wait_for_choice() {
            ChoiceResult::Ok(indices) => {
                let mut attackers = SmallVec::new();
                for idx in indices {
                    if idx == 0 {
                        // 0 means "done selecting" - skip
                        continue;
                    }
                    let creature_idx = idx - 1;
                    if creature_idx < available_creatures.len() {
                        attackers.push(available_creatures[creature_idx]);
                    }
                }
                ChoiceResult::Ok(attackers)
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
        // Server sends indices: [0] = no blockers, [N, M, ...] = encoded blocker-attacker pairs (1-based)
        match self.wait_for_choice() {
            ChoiceResult::Ok(indices) => {
                let mut blocks = SmallVec::new();
                for idx in indices {
                    if idx == 0 {
                        // 0 means "done selecting" - skip
                        continue;
                    }
                    let pair_idx = idx - 1;
                    let blocker_idx = pair_idx / attackers.len();
                    let attacker_idx = pair_idx % attackers.len();
                    if blocker_idx < available_blockers.len() && attacker_idx < attackers.len() {
                        blocks.push((available_blockers[blocker_idx], attackers[attacker_idx]));
                    }
                }
                ChoiceResult::Ok(blocks)
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
        // Server sends indices specifying the damage assignment order
        match self.wait_for_choice() {
            ChoiceResult::Ok(indices) => {
                let mut result = SmallVec::new();
                for idx in indices {
                    if idx < blockers.len() {
                        result.push(blockers[idx]);
                    }
                }
                // If we didn't get all blockers, add the remaining ones
                if result.len() < blockers.len() {
                    for &blocker in blockers {
                        if !result.contains(&blocker) {
                            result.push(blocker);
                        }
                    }
                }
                ChoiceResult::Ok(result)
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
        // Server sends indices of cards to discard (multi-select)
        match self.wait_for_choice() {
            ChoiceResult::Ok(indices) => {
                let mut discards = SmallVec::new();
                for idx in indices {
                    if idx < hand.len() {
                        discards.push(hand[idx]);
                    }
                }
                ChoiceResult::Ok(discards)
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
        // Server sends indices of permanents to sacrifice (multi-select)
        match self.wait_for_choice() {
            ChoiceResult::Ok(indices) => {
                let mut sacrifices = SmallVec::new();
                for idx in indices {
                    if idx < valid_permanents.len() {
                        sacrifices.push(valid_permanents[idx]);
                    }
                }
                ChoiceResult::Ok(sacrifices)
            }
            ChoiceResult::ExitGame => ChoiceResult::ExitGame,
            ChoiceResult::Error(e) => ChoiceResult::Error(e),
            ChoiceResult::UndoRequest(_) => ChoiceResult::Error("Undo not supported in network games".to_string()),
            ChoiceResult::NeedInput(_) => ChoiceResult::Error("NeedInput not supported in network games".to_string()),
        }
    }

    fn choose_permanents_to_not_untap(
        &mut self,
        _view: &GameStateView,
        _may_not_untap_permanents: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        // TODO: Add network protocol support for this choice
        // For now, auto-untap everything
        ChoiceResult::Ok(SmallVec::new())
    }

    fn choose_modes(
        &mut self,
        _view: &GameStateView,
        _spell_id: CardId,
        mode_descriptions: &[String],
        mode_count: usize,
        _min_modes: usize,
        _can_repeat: bool,
    ) -> ChoiceResult<SmallVec<[usize; 4]>> {
        // TODO: Add network protocol support for mode selection
        // For now, default to first N modes
        ChoiceResult::Ok((0..mode_count.min(mode_descriptions.len())).collect())
    }

    fn on_priority_passed(&mut self, _view: &GameStateView) {
        // Nothing to do
    }

    fn on_game_end(&mut self, _view: &GameStateView, _won: bool) {
        // Nothing to do
    }

    fn get_controller_type(&self) -> ControllerType {
        ControllerType::Remote
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
            choice_indices: vec![2],
            description: "Cast Lightning Bolt".to_string(),
            spell_ability: None,
            card_reveal: None,
            reveals: Vec::new(),
        })
        .unwrap();

        // Controller should receive it
        let result = controller.wait_for_choice();
        assert!(matches!(result, ChoiceResult::Ok(ref v) if v == &vec![2]));
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
