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
    /// The choice indices selected (multiple for attackers/blockers/discard)
    pub choice_indices: Vec<usize>,
    /// Human-readable description
    pub description: String,
    /// Action count (undo log position) at the time of choice
    /// This is used for synchronization validation with the server
    pub action_count: u64,
    /// Last N actions from the undo log (for sync debugging)
    /// Only populated when debug mode is enabled
    pub last_actions: Option<String>,
    /// Client's computed state hash (for server validation in debug mode)
    /// Computed using compute_view_hash from the GameStateView
    pub client_state_hash: Option<u64>,
    /// Debug synchronization info (only in network debug mode)
    /// Contains turn, phase, life totals, zone sizes for comparison
    pub debug_info: Option<super::DebugSyncInfo>,
}

/// A bundled card reveal with all info needed to instantiate
#[derive(Debug, Clone)]
pub struct BundledReveal {
    pub owner: PlayerId,
    pub card_id: CardId,
    pub card_name: String,
    pub reason: super::protocol::RevealReason,
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
    ///
    /// IMPORTANT: The `reveals` field contains all CardRevealed messages that
    /// arrived BEFORE this ChoiceRequest. They MUST be processed before the
    /// game evaluates available options. This eliminates race conditions from
    /// having separate channels for reveals vs choice requests.
    ChoiceRequest {
        /// Server's authoritative action count (for sync validation)
        action_count: u64,
        /// Server's choice sequence number
        choice_seq: u32,
        /// Bundled reveals that arrived before this ChoiceRequest
        /// These should be processed before evaluating available options
        reveals: Vec<BundledReveal>,
    },
    /// Server acknowledged our choice, continue
    ChoiceAcknowledged,
    /// Server reported an error
    Error(String),
    /// Game has ended
    GameEnded,
}

