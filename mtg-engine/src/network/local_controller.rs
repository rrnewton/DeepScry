//! Network-aware local controller wrapper
//!
//! This wraps any PlayerController and sends choices to the server after each decision.
//!
//! ## Architecture (Pre-Choice Hook Design)
//!
//! The pre-choice hook in GameLoop handles all network message processing:
//! - Blocks on the message channel
//! - Processes CardRevealed messages to update GameState
//! - Returns ChoiceRequest/OpponentChoice signals
//!
//! This controller simply:
//! 1. Delegates to the wrapped inner controller
//! 2. Sends the choice to the server via client_tx
//!
//! It does NOT:
//! - Block on message_rx (the hook does this)
//! - Process CardRevealed messages (the hook does this)
//! - Wait for ChoiceAccepted (the hook handles this on next choice)

use crate::core::{CardId, ManaCost, PlayerId, SpellAbility};
use crate::game::controller::{ChoiceResult, GameStateView, PlayerController};
use crate::game::snapshot::ControllerType;
use crate::network::protocol::ClientMessage;
use crate::network::ClientMessageSender;
use smallvec::SmallVec;
use std::cell::Cell;
use std::rc::Rc;

/// A choice made by the local player, to be sent to the server
#[derive(Debug, Clone)]
pub struct LocalChoice {
    /// The choice indices selected (multiple for attackers/blockers/discard)
    pub choice_indices: Vec<usize>,
    /// Human-readable description
    pub description: String,
    /// Action count (undo log position) at the time of choice
    pub action_count: u64,
    /// Last N actions from the undo log (for sync debugging)
    pub last_actions: Option<String>,
    /// Client's computed state hash (for server validation in debug mode)
    pub client_state_hash: Option<u64>,
    /// Debug synchronization info (only in network debug mode)
    pub debug_info: Option<super::DebugSyncInfo>,
}

/// A controller that wraps a local controller and sends choices to the server.
///
/// This is used on the client side for our player. The pre-choice hook in GameLoop
/// handles network message processing. This controller simply:
/// 1. Delegates to the inner controller
/// 2. Sends the choice to the server
pub struct NetworkLocalController<C: PlayerController> {
    /// The wrapped local controller
    inner: C,
    /// Channel to send client messages (choices) to WebSocket writer
    client_tx: ClientMessageSender,
    /// Network debug mode: include action log info in choices for sync validation
    network_debug: bool,
    /// Shared choice sequence number (pre-choice hook updates it, controller reads it)
    choice_seq: Rc<Cell<u32>>,
}

impl<C: PlayerController> NetworkLocalController<C> {
    /// Create a new network local controller
    ///
    /// # Arguments
    /// * `inner` - The actual controller to delegate choices to
    /// * `client_tx` - Channel to send client messages to WebSocket writer
    /// * `choice_seq` - Shared choice sequence number (hook updates it, we read it)
    pub fn new(inner: C, client_tx: ClientMessageSender, choice_seq: Rc<Cell<u32>>) -> Self {
        Self {
            inner,
            client_tx,
            network_debug: false,
            choice_seq,
        }
    }

    /// Enable network debug mode for action log transmission
    pub fn with_network_debug(mut self, enabled: bool) -> Self {
        self.network_debug = enabled;
        self
    }

    /// Get access to the inner controller
    pub fn inner(&self) -> &C {
        &self.inner
    }

    /// Get mutable access to the inner controller
    pub fn inner_mut(&mut self) -> &mut C {
        &mut self.inner
    }

    /// Send a choice to the server (fire-and-forget, no waiting for ack)
    fn send_choice(
        &self,
        choice_indices: Vec<usize>,
        action_count: u64,
        client_state_hash: Option<u64>,
        debug_info: Option<super::DebugSyncInfo>,
    ) {
        let client_msg = ClientMessage::SubmitChoice {
            choice_seq: self.choice_seq.get(),
            choice_indices,
            action_count,
            timestamp_ms: 0,
            client_state_hash,
            debug_info,
        };

        // Fire and forget - the hook will handle any errors on next recv
        let _ = self.client_tx.send(client_msg);
    }

