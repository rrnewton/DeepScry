//! Network controller for remote player communication
//!
//! This module provides `NetworkController`, a `PlayerController` implementation
//! that proxies player decisions over a network connection. It's used server-side
//! to represent remote players connected via WebSocket.

use crate::core::{CardId, ManaCost, PlayerId, SpellAbility};
use crate::game::controller::{ChoiceResult, GameStateView, PlayerController};
use crate::game::snapshot::ControllerType;
use crate::network::protocol::ChoiceType;
use crate::undo::GameAction;
use crate::zones::Zone;
use smallvec::SmallVec;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::Arc;

/// Message sent from NetworkController to WebSocket handler after a priority choice
/// This allows the handler to include the actual ability in OpponentChoice
#[derive(Debug, Clone)]
pub struct ChosenAbilityInfo {
    /// The choice sequence number this ability corresponds to
    pub choice_seq: u32,
    /// The ability chosen (None = pass priority)
    pub ability: Option<SpellAbility>,
}

// ═══════════════════════════════════════════════════════════════════════════
// CARD REVEAL INFO
// ═══════════════════════════════════════════════════════════════════════════

/// Information about a card that was revealed since the player's last choice
///
/// This captures card movements from hidden zones (library) to visible zones.
/// The server uses this to send `CardRevealed` messages to the client before
/// sending the next `ChoiceRequest`.
#[derive(Debug, Clone)]
pub struct CardRevealInfo {
    /// The card that was revealed
    pub card_id: CardId,
    /// Who owns the card
    pub owner: PlayerId,
    /// Zone the card moved from (typically Library)
    pub from_zone: Zone,
    /// Zone the card moved to
    pub to_zone: Zone,
}

// ═══════════════════════════════════════════════════════════════════════════
// CHANNEL TYPES
// ═══════════════════════════════════════════════════════════════════════════

/// Request sent to the network handler for forwarding to client
#[derive(Debug, Clone)]
pub struct ChoiceRequest {
    /// Sequence number for correlation
    pub choice_seq: u32,
    /// Type of choice being requested
    pub choice_type: ChoiceType,
    /// Human-readable options
    pub options: Vec<String>,
    /// Game state hash (excluding hidden info)
    pub state_hash: u64,
    /// Action count at this choice point (undo log position)
    /// This is the source of truth for synchronization
    pub action_count: u64,
    /// Cards revealed since this player's last choice
    ///
    /// The server should send `CardRevealed` messages for these before
    /// sending the `ChoiceRequest` to the client.
    pub reveals: Vec<CardRevealInfo>,
    /// Debug synchronization info (only when network_debug is enabled)
    pub debug_info: Option<super::DebugSyncInfo>,
}

/// Response received from the network handler
#[derive(Debug, Clone)]
pub struct ChoiceResponse {
    /// Sequence number (must match request)
    pub choice_seq: u32,
    /// Indices of the chosen options
    ///
    /// For single-select choices, this is a 1-element vec.
    /// For multi-select choices (attackers, blockers), contains all selected indices.
    pub choice_indices: Vec<usize>,
}

/// Error that can occur during network communication
#[derive(Debug, Clone)]
pub enum NetworkError {
    /// Client disconnected
    Disconnected,
    /// Request timed out
    Timeout,
    /// Sequence number mismatch
    SequenceMismatch { expected: u32, got: u32 },
    /// Invalid choice index
    InvalidChoice { max: usize, got: usize },
    /// Channel error
    ChannelError(String),
}

impl std::fmt::Display for NetworkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NetworkError::Disconnected => write!(f, "Client disconnected"),
            NetworkError::Timeout => write!(f, "Request timed out"),
            NetworkError::SequenceMismatch { expected, got } => {
                write!(f, "Sequence mismatch: expected {}, got {}", expected, got)
            }
            NetworkError::InvalidChoice { max, got } => {
                write!(f, "Invalid choice index: {} (max {})", got, max)
            }
            NetworkError::ChannelError(msg) => write!(f, "Channel error: {}", msg),
        }
    }
}

impl std::error::Error for NetworkError {}

// ═══════════════════════════════════════════════════════════════════════════
// NETWORK CONTROLLER
// ═══════════════════════════════════════════════════════════════════════════

