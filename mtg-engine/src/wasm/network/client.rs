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
    pub choice_indices: Vec<usize>,
    pub description: String,
    pub spell_ability: Option<SpellAbility>,
    pub action_count: u64,
    /// For LibrarySearchByName choices: the specific CardId that was chosen.
    /// Allows the shadow game's WasmRemoteController to know which card moved to hand.
    pub library_search_result: Option<crate::core::CardId>,
    /// Authoritative target CardIds for choices that need them (e.g., damage
    /// assignment among multiple blockers). Used by `WasmRemoteController` to
    /// pick the correct blocker even when the index would point to a different
    /// CardId in the client's view than in the server's view.
    pub target_card_ids: Option<Vec<crate::core::CardId>>,
}

/// Data from a ChoiceRequest message
#[derive(Debug, Clone)]
pub struct ChoiceRequestData {
    pub choice_seq: u32,
    pub choice_type: ChoiceType,
    pub options: Vec<String>,
    pub state_hash: u64,
    pub action_count: u64,
    /// Server's authoritative list of available abilities for Priority choices.
    /// Index 0 is "Pass priority" (None), indices 1+ are the actual abilities.
    /// Used for DESYNC DETECTION: local abilities are validated against these.
    /// Any mismatch is a FATAL error (per NETWORK_ARCHITECTURE.md).
    pub abilities: Option<Vec<Option<SpellAbility>>>,
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

    /// Last submitted choice sequence number (for duplicate detection)
    /// This is stored in the client so it persists across controller instances
    last_submitted_choice_seq: Option<u32>,

    /// Outbound message queue (JavaScript polls and sends)
    outbound_queue: VecDeque<String>,

    /// Last error message
    last_error: Option<String>,

    /// Game winner (if game ended)
    winner: Option<Option<PlayerId>>,

    // Game initialization data (from GameStarted message)
    /// Starting life total
    starting_life: i32,
    /// Initial state hash for verification
    initial_state_hash: u64,
    /// Our library size after drawing
    library_size: usize,
    /// Opponent's library size
    opponent_library_size: usize,
    /// Opponent's hand count
    opponent_hand_count: usize,
    /// CardID ranges for late-binding architecture (mtg-d0jg3)
    /// CRITICAL: WASM must use these ranges to initialize game state
    /// with the same CardIDs as the server for behavioral identity.
    deck_card_ids: Option<crate::network::DeckCardIdRanges>,
    /// Serialized RNG state for deterministic shuffles
    /// Must be deserialized and used to seed the game RNG for identical behavior.
    rng_state: Vec<u8>,
    /// Token definitions that may be created during the game
    token_definitions: std::collections::HashMap<String, crate::loader::CardDefinition>,

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
            last_submitted_choice_seq: None,
            outbound_queue: VecDeque::new(),
            last_error: None,
            winner: None,
            starting_life: 20, // Default, updated by GameStarted
            initial_state_hash: 0,
            library_size: 0,
            opponent_library_size: 0,
            opponent_hand_count: 0,
            deck_card_ids: None,
            rng_state: Vec::new(),
            token_definitions: std::collections::HashMap::new(),
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