/// Reveal message type for controller → game thread communication
pub type RevealMsg = (
    crate::core::PlayerId,
    super::protocol::CardReveal,
    super::protocol::RevealReason,
);

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
    /// Channel to send bundled reveals to the game thread for processing
    /// This is used to push reveals received with ChoiceRequest so drain_reveals() can process them
    reveal_tx: Option<mpsc::Sender<RevealMsg>>,
    /// Whether we've been disconnected
    disconnected: bool,
    /// Network debug mode: include action log info in choices for sync validation
    network_debug: bool,
    /// Last received server action count (from ChoiceRequest)
    server_action_count: Option<u64>,
    /// Last received server choice sequence (from ChoiceRequest)
    server_choice_seq: Option<u32>,
    /// Bundled reveals from the last ChoiceRequest that need processing
    /// These are extracted when wait_for_choice_request receives a ChoiceRequest
    pending_reveals: Vec<BundledReveal>,
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
            reveal_tx: None,
            disconnected: false,
            network_debug: false,
            server_action_count: None,
            server_choice_seq: None,
            pending_reveals: Vec::new(),
        }
    }

    /// Set the reveal channel for pushing bundled reveals to the game thread.
    /// This channel is used to send reveals that arrive with ChoiceRequest messages
    /// so that drain_reveals() can process them before evaluating available options.
    pub fn with_reveal_tx(mut self, reveal_tx: mpsc::Sender<RevealMsg>) -> Self {
        self.reveal_tx = Some(reveal_tx);
        self
    }

    /// Enable network debug mode for action log transmission
    ///
    /// When enabled, the last N actions are included with each choice
    /// for sync validation and debugging.
    pub fn with_network_debug(mut self, enabled: bool) -> Self {
        self.network_debug = enabled;
        self
    }

    /// Push bundled reveals to the reveal channel for processing by drain_reveals().
    /// This is called after receiving a ChoiceRequest with bundled reveals.
    fn push_pending_reveals(&mut self) {
        if let Some(ref reveal_tx) = self.reveal_tx {
            for reveal in self.pending_reveals.drain(..) {
                let card_reveal = super::protocol::CardReveal {
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
                    "NetworkLocalController: {} bundled reveals dropped (no reveal_tx channel)",
                    self.pending_reveals.len()
                );
                self.pending_reveals.clear();
            }
        }
    }

    /// Wait for a ChoiceRequest from the server before making a choice
    ///
    /// This blocks until the server sends a ChoiceRequest, ensuring the client
    /// doesn't run ahead of the server. If a ChoiceRequest was already received,
    /// this returns immediately.
    ///
    /// Returns true if we're ready to proceed, false if we're disconnected.
    /// When true, the `pending_reveals` field is populated with any reveals
    /// that were bundled with the ChoiceRequest.
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
                    reveals,
                }) => {
                    log::trace!(
                        "NetworkLocalController: received ChoiceRequest seq={} action_count={} with {} bundled reveals",
                        choice_seq,
                        action_count,
                        reveals.len()
                    );
                    self.server_action_count = Some(action_count);
                    self.server_choice_seq = Some(choice_seq);
                    self.pending_reveals = reveals;
                    // Push bundled reveals to the reveal channel for drain_reveals() to process
                    self.push_pending_reveals();
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
    /// * `choice_indices` - The selected choice indices (single for priority, multiple for attackers)
    /// * `description` - Human-readable description of the choice
    /// * `action_count` - Current action count (undo log position) for sync validation
    /// * `last_actions` - Formatted string of last N actions (debug mode only)
    /// * `client_state_hash` - Client's computed state hash (debug mode only)
    /// * `debug_info` - Full debug sync info (debug mode only)
    fn send_choice(
        &mut self,
        choice_indices: Vec<usize>,
        description: String,
        action_count: u64,
        last_actions: Option<String>,
        client_state_hash: Option<u64>,
        debug_info: Option<super::DebugSyncInfo>,
    ) -> Result<(), String> {
        if self.disconnected {
            return Err("Disconnected from server".to_string());
        }

        log::trace!(
            "NetworkLocalController: sending choice {:?} ({}) at action_count={}",
            choice_indices,
            description,
            action_count
        );

        // Send choice
        if self
            .choice_tx
            .send(LocalChoice {
                choice_indices,
                description,
                action_count,
                last_actions,
                client_state_hash,
                debug_info,
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
                    reveals: _,
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

    /// Get all debug fields for a choice when network_debug is enabled
    ///
    /// Returns (last_actions, client_state_hash, debug_info) tuple.
    /// All fields are None when network_debug is disabled.
    fn get_debug_fields(&self, view: &GameStateView) -> (Option<String>, Option<u64>, Option<super::DebugSyncInfo>) {
        if self.network_debug {
            let last_actions = Some(view.format_last_n_actions(20));
            let client_state_hash = Some(crate::game::compute_view_hash(view));
            let debug_info = Some(crate::game::build_debug_sync_info(view, 10));
            (last_actions, client_state_hash, debug_info)
        } else {
            (None, None, None)
        }
    }
}

/// Note: Wildcards are intentional in match arms on ChoiceResult - the enum
/// has several variants; we handle Choice and ExitGame specially, others pass through.
#[allow(clippy::wildcard_enum_match_arm)]
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
        let action_count = self.server_action_count.unwrap_or_else(|| view.action_count() as u64);
        let (last_actions, client_state_hash, debug_info) = self.get_debug_fields(view);
        match &result {
            ChoiceResult::Ok(Some(ability)) => {
                let idx = available.iter().position(|a| a == ability).unwrap_or(0) + 1;
                let desc = format!("Play: {:?}", ability);
                if let Err(e) = self.send_choice(
                    vec![idx],
                    desc,
                    action_count,
                    last_actions,
                    client_state_hash,
                    debug_info,
                ) {
                    log::error!("Failed to send choice: {}", e);
                    return ChoiceResult::ExitGame;
                }
            }
            ChoiceResult::Ok(None) => {
                if let Err(e) = self.send_choice(
                    vec![0],
                    "Pass".to_string(),
                    action_count,
                    last_actions,
                    client_state_hash,
                    debug_info,
                ) {
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
            // For targets, send all selected indices
            let indices: Vec<usize> = if targets.is_empty() {
                vec![valid_targets.len()] // No target
            } else {
                targets
                    .iter()
                    .filter_map(|&t| valid_targets.iter().position(|&vt| vt == t))
                    .collect()
            };
            let desc = format!("Target: {:?}", targets);
            let action_count = self.server_action_count.unwrap_or_else(|| view.action_count() as u64);
            let (last_actions, client_state_hash, debug_info) = self.get_debug_fields(view);
            if let Err(e) = self.send_choice(indices, desc, action_count, last_actions, client_state_hash, debug_info) {
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
            // Send all mana source indices
            let indices: Vec<usize> = if sources.is_empty() {
                vec![available_sources.len()]
            } else {
                sources
                    .iter()
                    .filter_map(|&s| available_sources.iter().position(|&as_| as_ == s))
                    .collect()
            };
            let desc = format!("Mana source: {:?}", sources);
            let action_count = self.server_action_count.unwrap_or_else(|| view.action_count() as u64);
            let (last_actions, client_state_hash, debug_info) = self.get_debug_fields(view);
            if let Err(e) = self.send_choice(indices, desc, action_count, last_actions, client_state_hash, debug_info) {
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
            // Send all attacker indices (multi-select)
            // Index 0 means "done selecting" / no attackers
            // Index N means attacker at position N-1 in available_creatures
            let indices: Vec<usize> = if attackers.is_empty() {
                vec![0] // No attackers
            } else {
                attackers
                    .iter()
                    .filter_map(|&a| available_creatures.iter().position(|&ac| ac == a).map(|i| i + 1))
                    .collect()
            };
            let desc = format!("Attackers: {:?}", attackers);
            let action_count = self.server_action_count.unwrap_or_else(|| view.action_count() as u64);
            let (last_actions, client_state_hash, debug_info) = self.get_debug_fields(view);
            if let Err(e) = self.send_choice(indices, desc, action_count, last_actions, client_state_hash, debug_info) {
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
            // Send all blocker assignments (multi-select)
            // Index 0 means "done selecting" / no blockers
            // For each block, encode as blocker_idx * num_attackers + attacker_idx + 1
            let indices: Vec<usize> = if blocks.is_empty() {
                vec![0] // No blockers
            } else {
                blocks
                    .iter()
                    .filter_map(|&(blocker, attacker)| {
                        let blocker_idx = available_blockers.iter().position(|&b| b == blocker)?;
                        let attacker_idx = attackers.iter().position(|&a| a == attacker)?;
                        Some(blocker_idx * attackers.len() + attacker_idx + 1)
                    })
                    .collect()
            };
            let desc = format!("Blocks: {:?}", blocks);
            let action_count = self.server_action_count.unwrap_or_else(|| view.action_count() as u64);
            let (last_actions, client_state_hash, debug_info) = self.get_debug_fields(view);
            if let Err(e) = self.send_choice(indices, desc, action_count, last_actions, client_state_hash, debug_info) {
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
            // Send all damage order indices (the order matters for damage assignment)
            let indices: Vec<usize> = if order.is_empty() {
                vec![0]
            } else {
                order
                    .iter()
                    .filter_map(|&b| blockers.iter().position(|&bl| bl == b))
                    .collect()
            };
            let desc = format!("Damage order: {:?}", order);
            let action_count = self.server_action_count.unwrap_or_else(|| view.action_count() as u64);
            let (last_actions, client_state_hash, debug_info) = self.get_debug_fields(view);
            if let Err(e) = self.send_choice(indices, desc, action_count, last_actions, client_state_hash, debug_info) {
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
            // Send all discard indices (multi-select)
            let indices: Vec<usize> = if discards.is_empty() {
                vec![hand.len()]
            } else {
                discards
                    .iter()
                    .filter_map(|&c| hand.iter().position(|&h| h == c))
                    .collect()
            };
            let desc = format!("Discard: {:?}", discards);
            let action_count = self.server_action_count.unwrap_or_else(|| view.action_count() as u64);
            let (last_actions, client_state_hash, debug_info) = self.get_debug_fields(view);
            if let Err(e) = self.send_choice(indices, desc, action_count, last_actions, client_state_hash, debug_info) {
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
            // Single-select from library
            let idx = match card {
                Some(c) => valid_cards.iter().position(|&v| v == *c).unwrap_or(valid_cards.len()),
                None => valid_cards.len(),
            };
            let desc = format!("From library: {:?}", card);
            let action_count = self.server_action_count.unwrap_or_else(|| view.action_count() as u64);
            let (last_actions, client_state_hash, debug_info) = self.get_debug_fields(view);
            if let Err(e) = self.send_choice(
                vec![idx],
                desc,
                action_count,
                last_actions,
                client_state_hash,
                debug_info,
            ) {
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
            // Send all sacrifice indices (multi-select)
            let indices: Vec<usize> = if permanents.is_empty() {
                vec![valid_permanents.len()]
            } else {
                permanents
                    .iter()
                    .filter_map(|&p| valid_permanents.iter().position(|&vp| vp == p))
                    .collect()
            };
            let desc = format!("Sacrifice: {:?}", permanents);
            let action_count = view.action_count() as u64;
            let (last_actions, client_state_hash, debug_info) = self.get_debug_fields(view);
            if let Err(e) = self.send_choice(indices, desc, action_count, last_actions, client_state_hash, debug_info) {
                log::error!("Failed to send choice: {}", e);
                return ChoiceResult::ExitGame;
            }
        }

        result
    }

    fn choose_permanents_to_not_untap(
        &mut self,
        view: &GameStateView,
        may_not_untap_permanents: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        // Delegate to inner controller
        self.inner
            .choose_permanents_to_not_untap(view, may_not_untap_permanents)
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
        // Delegate to inner controller
        self.inner
            .choose_modes(view, spell_id, mode_descriptions, mode_count, min_modes, can_repeat)
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
        // Return Network type so we don't auto-pass when available_count=0.
        // This ensures we always wait for ChoiceRequest and send SubmitChoice,
        // even when there are no abilities available.
        ControllerType::Network
    }
}