/// Controller that communicates with a remote player over network
///
/// This is used SERVER-SIDE to represent a remote player. When the game
/// needs a decision from this player, the controller:
/// 1. Builds an options list matching the local display format
/// 2. Computes the network state hash (public info only)
/// 3. Sends a ChoiceRequest over the channel
/// 4. Awaits the ChoiceResponse
/// 5. Returns the selected option to the game loop
///
/// The actual WebSocket communication is handled by a separate task that
/// reads from request_rx and writes to response_tx.
pub struct NetworkController {
    /// Player ID this controller represents
    player_id: PlayerId,
    /// Channel to send choice requests
    request_tx: mpsc::Sender<ChoiceRequest>,
    /// Channel to receive choice responses
    response_rx: mpsc::Receiver<ChoiceResponse>,
    /// Current choice sequence number
    choice_seq: u32,
    /// Shared index into the undo_log where reveals were last sent.
    /// This is shared with the immediate reveal pusher to prevent duplicate reveals.
    /// Initialized to the number of opening hand draw actions (14 for 7+7).
    shared_reveal_index: Arc<AtomicUsize>,
    /// Network debug mode - include debug info in choice requests
    network_debug: bool,
    /// Channel to send the chosen ability back to WebSocket handler
    /// This allows OpponentChoice to include the actual ability for opponent sync
    ability_tx: Option<mpsc::Sender<ChosenAbilityInfo>>,
}

impl NetworkController {
    /// Create a new network controller with a shared reveal index
    ///
    /// The `shared_reveal_index` is shared with the immediate reveal pusher hook
    /// to ensure both systems track the same position and don't send duplicate reveals.
    pub fn new(
        player_id: PlayerId,
        request_tx: mpsc::Sender<ChoiceRequest>,
        response_rx: mpsc::Receiver<ChoiceResponse>,
        shared_reveal_index: Arc<AtomicUsize>,
    ) -> Self {
        NetworkController {
            player_id,
            request_tx,
            response_rx,
            choice_seq: 0,
            shared_reveal_index,
            network_debug: false,
            ability_tx: None,
        }
    }

    /// Get the shared reveal index for the immediate reveal pusher
    ///
    /// This allows the reveal pusher hook to share the same tracking index.
    pub fn shared_reveal_index(&self) -> Arc<AtomicUsize> {
        self.shared_reveal_index.clone()
    }

    /// Set the channel for sending chosen abilities back to WebSocket handler
    ///
    /// When a priority choice is made, the chosen ability (or None for pass) is sent
    /// on this channel so the WebSocket handler can include it in OpponentChoice.
    pub fn set_ability_tx(&mut self, tx: mpsc::Sender<ChosenAbilityInfo>) {
        self.ability_tx = Some(tx);
    }

    /// Enable network debug mode
    ///
    /// When enabled, debug synchronization info is included in each choice request.
    pub fn set_network_debug(&mut self, enabled: bool) {
        self.network_debug = enabled;
    }