    /// Get debug fields for a choice when network_debug is enabled
    fn get_debug_fields(&self, view: &GameStateView) -> (Option<u64>, Option<super::DebugSyncInfo>) {
        if self.network_debug {
            let client_state_hash = Some(crate::game::compute_view_hash(view));
            let debug_info = Some(crate::game::build_debug_sync_info(view, 10));
            (client_state_hash, debug_info)
        } else {
            (None, None)
        }
    }

    /// Helper to wrap a choice result and send to server
    fn handle_choice<T>(&self, view: &GameStateView, result: ChoiceResult<T>, indices: Vec<usize>) -> ChoiceResult<T> {
        match &result {
            ChoiceResult::Ok(_) => {
                let (hash, debug) = self.get_debug_fields(view);
                self.send_choice(indices, view.action_count() as u64, hash, debug);
            }
            _ => {}
        }
        result
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
        let result = self.inner.choose_spell_ability_to_play(view, available);

        // Convert result to index and send
        if let ChoiceResult::Ok(ref choice) = result {
            let idx = match choice {
                None => 0, // Pass
                Some(ability) => available.iter().position(|a| a == ability).map(|i| i + 1).unwrap_or(0),
            };
            let (hash, debug) = self.get_debug_fields(view);
            self.send_choice(vec![idx], view.action_count() as u64, hash, debug);
        }

        result
    }

    fn choose_targets(
        &mut self,
        view: &GameStateView,
        spell: CardId,
        valid_targets: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        let result = self.inner.choose_targets(view, spell, valid_targets);

        if let ChoiceResult::Ok(ref targets) = result {
            let indices: Vec<usize> = targets
                .iter()
                .filter_map(|t| valid_targets.iter().position(|v| v == t))
                .collect();
            let (hash, debug) = self.get_debug_fields(view);
            self.send_choice(indices, view.action_count() as u64, hash, debug);
        }

        result
    }

    fn choose_mana_sources_to_pay(
        &mut self,
        view: &GameStateView,
        cost: &ManaCost,
        available_sources: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        let result = self.inner.choose_mana_sources_to_pay(view, cost, available_sources);

        if let ChoiceResult::Ok(ref sources) = result {
            let indices: Vec<usize> = sources
                .iter()
                .filter_map(|s| available_sources.iter().position(|a| a == s))
                .collect();
            let (hash, debug) = self.get_debug_fields(view);
            self.send_choice(indices, view.action_count() as u64, hash, debug);
        }

        result
    }

    fn choose_attackers(
        &mut self,
        view: &GameStateView,
        available_creatures: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        let result = self.inner.choose_attackers(view, available_creatures);

        if let ChoiceResult::Ok(ref attackers) = result {
            // Index 0 = pass, index N = creature N-1
            let indices: Vec<usize> = attackers
                .iter()
                .filter_map(|a| available_creatures.iter().position(|c| c == a).map(|i| i + 1))
                .collect();
            let indices = if indices.is_empty() { vec![0] } else { indices };
            let (hash, debug) = self.get_debug_fields(view);
            self.send_choice(indices, view.action_count() as u64, hash, debug);
        }

        result
    }

    fn choose_blockers(
        &mut self,
        view: &GameStateView,
        available_blockers: &[CardId],
        attackers: &[CardId],
    ) -> ChoiceResult<SmallVec<[(CardId, CardId); 8]>> {
        let result = self.inner.choose_blockers(view, available_blockers, attackers);

        if let ChoiceResult::Ok(ref blocks) = result {
            // Index 0 = pass, index N = (blocker_idx, attacker_idx) pair + 1
            let indices: Vec<usize> = blocks
                .iter()
                .filter_map(|(blocker, attacker)| {
                    let blocker_idx = available_blockers.iter().position(|b| b == blocker)?;
                    let attacker_idx = attackers.iter().position(|a| a == attacker)?;
                    Some(blocker_idx * attackers.len() + attacker_idx + 1)
                })
                .collect();
            let indices = if indices.is_empty() { vec![0] } else { indices };
            let (hash, debug) = self.get_debug_fields(view);
            self.send_choice(indices, view.action_count() as u64, hash, debug);
        }

        result
    }

    fn choose_damage_assignment_order(
        &mut self,
        view: &GameStateView,
        attacker: CardId,
        blockers: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        let result = self.inner.choose_damage_assignment_order(view, attacker, blockers);

        if let ChoiceResult::Ok(ref order) = result {
            let indices: Vec<usize> = order
                .iter()
                .filter_map(|b| blockers.iter().position(|bl| bl == b))
                .collect();
            let (hash, debug) = self.get_debug_fields(view);
            self.send_choice(indices, view.action_count() as u64, hash, debug);
        }

        result
    }

