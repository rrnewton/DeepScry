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
use std::sync::mpsc;

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
}

/// Response received from the network handler
#[derive(Debug, Clone)]
pub struct ChoiceResponse {
    /// Sequence number (must match request)
    pub choice_seq: u32,
    /// Index of the chosen option
    pub choice_index: usize,
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
}

impl NetworkController {
    /// Create a new network controller
    pub fn new(
        player_id: PlayerId,
        request_tx: mpsc::Sender<ChoiceRequest>,
        response_rx: mpsc::Receiver<ChoiceResponse>,
    ) -> Self {
        NetworkController {
            player_id,
            request_tx,
            response_rx,
            choice_seq: 0,
        }
    }

    /// Send a choice request and wait for response
    ///
    /// This also collects any card reveals since this player's last choice
    /// and includes them in the request.
    fn request_choice(
        &self,
        view: &GameStateView,
        choice_type: ChoiceType,
        options: Vec<String>,
        state_hash: u64,
    ) -> Result<usize, NetworkError> {
        // Collect reveals since last choice
        let reveals = self.collect_reveals_since_last_choice(view);

        // Get action count from GameState undo log for synchronization
        let action_count = view.action_count() as u64;

        let request = ChoiceRequest {
            choice_seq: self.choice_seq + 1,
            choice_type,
            options: options.clone(),
            state_hash,
            action_count,
            reveals,
        };

        // Send request
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

        // Verify choice is valid
        if response.choice_index >= options.len() {
            return Err(NetworkError::InvalidChoice {
                max: options.len(),
                got: response.choice_index,
            });
        }

        Ok(response.choice_index)
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
    /// a `ChoicePoint` for this player (or reach the start of the log).
    /// Returns all `MoveCard` actions from Library that this player should see.
    ///
    /// A player sees a reveal if:
    /// - They own the card (e.g., their own draw)
    /// - The card moved to a public zone (battlefield, graveyard, stack, exile)
    fn collect_reveals_since_last_choice(&self, view: &GameStateView) -> Vec<CardRevealInfo> {
        let actions = view.undo_log_actions();
        let mut reveals = Vec::new();

        // Scan backwards from the end of the log
        for action in actions.iter().rev() {
            match action {
                // Stop when we hit this player's last choice
                GameAction::ChoicePoint { player_id, .. } if *player_id == self.player_id => {
                    break;
                }
                // Collect card moves from library
                GameAction::MoveCard {
                    card_id,
                    from_zone: Zone::Library,
                    to_zone,
                    owner,
                } => {
                    // Player sees reveal if:
                    // 1. They own the card (their draw)
                    // 2. Card went to a public zone (all players see)
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

        // Reverse to get chronological order
        reveals.reverse();
        reveals
    }

    /// Compute a simple state hash for verification
    ///
    /// This uses a simplified approach since we can't easily access
    /// the full GameState from GameStateView. In production, the server
    /// would compute the proper network state hash.
    fn compute_simple_hash(&self, view: &GameStateView) -> u64 {
        // Use turn number and player life totals as a simple hash
        // This is a placeholder - in production we'd use compute_network_state_hash
        let mut hash: u64 = 0;
        hash = hash.wrapping_add(view.turn_number() as u64);
        hash = hash.wrapping_mul(31).wrapping_add(view.life() as u64);
        hash
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
        let state_hash = self.compute_simple_hash(view);

        // Send request and get response
        let choice_type = ChoiceType::Priority {
            available_count: available.len(),
        };

        match self.request_choice(view, choice_type, options, state_hash) {
            Ok(0) => {
                self.increment_choice_seq();
                ChoiceResult::Ok(None) // Pass priority
            }
            Ok(idx) => {
                self.increment_choice_seq();
                // Subtract 1 because option 0 is "Pass priority"
                let ability_idx = idx - 1;
                if ability_idx < available.len() {
                    ChoiceResult::Ok(Some(available[ability_idx].clone()))
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
        let state_hash = self.compute_simple_hash(view);

        // Send request
        let choice_type = ChoiceType::Targets {
            spell_id: spell,
            target_count: 1, // FIXME-UNFINISHED: Support multiple targets
        };

        match self.request_choice(view, choice_type, options, state_hash) {
            Ok(idx) => {
                self.increment_choice_seq();
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
        let state_hash = self.compute_simple_hash(view);

        // Send request
        let choice_type = ChoiceType::ManaSources { cost: *cost };

        // FIXME-UNFINISHED: Needs multi-select for paying costs with multiple sources
        match self.request_choice(view, choice_type, options, state_hash) {
            Ok(idx) => {
                self.increment_choice_seq();
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
        let state_hash = self.compute_simple_hash(view);

        // Send request
        let choice_type = ChoiceType::Attackers {
            available_count: available_creatures.len(),
        };

        // FIXME-UNFINISHED: Support multi-select for attackers (currently single selection)
        match self.request_choice(view, choice_type, options, state_hash) {
            Ok(0) => {
                self.increment_choice_seq();
                ChoiceResult::Ok(SmallVec::new()) // No attackers
            }
            Ok(idx) => {
                self.increment_choice_seq();
                let creature_idx = idx - 1;
                if creature_idx < available_creatures.len() {
                    ChoiceResult::Ok(SmallVec::from_slice(&[available_creatures[creature_idx]]))
                } else {
                    ChoiceResult::Error("Invalid attacker index from network".to_string())
                }
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
        let state_hash = self.compute_simple_hash(view);

        // Send request
        let choice_type = ChoiceType::Blockers {
            attacker_count: attackers.len(),
            blocker_count: available_blockers.len(),
        };

        // FIXME-UNFINISHED: Support multi-select for blockers (currently single selection)
        match self.request_choice(view, choice_type, options, state_hash) {
            Ok(0) => {
                self.increment_choice_seq();
                ChoiceResult::Ok(SmallVec::new()) // No blockers
            }
            Ok(idx) => {
                self.increment_choice_seq();
                // Decode blocker-attacker pair from index
                let pair_idx = idx - 1;
                let blocker_idx = pair_idx / attackers.len();
                let attacker_idx = pair_idx % attackers.len();
                if blocker_idx < available_blockers.len() && attacker_idx < attackers.len() {
                    ChoiceResult::Ok(SmallVec::from_slice(&[(
                        available_blockers[blocker_idx],
                        attackers[attacker_idx],
                    )]))
                } else {
                    ChoiceResult::Error("Invalid blocker index from network".to_string())
                }
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
        let state_hash = self.compute_simple_hash(view);

        // Send request
        let choice_type = ChoiceType::DamageOrder {
            attacker,
            blocker_count: blockers.len(),
        };

        // FIXME-UNFINISHED: Support full ordering of all blockers (only picks first currently)
        match self.request_choice(view, choice_type, options, state_hash) {
            Ok(idx) => {
                self.increment_choice_seq();
                if idx < blockers.len() {
                    // Return all blockers with the chosen one first
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
        let state_hash = self.compute_simple_hash(view);

        // Send request
        let choice_type = ChoiceType::Discard { count };

        // FIXME-UNFINISHED: Support multi-select for discarding multiple cards
        match self.request_choice(view, choice_type, options, state_hash) {
            Ok(idx) => {
                self.increment_choice_seq();
                if idx < hand.len() {
                    ChoiceResult::Ok(SmallVec::from_slice(&[hand[idx]]))
                } else {
                    ChoiceResult::Error("Invalid discard index from network".to_string())
                }
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
        let state_hash = self.compute_simple_hash(view);

        // Send request
        let choice_type = ChoiceType::LibrarySearch {
            valid_count: valid_cards.len(),
        };

        match self.request_choice(view, choice_type, options, state_hash) {
            Ok(0) => {
                self.increment_choice_seq();
                ChoiceResult::Ok(None) // Declined to find
            }
            Ok(idx) => {
                self.increment_choice_seq();
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
        // FIXME-UNFINISHED: Add ControllerType::Network variant
        ControllerType::Zero
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

        let controller = NetworkController::new(PlayerId::new(1), request_tx, response_rx);

        assert_eq!(controller.player_id(), PlayerId::new(1));
        assert_eq!(controller.choice_seq, 0);
    }

    #[test]
    fn test_choice_request_response() {
        let (request_tx, request_rx) = mpsc::channel();
        let (response_tx, response_rx) = mpsc::channel();

        let controller = NetworkController::new(PlayerId::new(1), request_tx, response_rx);

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
                    choice_index: 2,
                })
                .unwrap();
        });

        let options = vec!["Option A".to_string(), "Option B".to_string(), "Option C".to_string()];

        let result = controller.request_choice(&view, ChoiceType::Priority { available_count: 2 }, options, 0x12345678);

        assert_eq!(result.unwrap(), 2);
    }

    #[test]
    fn test_sequence_mismatch_error() {
        let (request_tx, request_rx) = mpsc::channel();
        let (response_tx, response_rx) = mpsc::channel();

        let controller = NetworkController::new(PlayerId::new(1), request_tx, response_rx);

        // Create a test game state and view
        let game = create_test_game_state();
        let view = GameStateView::new(&game, PlayerId::new(1));

        std::thread::spawn(move || {
            let _request: ChoiceRequest = request_rx.recv().unwrap();
            // Send response with wrong sequence number
            response_tx
                .send(ChoiceResponse {
                    choice_seq: 999, // Wrong sequence
                    choice_index: 0,
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
    fn test_invalid_choice_error() {
        let (request_tx, request_rx) = mpsc::channel();
        let (response_tx, response_rx) = mpsc::channel();

        let controller = NetworkController::new(PlayerId::new(1), request_tx, response_rx);

        // Create a test game state and view
        let game = create_test_game_state();
        let view = GameStateView::new(&game, PlayerId::new(1));

        std::thread::spawn(move || {
            let _request: ChoiceRequest = request_rx.recv().unwrap();
            response_tx
                .send(ChoiceResponse {
                    choice_seq: 1,
                    choice_index: 10, // Invalid - only 2 options
                })
                .unwrap();
        });

        let result = controller.request_choice(
            &view,
            ChoiceType::Priority { available_count: 1 },
            vec!["Pass".to_string(), "Play land".to_string()],
            0,
        );

        match result {
            Err(NetworkError::InvalidChoice { max: 2, got: 10 }) => {}
            _ => panic!("Expected InvalidChoice error"),
        }
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