    /// Send a choice request and wait for response
    ///
    /// This also collects any card reveals since this player's last choice
    /// and includes them in the request, ensuring reveals arrive before the client's
    /// shadow GameLoop needs them (eliminates race conditions with immediate reveals).
    fn request_choice(
        &mut self,
        view: &GameStateView,
        choice_type: ChoiceType,
        options: Vec<String>,
        state_hash: u64,
    ) -> Result<Vec<usize>, NetworkError> {
        // Collect reveals since this player's last choice and include them in the request.
        // This ensures reveals are sent BEFORE the ChoiceRequest arrives, eliminating
        // race conditions where the client's shadow GameLoop needs reveals before they arrive.
        //
        // The immediate reveal system (reveal_pusher) coordinates with us via shared_reveal_index
        // to ensure each reveal is only sent once (either here or by the pusher, not both).
        let reveals = self.collect_reveals_since_last_choice(view);

        if !reveals.is_empty() {
            log::debug!(
                "NetworkController {:?}: collected {} reveals for ChoiceRequest (action_count={})",
                self.player_id,
                reveals.len(),
                view.action_count()
            );
            for reveal in &reveals {
                log::debug!(
                    "  Reveal: card {:?} from {:?} to {:?} (owner {:?})",
                    reveal.card_id,
                    reveal.from_zone,
                    reveal.to_zone,
                    reveal.owner
                );
            }
        }

        // Get action count from GameState undo log for synchronization
        let action_count = view.action_count() as u64;

        // Build debug info if network debug mode is enabled
        let debug_info = if self.network_debug {
            Some(crate::game::build_debug_sync_info(view, 10))
        } else {
            None
        };

        let request = ChoiceRequest {
            choice_seq: self.choice_seq + 1,
            choice_type,
            options: options.clone(),
            state_hash,
            action_count,
            reveals,
            debug_info,
        };

        // Send request
        log::debug!(
            "Server NetworkController {:?}: sending ChoiceRequest #{} (action_count={}, type={:?})",
            self.player_id,
            request.choice_seq,
            request.action_count,
            request.choice_type
        );
        self.request_tx
            .send(request)
            .map_err(|e| NetworkError::ChannelError(e.to_string()))?;

        // Wait for response
        let response = self
            .response_rx
            .recv()
            .map_err(|e| NetworkError::ChannelError(e.to_string()))?;

        // Verify sequence number
        if response.choice_seq != self.choice_seq + 1 {
            return Err(NetworkError::SequenceMismatch {
                expected: self.choice_seq + 1,
                got: response.choice_seq,
            });
        }

        // Validate choice indices and clamp invalid ones to safe values
        // This makes the network protocol robust to desync - we log a warning but don't crash
        let mut corrected_indices = response.choice_indices.clone();
        let mut had_invalid = false;

        for idx in &mut corrected_indices {
            if *idx >= options.len() {
                log::warn!(
                    "NetworkController {:?}: Invalid choice index {} (max {}), clamping to 0. \
                     This may indicate client/server desync.",
                    self.player_id,
                    *idx,
                    options.len()
                );
                // Clamp to index 0 which is typically "Pass" or the safest default
                *idx = 0;
                had_invalid = true;
            }
        }

        if had_invalid {
            log::warn!(
                "NetworkController {:?}: Corrected choice from {:?} to {:?}",
                self.player_id,
                response.choice_indices,
                corrected_indices
            );
        }

        Ok(corrected_indices)
    }

    /// Increment choice sequence after a successful choice
    fn increment_choice_seq(&mut self) {
        self.choice_seq += 1;
    }

    /// Format a SpellAbility for display (matching format_choice_menu)
    fn format_spell_ability(&self, view: &GameStateView, ability: &SpellAbility) -> String {
        match ability {
            SpellAbility::PlayLand { card_id } => {
                let name = view.card_name(*card_id).unwrap_or_default();
                format!("Play land: {}", name)
            }
            SpellAbility::CastSpell { card_id } => {
                let name = view.card_name(*card_id).unwrap_or_default();
                format!("Cast spell: {}", name)
            }
            SpellAbility::ActivateAbility { card_id, ability_index } => {
                let name = view.card_name(*card_id).unwrap_or_default();
                format!("Activate: {} (ability {})", name, ability_index)
            }
            SpellAbility::CastFromExile {
                card_id,
                alternative_cost,
                ..
            } => {
                let name = view.card_name(*card_id).unwrap_or_default();
                format!("Cast from exile: {} (for {})", name, alternative_cost)
            }
        }
    }

    /// Format a card for display
    fn format_card(&self, view: &GameStateView, card_id: CardId) -> String {
        view.card_name(card_id)
            .unwrap_or_else(|| format!("Card #{}", card_id.as_u32()))
    }

