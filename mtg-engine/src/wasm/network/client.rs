//! WASM Network Client State Machine
//!
//! Manages connection state and message queues for browser-based network gameplay.
//! Unlike the native client which blocks on channels, this uses queues that
//! JavaScript can fill from WebSocket callbacks.

use crate::core::{PlayerId, SpellAbility};
use crate::network::{CardReveal, ChoiceType, ClientMessage, DeckSubmission, RevealReason, ServerMessage};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

/// Connection state for the WASM network client
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkState {
    /// Not connected to any server
    Disconnected,
    /// WebSocket connection established, awaiting authentication
    Connecting,
    /// Authenticated, waiting for opponent to join
    WaitingForOpponent,
    /// Game is in progress
    InGame,
    /// Game has ended
    GameEnded,
    /// Error state
    Error,
}

impl NetworkState {
    /// Get a string representation for JavaScript
    pub fn as_str(&self) -> &'static str {
        match self {
            NetworkState::Disconnected => "disconnected",
            NetworkState::Connecting => "connecting",
            NetworkState::WaitingForOpponent => "waiting_for_opponent",
            NetworkState::InGame => "in_game",
            NetworkState::GameEnded => "game_ended",
            NetworkState::Error => "error",
        }
    }
}

/// Data from an OpponentChoice message
#[derive(Debug, Clone)]
pub struct OpponentChoiceData {
    pub choice_seq: u32,
    pub choice_index: usize,
    pub description: String,
    pub spell_ability: Option<SpellAbility>,
    pub action_count: u64,
}

/// Data from a ChoiceRequest message
#[derive(Debug, Clone)]
pub struct ChoiceRequestData {
    pub choice_seq: u32,
    pub choice_type: ChoiceType,
    pub options: Vec<String>,
    pub state_hash: u64,
    pub action_count: u64,
}

/// WASM Network Client
///
/// Manages connection state and provides non-blocking access to server messages.
/// JavaScript fills the incoming queues via WebSocket callbacks, and polls
/// the outgoing queue to send messages.
pub struct WasmNetworkClient {
    /// Current connection state
    state: NetworkState,

    /// Our player ID (set after GameStarted)
    our_player_id: Option<PlayerId>,

    /// Opponent's player ID
    opponent_id: Option<PlayerId>,

    /// Opponent's name
    opponent_name: Option<String>,

    /// Whether network debug mode is enabled
    network_debug: bool,

    /// Queued CardRevealed messages (processed before draws)
    pending_reveals: VecDeque<(PlayerId, CardReveal, RevealReason)>,

    /// Queued OpponentChoice messages (consumed by WasmRemoteController)
    opponent_choices: VecDeque<OpponentChoiceData>,

    /// Current ChoiceRequest from server (if any)
    current_choice_request: Option<ChoiceRequestData>,

    /// Whether the last SubmitChoice was acknowledged
    choice_acknowledged: bool,

    /// Outbound message queue (JavaScript polls and sends)
    outbound_queue: VecDeque<String>,

    /// Last error message
    last_error: Option<String>,

    /// Game winner (if game ended)
    winner: Option<Option<PlayerId>>,

    // Connection parameters (set before connecting)
    /// Server URL
    server_url: Option<String>,
    /// Password for authentication
    password: Option<String>,
    /// Player name
    player_name: Option<String>,
    /// Deck JSON for submission
    deck_json: Option<String>,
}

impl WasmNetworkClient {
    /// Create a new WASM network client
    pub fn new() -> Self {
        Self {
            state: NetworkState::Disconnected,
            our_player_id: None,
            opponent_id: None,
            opponent_name: None,
            network_debug: false,
            pending_reveals: VecDeque::new(),
            opponent_choices: VecDeque::new(),
            current_choice_request: None,
            choice_acknowledged: true, // Start acknowledged (no pending)
            outbound_queue: VecDeque::new(),
            last_error: None,
            winner: None,
            server_url: None,
            password: None,
            player_name: None,
            deck_json: None,
        }
    }

    /// Get current connection state
    pub fn state(&self) -> NetworkState {
        self.state
    }