    fn choose_cards_to_discard(
        &mut self,
        view: &GameStateView,
        hand: &[CardId],
        count: usize,
    ) -> ChoiceResult<SmallVec<[CardId; 7]>> {
        let result = self.inner.choose_cards_to_discard(view, hand, count);

        if let ChoiceResult::Ok(ref discards) = result {
            let indices: Vec<usize> = discards
                .iter()
                .filter_map(|d| hand.iter().position(|h| h == d))
                .collect();
            let (hash, debug) = self.get_debug_fields(view);
            self.send_choice(indices, view.action_count() as u64, hash, debug);
        }

        result
    }

    fn choose_from_library(&mut self, view: &GameStateView, valid_cards: &[CardId]) -> ChoiceResult<Option<CardId>> {
        let result = self.inner.choose_from_library(view, valid_cards);

        if let ChoiceResult::Ok(ref choice) = result {
            let idx = match choice {
                Some(card) => valid_cards.iter().position(|c| c == card).unwrap_or(valid_cards.len()),
                None => valid_cards.len(),
            };
            let (hash, debug) = self.get_debug_fields(view);
            self.send_choice(vec![idx], view.action_count() as u64, hash, debug);
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
        let result = self
            .inner
            .choose_permanents_to_sacrifice(view, valid_permanents, count, card_type_description);

        if let ChoiceResult::Ok(ref sacrifices) = result {
            let indices: Vec<usize> = sacrifices
                .iter()
                .filter_map(|s| valid_permanents.iter().position(|p| p == s))
                .collect();
            let (hash, debug) = self.get_debug_fields(view);
            self.send_choice(indices, view.action_count() as u64, hash, debug);
        }

        result
    }

    fn choose_permanents_to_not_untap(
        &mut self,
        view: &GameStateView,
        may_not_untap_permanents: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        let result = self
            .inner
            .choose_permanents_to_not_untap(view, may_not_untap_permanents);

        if let ChoiceResult::Ok(ref stay_tapped) = result {
            let indices: Vec<usize> = stay_tapped
                .iter()
                .filter_map(|s| may_not_untap_permanents.iter().position(|p| p == s))
                .collect();
            let (hash, debug) = self.get_debug_fields(view);
            self.send_choice(indices, view.action_count() as u64, hash, debug);
        }

        result
    }

    fn choose_modes(
        &mut self,
        view: &GameStateView,
        spell_id: CardId,
        mode_descriptions: &[String],
        mode_count: usize,
        min_modes: usize,
        can_repeat: bool,
    ) -> ChoiceResult<SmallVec<[usize; 4]>> {
        let result = self
            .inner
            .choose_modes(view, spell_id, mode_descriptions, mode_count, min_modes, can_repeat);

        if let ChoiceResult::Ok(ref modes) = result {
            let indices: Vec<usize> = modes.iter().copied().collect();
            let (hash, debug) = self.get_debug_fields(view);
            self.send_choice(indices, view.action_count() as u64, hash, debug);
        }

        result
    }

    fn on_priority_passed(&mut self, view: &GameStateView) {
        self.inner.on_priority_passed(view);
    }

    fn on_game_end(&mut self, view: &GameStateView, won: bool) {
        self.inner.on_game_end(view, won);
    }

    fn get_controller_type(&self) -> ControllerType {
        // Return Network type so GameLoop knows this is a network-controlled local player
        ControllerType::Network
    }
}

// Legacy types for backward compatibility (can be removed later)

/// A buffered card reveal waiting to be processed
#[derive(Debug, Clone)]
pub struct BufferedReveal {
    pub owner: PlayerId,
    pub card: crate::network::protocol::CardReveal,
    pub reason: crate::network::protocol::RevealReason,
}

/// Shared buffer for pending reveals (legacy - used by old architecture)
pub type PendingReveals = std::sync::Arc<std::sync::Mutex<Vec<BufferedReveal>>>;

/// A bundled card reveal (legacy)
#[derive(Debug, Clone)]
pub struct BundledReveal {
    pub owner: PlayerId,
    pub card_id: CardId,
    pub card_name: String,
    pub reason: crate::network::protocol::RevealReason,
}