    /// Collect card reveals since this player's last choice
    ///
    /// Scans the undo log backwards from the current position until we find
    /// a `ChoicePoint` for this player, or until we reach `last_reveal_index`
    /// (actions before that were already sent during handshake).
    /// Returns `MoveCard` actions from Library that this player should see.
    ///
    /// A player sees a reveal if:
    /// - They own the card (e.g., their own draw to hand)
    /// - The card moved to a public zone (battlefield, graveyard, stack, exile)
    ///
    /// We do NOT reveal opponent's draws to hand - that would leak hidden information.
    ///
    /// Called by request_choice to bundle reveals with the ChoiceRequest.
    /// Uses the shared_reveal_index to coordinate with the immediate reveal pusher.
    ///
    /// Note: Wildcard is intentional - GameAction has many variants;
    /// we only collect MoveCard from Library.
    #[allow(clippy::wildcard_enum_match_arm)]
    fn collect_reveals_since_last_choice(&mut self, view: &GameStateView) -> Vec<CardRevealInfo> {
        let actions = view.undo_log_actions();
        let mut reveals = Vec::new();
        let total_actions = actions.len();

        // Read the shared index - this may have been updated by the immediate reveal pusher
        let last_reveal_index = self.shared_reveal_index.load(Ordering::Acquire);

        // Scan backwards from the end of the log, but stop at last_reveal_index
        for (rev_idx, action) in actions.iter().rev().enumerate() {
            // Convert reverse index to forward index
            let forward_idx = total_actions.saturating_sub(rev_idx + 1);

            // Stop if we've reached actions that were already handled
            if forward_idx < last_reveal_index {
                break;
            }

            match action {
                // Stop when we hit this player's last choice
                GameAction::ChoicePoint { player_id, .. } if *player_id == self.player_id => {
                    break;
                }
                // Collect card moves from library for THIS player only
                // A player sees a reveal if:
                // - They own the card (their own draw to hand)
                // - The card moved to a public zone (battlefield, graveyard, stack, exile)
                // We do NOT reveal opponent's draws to hand - that's hidden information
                GameAction::MoveCard {
                    card_id,
                    from_zone: Zone::Library,
                    to_zone,
                    owner,
                } => {
                    let is_public_zone =
                        matches!(to_zone, Zone::Battlefield | Zone::Graveyard | Zone::Stack | Zone::Exile);
                    let is_own_card = *owner == self.player_id;

                    if is_own_card || is_public_zone {
                        reveals.push(CardRevealInfo {
                            card_id: *card_id,
                            owner: *owner,
                            from_zone: Zone::Library,
                            to_zone: *to_zone,
                        });
                    }
                }
                _ => {}
            }
        }

        // Update shared index to current position after collecting reveals
        // This tracks our progress through the undo log for reveal collection.
        // The immediate reveal pusher runs in parallel but doesn't update the index,
        // so reveals may be sent twice (once via pusher, once here) - client handles duplicates.
        if !reveals.is_empty() {
            self.shared_reveal_index.store(total_actions, Ordering::Release);
        }

        // Reverse to get chronological order
        reveals.reverse();
        reveals
    }

    /// Compute a network-safe state hash for verification
    ///
    /// Uses `compute_view_hash` to compute a deterministic hash from the view.
    /// This produces identical results on server and client for the same
    /// game state, enabling early sync drift detection.
    fn compute_view_hash(&self, view: &GameStateView) -> u64 {
        crate::game::compute_view_hash(view)
    }
}

impl PlayerController for NetworkController {
    fn player_id(&self) -> PlayerId {
        self.player_id
    }

    fn choose_spell_ability_to_play(
        &mut self,
        view: &GameStateView,
        available: &[SpellAbility],
    ) -> ChoiceResult<Option<SpellAbility>> {
        // Build options list
        let mut options = vec!["Pass priority".to_string()];
        for ability in available {
            options.push(self.format_spell_ability(view, ability));
        }

        // Compute state hash
        let state_hash = self.compute_view_hash(view);

        // Send request and get response
        let choice_type = ChoiceType::Priority {
            available_count: available.len(),
        };

        match self.request_choice(view, choice_type, options, state_hash) {
            Ok(indices) if indices.first() == Some(&0) => {
                self.increment_choice_seq();
                // Send ability info (None = pass priority)
                if let Some(tx) = &self.ability_tx {
                    let _ = tx.send(ChosenAbilityInfo {
                        choice_seq: self.choice_seq,
                        ability: None,
                    });
                }
                ChoiceResult::Ok(None) // Pass priority
            }
            Ok(indices) => {
                self.increment_choice_seq();
                // Single-select: use first index. Subtract 1 because option 0 is "Pass priority"
                let idx = indices.first().copied().unwrap_or(0);
                let ability_idx = idx - 1;
                if ability_idx < available.len() {
                    let ability = available[ability_idx].clone();
                    // Send ability info to WebSocket handler for OpponentChoice
                    if let Some(tx) = &self.ability_tx {
                        let _ = tx.send(ChosenAbilityInfo {
                            choice_seq: self.choice_seq,
                            ability: Some(ability.clone()),
                        });
                    }
                    ChoiceResult::Ok(Some(ability))
                } else {
                    ChoiceResult::Error("Invalid ability index from network".to_string())
                }
            }
            Err(NetworkError::Disconnected) => ChoiceResult::ExitGame,
            Err(e) => ChoiceResult::Error(e.to_string()),
        }
    }