    /// Get our player name (as sent to server during authentication)
    pub fn our_name(&self) -> Option<&str> {
        self.player_name.as_deref()
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

    /// Get the starting life total from GameStarted
    pub fn starting_life(&self) -> i32 {
        self.starting_life
    }

    /// Get the initial state hash from GameStarted
    pub fn initial_state_hash(&self) -> u64 {
        self.initial_state_hash
    }

    /// Get our library size after drawing (from GameStarted)
    pub fn library_size(&self) -> usize {
        self.library_size
    }

    /// Get opponent's library size (from GameStarted)
    pub fn opponent_library_size(&self) -> usize {
        self.opponent_library_size
    }

    /// Get opponent's hand count (from GameStarted)
    pub fn opponent_hand_count(&self) -> usize {
        self.opponent_hand_count
    }

    /// Get CardID ranges for late-binding architecture (mtg-d0jg3)
    ///
    /// CRITICAL: WASM must use these ranges to initialize game state
    /// with `init_game_reserve_only()` for behavioral identity with native.
    pub fn deck_card_ids(&self) -> Option<&crate::network::DeckCardIdRanges> {
        self.deck_card_ids.as_ref()
    }

    /// Get serialized RNG state for deterministic shuffles
    ///
    /// Must be deserialized and used to seed the game RNG for identical
    /// shuffle behavior between server and client.
    pub fn rng_state(&self) -> &[u8] {
        &self.rng_state
    }

    /// Get token definitions for token creation
    pub fn token_definitions(&self) -> &std::collections::HashMap<String, crate::loader::CardDefinition> {
        &self.token_definitions
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
    ///
    /// If player_name is empty, server will assign a default name with suffix.
    pub fn authenticate(&mut self, password: &str, player_name: &str, deck: DeckSubmission) {
        let msg = ClientMessage::Authenticate {
            password: password.to_string(),
            player_name: if player_name.is_empty() {
                None
            } else {
                Some(player_name.to_string())
            },
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
                your_name,
            } => {
                if success {
                    log::info!(
                        "WasmNetworkClient: Authenticated as '{}' ({:?})",
                        your_name.as_deref().unwrap_or("<pending>"),
                        your_player_id
                    );
                    self.our_player_id = your_player_id;
                    // Store the assigned name if provided
                    // (it will be None for first player until P2 connects)
                    self.state = NetworkState::WaitingForOpponent;
                } else {
                    log::error!("WasmNetworkClient: Auth failed: {:?}", error);
                    self.state = NetworkState::Error;
                    self.last_error = error;
                }
            }

            ServerMessage::BugReportResult { success, error, .. } => {
                if !success {
                    log::error!("WasmNetworkClient: Bug report submission failed: {:?}", error);
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
                opponent_hand_count,
                library_size,
                opponent_library_size,
                opponent_decklist: _, // Not used in WASM client
                starting_life,
                initial_state_hash,
                network_debug,
                deck_card_ids,
                token_definitions,
                rng_state,
            } => {
                log::info!(
                    "WasmNetworkClient: Game started! We are {:?}, opponent: {}, life: {}",
                    your_player_id,
                    opponent_name,
                    starting_life
                );

                // Log critical sync data (mtg-d0jg3: behavioral identity)
                if let Some(ref ranges) = deck_card_ids {
                    log::info!(
                        "WasmNetworkClient: DeckCardIdRanges received - P1:[{}..{}), P2:[{}..{})",
                        ranges.p1_start,
                        ranges.p1_end,
                        ranges.p2_start,
                        ranges.p2_end
                    );
                } else {
                    log::warn!("WasmNetworkClient: No DeckCardIdRanges - late-binding CardIDs unavailable!");
                }
                if !rng_state.is_empty() {
                    log::info!("WasmNetworkClient: RNG state received ({} bytes)", rng_state.len());
                } else {
                    log::warn!("WasmNetworkClient: No RNG state - shuffles may diverge!");
                }
                if !token_definitions.is_empty() {
                    log::info!(
                        "WasmNetworkClient: {} token definitions received",
                        token_definitions.len()
                    );
                }

                self.our_player_id = Some(your_player_id);
                self.opponent_id = Some(if your_player_id.as_u32() == 0 {
                    PlayerId::new(1)
                } else {
                    PlayerId::new(0)
                });
                self.opponent_name = Some(opponent_name);
                self.network_debug = network_debug;
                self.starting_life = starting_life;
                self.initial_state_hash = initial_state_hash;
                self.library_size = library_size;
                self.opponent_library_size = opponent_library_size;
                self.opponent_hand_count = opponent_hand_count;
                // CRITICAL: Store late-binding architecture data (mtg-d0jg3)
                self.deck_card_ids = deck_card_ids;
                self.rng_state = rng_state;
                self.token_definitions = token_definitions;
                self.state = NetworkState::InGame;

                // NOTE: Do NOT queue opening_hand reveals here!
                // The server sends individual CardRevealed messages for opening hand cards
                // immediately after GameStarted. If we queue opening_hand here AND process
                // the CardRevealed messages, we'd double-process the same cards.
                //
                // The native client also does NOT queue opening_hand as reveals - it just
                // registers the card definitions for later reference.
                //
                // The opening_hand field is informational only (hand count, which cards we got).
                let _ = opening_hand; // Acknowledge we intentionally ignore
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
                abilities,
                ..
            } => {
                log::debug!(
                    "WasmNetworkClient: ChoiceRequest seq={} type={:?} action_count={} abilities={}",
                    choice_seq,
                    choice_type,
                    action_count,
                    abilities.as_ref().map(|a| a.len()).unwrap_or(0)
                );
                self.current_choice_request = Some(ChoiceRequestData {
                    choice_seq,
                    choice_type,
                    options,
                    state_hash,
                    action_count,
                    abilities,
                });
            }

            ServerMessage::OpponentChoice {
                choice_seq,
                choice_indices,
                description,
                spell_ability,
                action_count,
                library_search_result,
                target_card_ids,
                ..
            } => {
                log::debug!(
                    "WasmNetworkClient: OpponentChoice seq={} indices={:?} action_count={} desc={}",
                    choice_seq,
                    choice_indices,
                    action_count,
                    description
                );
                self.opponent_choices.push_back(OpponentChoiceData {
                    choice_seq,
                    choice_indices,
                    description,
                    spell_ability,
                    action_count,
                    library_search_result,
                    target_card_ids,
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

            ServerMessage::LibraryReordered { player, new_order } => {
                // Library shuffle notification - for now just log it
                // The client doesn't track library order since cards are revealed individually
                log::debug!(
                    "WasmNetworkClient: Library reordered for {:?} ({} cards)",
                    player,
                    new_order.len()
                );
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

    /// Peek at the next OpponentChoice without consuming it.
    ///
    /// Used by SMART damage assignment overrides in `WasmRemoteController` so
    /// the override can extract `target_card_ids` BEFORE calling `try_get_choice`,
    /// which pops the entry. The peek-then-pop pattern is needed because the
    /// trait method receives no protocol context, only blocker lists.
    pub fn peek_opponent_choice(&self) -> Option<&OpponentChoiceData> {
        self.opponent_choices.front()
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
    pub fn submit_choice(&mut self, choice_indices: Vec<usize>, action_count: u64, state_hash: Option<u64>) {
        self.submit_choice_with_targets(choice_indices, action_count, state_hash, None)
    }

    /// Queue a SubmitChoice response, including authoritative `target_card_ids`.
    ///
    /// Used for choices like SMART damage assignment (`choose_blocker_for_lethal_damage`,
    /// `choose_blocker_for_remaining_damage`) where the server needs the actual chosen
    /// CardId — not just an index — because the client and server may have different
    /// blocker lists in network mode (mtg-e05f9c).
    pub fn submit_choice_with_targets(
        &mut self,
        choice_indices: Vec<usize>,
        action_count: u64,
        state_hash: Option<u64>,
        target_card_ids: Option<Vec<crate::core::CardId>>,
    ) {
        if let Some(ref request) = self.current_choice_request {
            let choice_seq = request.choice_seq;
            let msg = ClientMessage::SubmitChoice {
                choice_seq,
                choice_indices,
                action_count,
                timestamp_ms: crate::network::now_ms(),
                client_state_hash: state_hash,
                debug_info: None,
                spell_ability: None, // WASM client doesn't track spell_ability yet
                target_card_ids,
            };
            self.queue_outbound(msg);
            self.choice_acknowledged = false;
            self.last_submitted_choice_seq = Some(choice_seq);
            self.current_choice_request = None;
            log::debug!(
                "WasmNetworkClient: Submitted choice seq={}, waiting for ack",
                choice_seq
            );
        } else {
            log::warn!("WasmNetworkClient: submit_choice called without pending request");
        }
    }

    /// Get the last submitted choice sequence number
    pub fn last_submitted_choice_seq(&self) -> Option<u32> {
        self.last_submitted_choice_seq
    }

    /// Clear the last submitted choice sequence (called when ack is processed)
    pub fn clear_last_submitted_choice_seq(&mut self) {
        self.last_submitted_choice_seq = None;
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
        // Clear late-binding architecture data (mtg-d0jg3)
        self.deck_card_ids = None;
        self.rng_state.clear();
        self.token_definitions.clear();
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