    /// Get our player ID (if assigned)
    pub fn our_player_id(&self) -> Option<PlayerId> {
        self.our_player_id
    }

    /// Get opponent's player ID (if known)
    pub fn opponent_id(&self) -> Option<PlayerId> {
        self.opponent_id
    }

    /// Get opponent's name (if known)
    pub fn opponent_name(&self) -> Option<&str> {
        self.opponent_name.as_deref()
    }

    /// Get the last error message
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    /// Get the game winner (if game ended)
    pub fn winner(&self) -> Option<Option<PlayerId>> {
        self.winner
    }

    /// Check if network debug mode is enabled
    pub fn is_network_debug(&self) -> bool {
        self.network_debug
    }

    /// Set connection parameters before connecting
    pub fn set_connection_params(&mut self, server_url: &str, password: &str, player_name: &str, deck_json: &str) {
        self.server_url = Some(server_url.to_string());
        self.password = Some(password.to_string());
        self.player_name = Some(player_name.to_string());
        self.deck_json = Some(deck_json.to_string());
        log::info!("WasmNetworkClient: Connection params set for {}", player_name);
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // CONNECTION LIFECYCLE
    // ═══════════════════════════════════════════════════════════════════════════

    /// Called when WebSocket connection opens
    ///
    /// Automatically sends authentication if connection params are set.
    pub fn on_open(&mut self) {
        log::info!("WasmNetworkClient: WebSocket connected");
        self.state = NetworkState::Connecting;

        // Auto-authenticate if we have stored params
        if let (Some(password), Some(player_name), Some(deck_json)) =
            (self.password.clone(), self.player_name.clone(), self.deck_json.clone())
        {
            match serde_json::from_str::<DeckSubmission>(&deck_json) {
                Ok(deck) => {
                    self.authenticate(&password, &player_name, deck);
                    log::info!("WasmNetworkClient: Auto-authenticated as {}", player_name);
                }
                Err(e) => {
                    log::error!("WasmNetworkClient: Failed to parse deck JSON: {}", e);
                    self.last_error = Some(format!("Invalid deck JSON: {}", e));
                    self.state = NetworkState::Error;
                }
            }
        }
    }

    /// Called when WebSocket connection closes
    pub fn on_close(&mut self) {
        log::info!("WasmNetworkClient: WebSocket closed");
        if self.state != NetworkState::GameEnded {
            self.state = NetworkState::Disconnected;
            self.last_error = Some("Connection closed".to_string());
        }
    }

    /// Called when WebSocket encounters an error
    pub fn on_error(&mut self, error: &str) {
        log::error!("WasmNetworkClient: WebSocket error: {}", error);
        self.state = NetworkState::Error;
        self.last_error = Some(error.to_string());
    }

    /// Queue authentication message
    pub fn authenticate(&mut self, password: &str, player_name: &str, deck: DeckSubmission) {
        let msg = ClientMessage::Authenticate {
            password: password.to_string(),
            player_name: player_name.to_string(),
            deck,
        };
        self.queue_outbound(msg);
    }

    /// Queue disconnect message
    pub fn disconnect(&mut self) {
        let msg = ClientMessage::Disconnect;
        self.queue_outbound(msg);
        self.state = NetworkState::Disconnected;
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // MESSAGE HANDLING
    // ═══════════════════════════════════════════════════════════════════════════

    /// Process a server message (called from JavaScript after receiving)
    ///
    /// Returns true if the message was processed successfully.
    pub fn on_message(&mut self, json: &str) -> bool {
        match serde_json::from_str::<ServerMessage>(json) {
            Ok(msg) => {
                self.handle_server_message(msg);
                true
            }
            Err(e) => {
                log::error!("WasmNetworkClient: Failed to parse message: {}", e);
                self.last_error = Some(format!("Parse error: {}", e));
                false
            }
        }
    }

    /// Handle a parsed server message
    fn handle_server_message(&mut self, msg: ServerMessage) {
        match msg {
            ServerMessage::AuthResult {
                success,
                error,
                your_player_id,
            } => {
                if success {
                    log::info!("WasmNetworkClient: Authenticated as {:?}", your_player_id);
                    self.our_player_id = your_player_id;
                    self.state = NetworkState::WaitingForOpponent;
                } else {
                    log::error!("WasmNetworkClient: Auth failed: {:?}", error);
                    self.state = NetworkState::Error;
                    self.last_error = error;
                }
            }

            ServerMessage::WaitingForOpponent => {
                log::info!("WasmNetworkClient: Waiting for opponent");
                self.state = NetworkState::WaitingForOpponent;
            }

            ServerMessage::GameStarted {
                your_player_id,
                opponent_name,
                opening_hand,
                network_debug,
                ..
            } => {
                log::info!(
                    "WasmNetworkClient: Game started! We are {:?}, opponent: {}",
                    your_player_id,
                    opponent_name
                );
                self.our_player_id = Some(your_player_id);
                self.opponent_id = Some(if your_player_id.as_u32() == 0 {
                    PlayerId::new(1)
                } else {
                    PlayerId::new(0)
                });
                self.opponent_name = Some(opponent_name);
                self.network_debug = network_debug;
                self.state = NetworkState::InGame;

                // Queue opening hand reveals
                for card in opening_hand {
                    self.pending_reveals
                        .push_back((your_player_id, card, RevealReason::OpeningHand));
                }
            }

            ServerMessage::CardRevealed { owner, card, reason } => {
                log::debug!(
                    "WasmNetworkClient: Card revealed - {} ({:?}) for {:?}",
                    card.name,
                    reason,
                    owner
                );
                self.pending_reveals.push_back((owner, card, reason));
            }

            ServerMessage::ChoiceRequest {
                choice_seq,
                choice_type,
                options,
                state_hash,
                action_count,
                ..
            } => {
                log::debug!(
                    "WasmNetworkClient: ChoiceRequest seq={} type={:?} action_count={}",
                    choice_seq,
                    choice_type,
                    action_count
                );
                self.current_choice_request = Some(ChoiceRequestData {
                    choice_seq,
                    choice_type,
                    options,
                    state_hash,
                    action_count,
                });
            }

            ServerMessage::OpponentChoice {
                choice_seq,
                choice_index,
                description,
                spell_ability,
                action_count,
                ..
            } => {
                log::debug!(
                    "WasmNetworkClient: OpponentChoice seq={} index={} desc={}",
                    choice_seq,
                    choice_index,
                    description
                );
                self.opponent_choices.push_back(OpponentChoiceData {
                    choice_seq,
                    choice_index,
                    description,
                    spell_ability,
                    action_count,
                });
            }

            ServerMessage::ChoiceAccepted { choice_seq, .. } => {
                log::debug!("WasmNetworkClient: ChoiceAccepted seq={}", choice_seq);
                self.choice_acknowledged = true;
            }

            ServerMessage::GameEnded { winner, reason, .. } => {
                log::info!(
                    "WasmNetworkClient: Game ended - winner: {:?}, reason: {:?}",
                    winner,
                    reason
                );
                self.state = NetworkState::GameEnded;
                self.winner = Some(winner);
            }

            ServerMessage::SyncError { details, fatal } => {
                log::error!("WasmNetworkClient: Sync error (fatal={}): {:?}", fatal, details);
                if fatal {
                    self.state = NetworkState::Error;
                }
                self.last_error = Some(format!("Sync error: {:?}", details));
            }

            ServerMessage::Error { message, fatal } => {
                log::error!("WasmNetworkClient: Server error (fatal={}): {}", fatal, message);
                if fatal {
                    self.state = NetworkState::Error;
                }
                self.last_error = Some(message);
            }

            ServerMessage::Pong { .. } => {
                // Keepalive response, ignore
            }

            #[cfg(debug_assertions)]
            ServerMessage::DebugStateDump { .. } => {
                // Debug info, just log
                log::debug!("WasmNetworkClient: Received debug state dump");
            }
        }
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // MESSAGE QUEUE ACCESS (for controllers)
    // ═══════════════════════════════════════════════════════════════════════════

    /// Check if a ChoiceRequest is pending
    pub fn has_choice_request(&self) -> bool {
        self.current_choice_request.is_some()
    }

    /// Get the current ChoiceRequest (without consuming)
    pub fn peek_choice_request(&self) -> Option<&ChoiceRequestData> {
        self.current_choice_request.as_ref()
    }

    /// Consume the current ChoiceRequest
    pub fn take_choice_request(&mut self) -> Option<ChoiceRequestData> {
        self.current_choice_request.take()
    }

    /// Check if choice has been acknowledged
    pub fn is_choice_acknowledged(&self) -> bool {
        self.choice_acknowledged
    }

    /// Check if an OpponentChoice is pending
    pub fn has_opponent_choice(&self) -> bool {
        !self.opponent_choices.is_empty()
    }

    /// Pop the next OpponentChoice
    pub fn pop_opponent_choice(&mut self) -> Option<OpponentChoiceData> {
        self.opponent_choices.pop_front()
    }

    /// Check if reveals are pending
    pub fn has_pending_reveals(&self) -> bool {
        !self.pending_reveals.is_empty()
    }

    /// Pop the next pending reveal
    pub fn pop_reveal(&mut self) -> Option<(PlayerId, CardReveal, RevealReason)> {
        self.pending_reveals.pop_front()
    }

    /// Drain all pending reveals
    pub fn drain_reveals(&mut self) -> Vec<(PlayerId, CardReveal, RevealReason)> {
        self.pending_reveals.drain(..).collect()
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // OUTBOUND MESSAGE QUEUE
    // ═══════════════════════════════════════════════════════════════════════════

    /// Queue a SubmitChoice response
    pub fn submit_choice(&mut self, choice_index: usize, action_count: u64, state_hash: Option<u64>) {
        if let Some(ref request) = self.current_choice_request {
            let msg = ClientMessage::SubmitChoice {
                choice_seq: request.choice_seq,
                choice_index,
                action_count,
                timestamp_ms: crate::network::now_ms(),
                client_state_hash: state_hash,
                debug_info: None,
            };
            self.queue_outbound(msg);
            self.choice_acknowledged = false;
            self.current_choice_request = None;
        } else {
            log::warn!("WasmNetworkClient: submit_choice called without pending request");
        }
    }

    /// Queue a ping message
    pub fn ping(&mut self) {
        let msg = ClientMessage::Ping {
            timestamp_ms: crate::network::now_ms(),
        };
        self.queue_outbound(msg);
    }

    /// Queue an outbound message
    fn queue_outbound(&mut self, msg: ClientMessage) {
        match serde_json::to_string(&msg) {
            Ok(json) => {
                self.outbound_queue.push_back(json);
            }
            Err(e) => {
                log::error!("WasmNetworkClient: Failed to serialize message: {}", e);
            }
        }
    }

    /// Get the next outbound message (JavaScript polls this)
    pub fn get_outbound_message(&mut self) -> Option<String> {
        self.outbound_queue.pop_front()
    }

    /// Check if there are outbound messages
    pub fn has_outbound_messages(&self) -> bool {
        !self.outbound_queue.is_empty()
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // RESET
    // ═══════════════════════════════════════════════════════════════════════════

    /// Reset the client state for a new game
    pub fn reset(&mut self) {
        self.state = NetworkState::Disconnected;
        self.our_player_id = None;
        self.opponent_id = None;
        self.opponent_name = None;
        self.network_debug = false;
        self.pending_reveals.clear();
        self.opponent_choices.clear();
        self.current_choice_request = None;
        self.choice_acknowledged = true;
        self.outbound_queue.clear();
        self.last_error = None;
        self.winner = None;
    }
}

impl Default for WasmNetworkClient {
    fn default() -> Self {
        Self::new()
    }
}

/// Shared reference to WasmNetworkClient for use across controllers
pub type SharedNetworkClient = Rc<RefCell<WasmNetworkClient>>;

/// Create a new shared network client
pub fn new_shared_client() -> SharedNetworkClient {
    Rc::new(RefCell::new(WasmNetworkClient::new()))
}