    fn choose_targets(
        &mut self,
        view: &GameStateView,
        spell: CardId,
        valid_targets: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        // Build options list
        let options: Vec<String> = valid_targets
            .iter()
            .map(|&card_id| self.format_card(view, card_id))
            .collect();

        if options.is_empty() {
            return ChoiceResult::Ok(SmallVec::new());
        }

        // Compute state hash
        let state_hash = self.compute_view_hash(view);

        // Send request
        let choice_type = ChoiceType::Targets {
            spell_id: spell,
            target_count: 1, // FIXME-UNFINISHED: Support multiple targets
        };

        match self.request_choice(view, choice_type, options, state_hash) {
            Ok(indices) => {
                self.increment_choice_seq();
                // Single-select for now (FIXME-UNFINISHED for multiple targets)
                let idx = indices.first().copied().unwrap_or(0);
                if idx < valid_targets.len() {
                    ChoiceResult::Ok(SmallVec::from_slice(&[valid_targets[idx]]))
                } else {
                    ChoiceResult::Error("Invalid target index from network".to_string())
                }
            }
            Err(NetworkError::Disconnected) => ChoiceResult::ExitGame,
            Err(e) => ChoiceResult::Error(e.to_string()),
        }
    }

    fn choose_mana_sources_to_pay(
        &mut self,
        view: &GameStateView,
        cost: &ManaCost,
        available_sources: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        // Build options - for each source show what mana it produces
        let options: Vec<String> = available_sources
            .iter()
            .map(|&card_id| self.format_card(view, card_id))
            .collect();

        // Compute state hash
        let state_hash = self.compute_view_hash(view);

        // Send request
        let choice_type = ChoiceType::ManaSources { cost: *cost };

        // FIXME-UNFINISHED: Needs multi-select for paying costs with multiple sources
        match self.request_choice(view, choice_type, options, state_hash) {
            Ok(indices) => {
                self.increment_choice_seq();
                // Single-select for now
                let idx = indices.first().copied().unwrap_or(0);
                if idx < available_sources.len() {
                    ChoiceResult::Ok(SmallVec::from_slice(&[available_sources[idx]]))
                } else {
                    ChoiceResult::Error("Invalid mana source index from network".to_string())
                }
            }
            Err(NetworkError::Disconnected) => ChoiceResult::ExitGame,
            Err(e) => ChoiceResult::Error(e.to_string()),
        }
    }

    fn choose_attackers(
        &mut self,
        view: &GameStateView,
        available_creatures: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        // Build options - each creature can attack or not
        let mut options = vec!["Done selecting attackers".to_string()];
        for &card_id in available_creatures {
            options.push(format!("Attack with: {}", self.format_card(view, card_id)));
        }

        // Compute state hash
        let state_hash = self.compute_view_hash(view);

        // Send request
        let choice_type = ChoiceType::Attackers {
            available_count: available_creatures.len(),
        };

        // Multi-select for attackers: indices contains all selected attacker indices
        // Index 0 means "done selecting" (no more attackers), indices 1..N are creature indices
        match self.request_choice(view, choice_type, options, state_hash) {
            Ok(indices) if indices.is_empty() || indices == vec![0] => {
                self.increment_choice_seq();
                ChoiceResult::Ok(SmallVec::new()) // No attackers
            }
            Ok(indices) => {
                self.increment_choice_seq();
                // Convert indices to CardIds (indices are 1-based, 0 is "done")
                let mut attackers = SmallVec::new();
                for &idx in &indices {
                    if idx == 0 {
                        continue; // Skip "done selecting" marker
                    }
                    let creature_idx = idx - 1;
                    if creature_idx < available_creatures.len() {
                        attackers.push(available_creatures[creature_idx]);
                    } else {
                        return ChoiceResult::Error(format!("Invalid attacker index {} from network", idx));
                    }
                }
                ChoiceResult::Ok(attackers)
            }
            Err(NetworkError::Disconnected) => ChoiceResult::ExitGame,
            Err(e) => ChoiceResult::Error(e.to_string()),
        }
    }

    fn choose_blockers(
        &mut self,
        view: &GameStateView,
        available_blockers: &[CardId],
        attackers: &[CardId],
    ) -> ChoiceResult<SmallVec<[(CardId, CardId); 8]>> {
        // Build options for each blocker-attacker pair
        let mut options = vec!["Done selecting blockers".to_string()];
        for &blocker in available_blockers {
            for &attacker in attackers {
                options.push(format!(
                    "{} blocks {}",
                    self.format_card(view, blocker),
                    self.format_card(view, attacker)
                ));
            }
        }

        // Compute state hash
        let state_hash = self.compute_view_hash(view);

        // Send request
        let choice_type = ChoiceType::Blockers {
            attacker_count: attackers.len(),
            blocker_count: available_blockers.len(),
        };

        // Multi-select for blockers: indices contains all selected blocker-attacker pairs
        // Index 0 means "done selecting", indices 1..N encode (blocker, attacker) pairs
        match self.request_choice(view, choice_type, options, state_hash) {
            Ok(indices) if indices.is_empty() || indices == vec![0] => {
                self.increment_choice_seq();
                ChoiceResult::Ok(SmallVec::new()) // No blockers
            }
            Ok(indices) => {
                self.increment_choice_seq();
                // Convert indices to (blocker, attacker) pairs
                let mut blocks = SmallVec::new();
                for &idx in &indices {
                    if idx == 0 {
                        continue; // Skip "done selecting" marker
                    }
                    // Decode blocker-attacker pair from index
                    let pair_idx = idx - 1;
                    let blocker_idx = pair_idx / attackers.len();
                    let attacker_idx = pair_idx % attackers.len();
                    if blocker_idx < available_blockers.len() && attacker_idx < attackers.len() {
                        blocks.push((available_blockers[blocker_idx], attackers[attacker_idx]));
                    } else {
                        return ChoiceResult::Error(format!("Invalid blocker index {} from network", idx));
                    }
                }
                ChoiceResult::Ok(blocks)
            }
            Err(NetworkError::Disconnected) => ChoiceResult::ExitGame,
            Err(e) => ChoiceResult::Error(e.to_string()),
        }
    }

    fn choose_damage_assignment_order(
        &mut self,
        view: &GameStateView,
        attacker: CardId,
        blockers: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        // Build options for damage order
        let options: Vec<String> = blockers
            .iter()
            .map(|&card_id| format!("Assign damage first to: {}", self.format_card(view, card_id)))
            .collect();

        // Compute state hash
        let state_hash = self.compute_view_hash(view);

        // Send request
        let choice_type = ChoiceType::DamageOrder {
            attacker,
            blocker_count: blockers.len(),
        };

        // Multi-select for damage order: indices specify the full ordering
        match self.request_choice(view, choice_type, options, state_hash) {
            Ok(indices) => {
                self.increment_choice_seq();
                // If indices specify a full ordering, use it directly
                // Otherwise fall back to putting first index at front
                if indices.len() == blockers.len() {
                    // Full ordering provided
                    let mut result = SmallVec::new();
                    for &idx in &indices {
                        if idx < blockers.len() {
                            result.push(blockers[idx]);
                        } else {
                            return ChoiceResult::Error(format!("Invalid damage order index {} from network", idx));
                        }
                    }
                    ChoiceResult::Ok(result)
                } else {
                    // Single index: put that blocker first, others follow in original order
                    let idx = indices.first().copied().unwrap_or(0);
                    if idx < blockers.len() {
                        let mut result = SmallVec::new();
                        result.push(blockers[idx]);
                        for (i, &blocker) in blockers.iter().enumerate() {
                            if i != idx {
                                result.push(blocker);
                            }
                        }
                        ChoiceResult::Ok(result)
                    } else {
                        ChoiceResult::Error("Invalid damage order index from network".to_string())
                    }
                }
            }
            Err(NetworkError::Disconnected) => ChoiceResult::ExitGame,
            Err(e) => ChoiceResult::Error(e.to_string()),
        }
    }

    fn choose_cards_to_discard(
        &mut self,
        view: &GameStateView,
        hand: &[CardId],
        count: usize,
    ) -> ChoiceResult<SmallVec<[CardId; 7]>> {
        // Build options for each card in hand
        let options: Vec<String> = hand
            .iter()
            .map(|&card_id| format!("Discard: {}", self.format_card(view, card_id)))
            .collect();

        // Compute state hash
        let state_hash = self.compute_view_hash(view);

        // Send request
        let choice_type = ChoiceType::Discard { count };

        // Multi-select for discarding multiple cards
        match self.request_choice(view, choice_type, options, state_hash) {
            Ok(indices) => {
                self.increment_choice_seq();
                let mut discards = SmallVec::new();
                for &idx in &indices {
                    if idx < hand.len() {
                        discards.push(hand[idx]);
                    } else {
                        return ChoiceResult::Error(format!("Invalid discard index {} from network", idx));
                    }
                }
                ChoiceResult::Ok(discards)
            }
            Err(NetworkError::Disconnected) => ChoiceResult::ExitGame,
            Err(e) => ChoiceResult::Error(e.to_string()),
        }
    }

    fn choose_from_library(&mut self, view: &GameStateView, valid_cards: &[CardId]) -> ChoiceResult<Option<CardId>> {
        // Build options
        let mut options = vec!["Decline to find".to_string()];
        for &card_id in valid_cards {
            options.push(format!("Choose: {}", self.format_card(view, card_id)));
        }

        // Compute state hash
        let state_hash = self.compute_view_hash(view);

        // Send request
        let choice_type = ChoiceType::LibrarySearch {
            valid_count: valid_cards.len(),
        };

        match self.request_choice(view, choice_type, options, state_hash) {
            Ok(indices) if indices.first() == Some(&0) => {
                self.increment_choice_seq();
                ChoiceResult::Ok(None) // Declined to find
            }
            Ok(indices) => {
                self.increment_choice_seq();
                let idx = indices.first().copied().unwrap_or(0);
                let card_idx = idx - 1;
                if card_idx < valid_cards.len() {
                    ChoiceResult::Ok(Some(valid_cards[card_idx]))
                } else {
                    ChoiceResult::Error("Invalid library search index from network".to_string())
                }
            }
            Err(NetworkError::Disconnected) => ChoiceResult::ExitGame,
            Err(e) => ChoiceResult::Error(e.to_string()),
        }
    }

    fn choose_permanents_to_sacrifice(
        &mut self,
        view: &GameStateView,
        valid_permanents: &[CardId],
        count: usize,
        card_type_description: &str,
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        // Build options
        let options: Vec<String> = valid_permanents
            .iter()
            .map(|&card_id| format!("Sacrifice: {}", self.format_card(view, card_id)))
            .collect();

        // Compute state hash
        let state_hash = self.compute_view_hash(view);

        // Send request
        let choice_type = ChoiceType::Sacrifice {
            valid_count: valid_permanents.len(),
            count,
            card_type_description: card_type_description.to_string(),
        };

        // Request choice (client sends all selections in one message for multi-select)
        match self.request_choice(view, choice_type, options, state_hash) {
            Ok(indices) => {
                self.increment_choice_seq();
                let mut sacrifices = SmallVec::new();
                for idx in indices {
                    if idx < valid_permanents.len() {
                        let card_id = valid_permanents[idx];
                        if !sacrifices.contains(&card_id) {
                            sacrifices.push(card_id);
                        }
                    } else {
                        return ChoiceResult::Error(format!(
                            "Invalid sacrifice index {} (max {})",
                            idx,
                            valid_permanents.len() - 1
                        ));
                    }
                }
                ChoiceResult::Ok(sacrifices)
            }
            Err(NetworkError::Disconnected) => ChoiceResult::ExitGame,
            Err(e) => ChoiceResult::Error(e.to_string()),
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
        // TODO: Add network protocol support for mode selection (send ChoiceRequest)
        // For now, default to first N modes
        ChoiceResult::Ok((0..mode_count.min(mode_descriptions.len())).collect())
    }

    fn on_priority_passed(&mut self, _view: &GameStateView) {
        // No action needed for network controller
    }

    fn on_game_end(&mut self, _view: &GameStateView, _won: bool) {
        // The WebSocket handler will send GameEnded message separately
    }

    fn get_snapshot_state(&self) -> Option<serde_json::Value> {
        // Network controller has no persistent state to snapshot
        None
    }

    fn has_more_choices(&self) -> bool {
        // Network controller always has more choices (until disconnected)
        true
    }

    fn get_controller_type(&self) -> ControllerType {
        // Network controller must not auto-pass - always go through ChoiceRequest flow
        // so clients are notified even when there are 0 available abilities
        ControllerType::Network
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// TESTS
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::GameState;

    /// Create a minimal game state for testing
    fn create_test_game_state() -> GameState {
        GameState::new_two_player_with_capacity(
            "TestPlayer1".to_string(),
            "TestPlayer2".to_string(),
            20,
            60, // deck capacity
        )
    }

    #[test]
    fn test_network_controller_creation() {
        let (request_tx, _request_rx) = mpsc::channel();
        let (_response_tx, response_rx) = mpsc::channel();
        let shared_reveal_index = Arc::new(AtomicUsize::new(0));

        let controller = NetworkController::new(PlayerId::new(1), request_tx, response_rx, shared_reveal_index);

        assert_eq!(controller.player_id(), PlayerId::new(1));
        assert_eq!(controller.choice_seq, 0);
    }

    #[test]
    fn test_choice_request_response() {
        let (request_tx, request_rx) = mpsc::channel();
        let (response_tx, response_rx) = mpsc::channel();
        let shared_reveal_index = Arc::new(AtomicUsize::new(0));

        let mut controller = NetworkController::new(PlayerId::new(1), request_tx, response_rx, shared_reveal_index);

        // Create a test game state and view
        let game = create_test_game_state();
        let view = GameStateView::new(&game, PlayerId::new(1));

        // Spawn a thread to simulate the network handler
        std::thread::spawn(move || {
            let request: ChoiceRequest = request_rx.recv().unwrap();
            assert_eq!(request.choice_seq, 1);
            assert_eq!(request.options.len(), 3);
            // Verify reveals field exists (should be empty for this test)
            assert!(request.reveals.is_empty());

            response_tx
                .send(ChoiceResponse {
                    choice_seq: 1,
                    choice_indices: vec![2],
                })
                .unwrap();
        });

        let options = vec!["Option A".to_string(), "Option B".to_string(), "Option C".to_string()];

        let result = controller.request_choice(&view, ChoiceType::Priority { available_count: 2 }, options, 0x12345678);

        assert_eq!(result.unwrap(), vec![2]);
    }

    #[test]
    fn test_sequence_mismatch_error() {
        let (request_tx, request_rx) = mpsc::channel();
        let (response_tx, response_rx) = mpsc::channel();
        let shared_reveal_index = Arc::new(AtomicUsize::new(0));

        let mut controller = NetworkController::new(PlayerId::new(1), request_tx, response_rx, shared_reveal_index);

        // Create a test game state and view
        let game = create_test_game_state();
        let view = GameStateView::new(&game, PlayerId::new(1));

        std::thread::spawn(move || {
            let _request: ChoiceRequest = request_rx.recv().unwrap();
            // Send response with wrong sequence number
            response_tx
                .send(ChoiceResponse {
                    choice_seq: 999, // Wrong sequence
                    choice_indices: vec![0],
                })
                .unwrap();
        });

        let result = controller.request_choice(
            &view,
            ChoiceType::Priority { available_count: 0 },
            vec!["Pass".to_string()],
            0,
        );

        match result {
            Err(NetworkError::SequenceMismatch { expected: 1, got: 999 }) => {}
            _ => panic!("Expected SequenceMismatch error"),
        }
    }

    #[test]
    fn test_invalid_choice_clamped() {
        let (request_tx, request_rx) = mpsc::channel();
        let (response_tx, response_rx) = mpsc::channel();
        let shared_reveal_index = Arc::new(AtomicUsize::new(0));

        let mut controller = NetworkController::new(PlayerId::new(1), request_tx, response_rx, shared_reveal_index);

        // Create a test game state and view
        let game = create_test_game_state();
        let view = GameStateView::new(&game, PlayerId::new(1));

        std::thread::spawn(move || {
            let _request: ChoiceRequest = request_rx.recv().unwrap();
            response_tx
                .send(ChoiceResponse {
                    choice_seq: 1,
                    choice_indices: vec![10], // Invalid - only 2 options, should clamp to 0
                })
                .unwrap();
        });

        let result = controller.request_choice(
            &view,
            ChoiceType::Priority { available_count: 1 },
            vec!["Pass".to_string(), "Play land".to_string()],
            0,
        );

        // Invalid index 10 should be clamped to 0, not return an error
        assert_eq!(result.unwrap(), vec![0]);
    }

    #[test]
    fn test_network_error_display() {
        assert_eq!(NetworkError::Disconnected.to_string(), "Client disconnected");
        assert_eq!(NetworkError::Timeout.to_string(), "Request timed out");
        assert_eq!(
            NetworkError::SequenceMismatch { expected: 1, got: 2 }.to_string(),
            "Sequence mismatch: expected 1, got 2"
        );
        assert_eq!(
            NetworkError::InvalidChoice { max: 5, got: 10 }.to_string(),
            "Invalid choice index: 10 (max 5)"
        );
    }
}
