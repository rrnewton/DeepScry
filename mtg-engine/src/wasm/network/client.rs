//! WASM Network Client State Machine
//!
//! Manages connection state and message queues for browser-based network gameplay.
//! Unlike the native client which blocks on channels, this uses queues that
//! JavaScript can fill from WebSocket callbacks.

use crate::core::{PlayerId, SpellAbility};
use crate::network::{
    ActionLog, ChoiceEntry, ChoiceType, ClientMessage, DeckSubmission, ServerMessage, StateSyncEntry,
};
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

/// Data from an OpponentChoice message.
///
/// Carry-on view returned by [`WasmNetworkClient::pop_opponent_choice`] and
/// [`WasmNetworkClient::peek_opponent_choice`]. The underlying storage in
/// Phase 2 step 2 is now an `ActionLog<ChoiceEntry>` keyed by
/// `action_count`; this struct materialises the index + payload back into
/// the single shape `WasmRemoteController` and the SMART damage-assignment
/// overrides already consume. Keeping the shape stable across the refactor
/// localises the change to the storage layer.
#[derive(Debug, Clone)]
pub struct OpponentChoiceData {
    pub choice_seq: u32,
    pub choice_indices: Vec<usize>,
    pub description: String,
    pub spell_ability: Option<SpellAbility>,
    /// Server-reported `action_count` (= the `ActionLog<ChoiceEntry>` key
    /// in the new storage). Preserved for callers that still want to log
    /// or diff it.
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

impl OpponentChoiceData {
    /// Materialise the `ActionLog`-stored `(choice_seq, ChoiceEntry)`
    /// pair back into the legacy `OpponentChoiceData` shape that
    /// `WasmRemoteController` (and the SMART damage-assignment overrides)
    /// consume. Cheap clone of the structured payload — the same allocation
    /// the old `VecDeque` consumed when it `push_back`ed.
    ///
    /// The log key is `choice_seq` (mtg-sfihb), but `action_count` is still
    /// surfaced for diagnostics from the payload field.
    fn from_log_entry(entry: &ChoiceEntry) -> Self {
        Self {
            choice_seq: entry.choice_seq,
            choice_indices: entry.choice_indices.clone(),
            description: entry.description.clone(),
            spell_ability: entry.spell_ability.clone(),
            action_count: entry.action_count,
            library_search_result: entry.library_search_result,
            target_card_ids: entry.target_card_ids.clone(),
        }
    }
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

    /// Append-only, `action_count`-indexed shadow state-sync log.
    ///
    /// Backs the new Phase 2 reveal+reorder path described in
    /// `docs/NETWORK_ACTION_LOG.md` § 3.2. The WS receive handler pushes
    /// `ServerMessage::CardRevealed` / `LibraryReordered` here at a
    /// synthetic, strictly-monotonic `action_count` (wire-protocol option
    /// (b) from § 6 — server-authoritative `action_count` is a follow-up
    /// step). The sync callback non-destructively walks unapplied entries
    /// up to the frontier via [`apply_state_sync_up_to_frontier`].
    ///
    /// Replaces the legacy `pending_reveals` / `pending_library_reorders`
    /// VecDeques + `drain_*` helpers that the migration plan
    /// (`docs/NETWORK_ACTION_LOG_MIGRATION.md` § 1.1) deletes.
    pub state_sync: ActionLog<StateSyncEntry>,

    /// Synthetic `action_count` allocator for `state_sync` pushes.
    ///
    /// Since `ServerMessage::CardRevealed` / `LibraryReordered` do not yet
    /// carry an explicit `action_count` on the wire (mtg-engine/src/network/
    /// protocol.rs § 5 follow-up), the WS handler tags each receipt with
    /// `next_state_sync_ac` and bumps the counter. Strict monotonicity is
    /// thus trivially preserved and `ActionLog::push`'s invariant holds.
    next_state_sync_ac: u64,

    /// Cursor into `state_sync`: highest `action_count` whose entry has
    /// already been applied to the shadow `GameState`.
    ///
    /// `apply_state_sync_up_to_frontier` walks entries with
    /// `last_applied_state_sync_ac < ac <= frontier()` and bumps this
    /// cursor. The log itself is never popped or drained — invariant #4
    /// of `docs/NETWORK_ACTION_LOG.md` § 8.
    last_applied_state_sync_ac: u64,

    /// Append-only, `choice_seq`-indexed per-controller choice buffer for
    /// opponent (remote) choices.
    ///
    /// Backs the Phase 2 step 2 migration described in
    /// `docs/NETWORK_ACTION_LOG.md` § 3.1 / `NETWORK_ACTION_LOG_MIGRATION.md`
    /// § 2.1. The WS receive handler pushes `ServerMessage::OpponentChoice`
    /// here keyed by the server-reported **`choice_seq`** (NOT `action_count`).
    ///
    /// `choice_seq` is the only strictly-unique, strictly-monotonic per-choice
    /// key: the server bumps it exactly once per `ChoiceRequest`. `action_count`
    /// (= `undo_log.len()`) is NOT unique per choice — during multi-step combat
    /// damage assignment the server emits two choices
    /// (`choose_blocker_for_lethal_damage` then
    /// `choose_blocker_for_remaining_damage` for the same attacker) with no
    /// undoable action between them, so both carry the same `action_count`.
    /// Keying by `action_count` made the second `ActionLog::push` panic with
    /// "action_count must be strictly increasing" (mtg-sfihb). Keying by
    /// `choice_seq` is correct by construction.
    ///
    /// Reads are non-destructive: a per-client cursor
    /// (`next_opponent_choice_cursor`) records how far the controller has
    /// consumed, and a snapshot rewind resets the cursor without touching
    /// the log itself (invariant #3 of `docs/NETWORK_ACTION_LOG.md` § 8).
    ///
    /// Replaces the legacy `VecDeque<OpponentChoiceData>` queue and its
    /// destructive `pop_front` semantics. The structured payload is
    /// `ChoiceEntry`. See the "STATE-SYNC LOG" parallel for the same shape
    /// applied to reveals / reorders in step 1.
    pub opponent_choices: ActionLog<ChoiceEntry>,

    /// Read cursor into `opponent_choices`. The next `pop_opponent_choice`
    /// returns the first entry whose `choice_seq > next_opponent_choice_cursor`;
    /// the cursor then advances to that entry's `choice_seq`.
    ///
    /// This preserves the existing FIFO consume semantics that
    /// `WasmRemoteController` relies on while the underlying storage moves
    /// to append-only / non-destructive. A future step (the per-controller
    /// `choose_at(view, ac)` trait method described in
    /// `docs/NETWORK_ACTION_LOG.md` § 3.1) lets the engine ask for the
    /// entry at a specific `action_count`, bypassing this cursor entirely.
    next_opponent_choice_cursor: u64,

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
    /// CardID ranges for late-binding architecture (mtg-254)
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
    /// Lobby action to dispatch on WebSocket open (mtg-474).
    ///
    /// `None` (default) → preserve legacy behaviour: auto-send `Authenticate`
    /// against the well-known `DEFAULT_LOBBY_GAME` slot. The first authenticator
    /// becomes its creator, the second joins.
    ///
    /// `Some(LobbyAction::Create { .. })` → on open, send `CreateGame` for the
    /// named slot. Used by the landing-page lobby (web/index.html) when the
    /// user clicks "Create" and the page redirects to tui_game.html with
    /// `?lobby_create=...` query params.
    ///
    /// `Some(LobbyAction::Join { .. })` → on open, send `JoinGame` for the
    /// named slot. Used by the redirect from the lobby's "Join" button.
    lobby_action: Option<LobbyAction>,
}

/// Per-connection lobby action queued for the WebSocket-open callback.
///
/// Encapsulates the choice between `CreateGame` and `JoinGame` so the WASM
/// client can be driven from a `?lobby_create=` / `?lobby_join=` redirect
/// without growing a new wasm-bindgen flag per field.
#[derive(Debug, Clone)]
pub enum LobbyAction {
    /// Send `ClientMessage::CreateGame` on WS open.
    Create {
        /// Game slot name (must be unique among waiting games).
        game_name: String,
        /// Optional per-game password.
        game_password: Option<String>,
    },
    /// Send `ClientMessage::JoinGame` on WS open.
    Join {
        /// Existing slot to join (case-insensitive on the server).
        game_name: String,
        /// Per-game password if the creator set one.
        game_password: Option<String>,
    },
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
            state_sync: ActionLog::new(),
            next_state_sync_ac: 0,
            last_applied_state_sync_ac: 0,
            opponent_choices: ActionLog::new(),
            next_opponent_choice_cursor: 0,
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
            lobby_action: None,
        }
    }

    /// Configure the lobby action to dispatch on the next WebSocket `on_open`.
    ///
    /// Call this BEFORE the WebSocket opens. `None` reverts to the legacy
    /// `Authenticate` behaviour. See [`LobbyAction`] for variants.
    pub fn set_lobby_action(&mut self, action: Option<LobbyAction>) {
        self.lobby_action = action;
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

    /// Get CardID ranges for late-binding architecture (mtg-254)
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

        // Auto-dispatch the appropriate first message if we have stored params.
        // `lobby_action` (mtg-474) selects between legacy Authenticate vs the
        // explicit CreateGame / JoinGame paths used by the post-lobby redirect.
        let (Some(password), Some(player_name), Some(deck_json)) =
            (self.password.clone(), self.player_name.clone(), self.deck_json.clone())
        else {
            return;
        };
        let deck = match serde_json::from_str::<DeckSubmission>(&deck_json) {
            Ok(d) => d,
            Err(e) => {
                log::error!("WasmNetworkClient: Failed to parse deck JSON: {}", e);
                self.last_error = Some(format!("Invalid deck JSON: {}", e));
                self.state = NetworkState::Error;
                return;
            }
        };
        match self.lobby_action.clone() {
            None => {
                self.authenticate(&password, &player_name, deck);
                log::info!("WasmNetworkClient: Auto-authenticated as {}", player_name);
            }
            Some(LobbyAction::Create {
                game_name,
                game_password,
            }) => {
                self.create_game(&password, &game_name, game_password, &player_name, deck);
                log::info!("WasmNetworkClient: Sent CreateGame '{}' as {}", game_name, player_name);
            }
            Some(LobbyAction::Join {
                game_name,
                game_password,
            }) => {
                self.join_game(&password, &game_name, game_password, &player_name, deck);
                log::info!("WasmNetworkClient: Sent JoinGame '{}' as {}", game_name, player_name);
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

    /// Queue a `CreateGame` message — register a named pre-game slot on the
    /// server and wait for an opponent to `JoinGame` it.
    ///
    /// Used by the post-lobby redirect path (mtg-474): the landing-page
    /// lobby (web/index.html) closes its own browsing WS and redirects to
    /// `tui_game.html?lobby_create=NAME&...`. The TUI page then opens a fresh
    /// WS, calls `network_create_game`, and runs the per-game flow from there.
    /// This avoids re-using a lobby-mode WS for in-game traffic, which is
    /// what triggered the "Already authenticated / in a game" Error reply
    /// from `handle_player_websocket` in the previous design.
    pub fn create_game(
        &mut self,
        password: &str,
        game_name: &str,
        game_password: Option<String>,
        player_name: &str,
        deck: DeckSubmission,
    ) {
        let msg = ClientMessage::CreateGame {
            password: password.to_string(),
            game_name: if game_name.is_empty() {
                None
            } else {
                Some(game_name.to_string())
            },
            game_password,
            player_name: if player_name.is_empty() {
                None
            } else {
                Some(player_name.to_string())
            },
            deck,
            // The game page plays the game with legacy immediate-start; the
            // launcher's lobby socket already ran the Variant-1 waiting-room
            // rendezvous (mtg-682) and freed the slot before navigating here.
            waiting_room: false,
        };
        self.queue_outbound(msg);
    }

    /// Queue a `JoinGame` message — attach to an existing pre-game slot the
    /// lobby already advertised via `GameList`.
    pub fn join_game(
        &mut self,
        password: &str,
        game_name: &str,
        game_password: Option<String>,
        player_name: &str,
        deck: DeckSubmission,
    ) {
        let msg = ClientMessage::JoinGame {
            password: password.to_string(),
            game_name: game_name.to_string(),
            game_password,
            player_name: if player_name.is_empty() {
                None
            } else {
                Some(player_name.to_string())
            },
            deck,
            // Game-page join uses legacy immediate-start; the launcher already
            // did the Variant-1 waiting-room rendezvous (mtg-682).
            waiting_room: false,
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
                reconnect_token: _, // Stored by the JS layer, not the WASM game client
            } => {
                log::info!(
                    "WasmNetworkClient: Game started! We are {:?}, opponent: {}, life: {}",
                    your_player_id,
                    opponent_name,
                    starting_life
                );

                // Log critical sync data (mtg-254: behavioral identity)
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
                // CRITICAL: Store late-binding architecture data (mtg-254)
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
                self.push_state_sync(StateSyncEntry::RevealCard {
                    owner,
                    card: Box::new(card),
                    reason,
                });
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
                // Phase 2 step 2: append to the per-controller choice
                // buffer keyed by the server-reported `choice_seq`.
                //
                // `choice_seq` (NOT `action_count`) is the log key: the server
                // bumps it exactly once per ChoiceRequest, so it is strictly
                // unique and monotonic per choice and directly satisfies
                // `ActionLog::push`'s strict-monotonicity invariant
                // (docs/NETWORK_ACTION_LOG.md § 8 invariant #2). `action_count`
                // (= undo_log.len()) is NOT unique: multi-step combat damage
                // assignment emits two choices at the same action_count
                // (mtg-sfihb), which made keying by action_count panic on the
                // second push. If the server ever sends a stale or out-of-order
                // choice_seq, the push will panic — the correct response per
                // NETWORK_ARCHITECTURE.md § "Desync is ALWAYS a Fatal Error".
                self.opponent_choices.push(
                    u64::from(choice_seq),
                    ChoiceEntry {
                        choice_seq,
                        action_count,
                        choice_indices,
                        description,
                        spell_ability,
                        library_search_result,
                        target_card_ids,
                    },
                );
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
                // mtg-589 / Phase 2 step 1: Queue the authoritative library
                // order in the shadow state-sync log so the shadow GameState
                // can adopt it at the next sync point (BEFORE reveals/draws).
                // Without this, the shadow's library order drifts from the
                // server after any shuffle -> FATAL P2 state-hash mismatch.
                log::debug!(
                    "WasmNetworkClient: Library reordered for {:?} ({} cards) - logged",
                    player,
                    new_order.len()
                );
                self.push_state_sync(StateSyncEntry::LibraryReorder { player, new_order });
            }

            #[cfg(debug_assertions)]
            ServerMessage::DebugStateDump { .. } => {
                // Debug info, just log
                log::debug!("WasmNetworkClient: Received debug state dump");
            }

            // ─── Lobby messages ────────────────────────────────────────────
            // The WASM client doesn't yet drive the multi-game lobby UI
            // (it joins a single game at a time), so for now we just log
            // these so the match stays exhaustive and so a developer can see
            // them in the browser console while the lobby UI is wired up.
            // TODO(server-lobby): plumb GameList / GameCreated through to
            // the browser UI once the lobby front-end lands.
            ServerMessage::GameList {
                games,
                total_count,
                system_memory_used_percent,
                max_memory_percent,
            } => {
                log::info!(
                    "WasmNetworkClient: GameList ({}/{} waiting games, host_mem={:?}%, ceiling={}%)",
                    games.len(),
                    total_count,
                    system_memory_used_percent,
                    max_memory_percent
                );
            }

            ServerMessage::GameCreated {
                game_name,
                your_player_id,
                your_name,
            } => {
                log::info!(
                    "WasmNetworkClient: GameCreated name={:?} player={:?} display_name={:?}",
                    game_name,
                    your_player_id,
                    your_name
                );
                self.our_player_id = Some(your_player_id);
                self.state = NetworkState::WaitingForOpponent;
            }

            ServerMessage::ServerFull { reason } => {
                // Server-side admission denial. Treat as fatal for this
                // socket — the server will close it. The wire `reason` is
                // intentionally generic (no host metrics); see
                // `protocol::ServerMessage::ServerFull` docs.
                log::warn!("WasmNetworkClient: ServerFull: {}", reason);
                self.state = NetworkState::Error;
                self.last_error = Some(reason);
            }

            ServerMessage::JoinFailed { game_name, reason } => {
                log::warn!("WasmNetworkClient: JoinFailed for {:?}: {:?}", game_name, reason);
                self.last_error = Some(format!("Join failed for {game_name}: {reason:?}"));
            }

            // New lobby protocol messages — handled at the JS layer or
            // safely ignored in the WASM game client.
            ServerMessage::RegisterResult {
                success,
                player_name,
                error,
            } => {
                log::info!(
                    "WasmNetworkClient: RegisterResult name={:?} success={} error={:?}",
                    player_name,
                    success,
                    error
                );
            }

            ServerMessage::WaitingRoomUpdate { .. } => {
                // Forwarded to JS via callback in the future; currently a no-op
                // in the WASM game client (the lobby page handles it).
                log::debug!("WasmNetworkClient: WaitingRoomUpdate (lobby layer)");
            }

            ServerMessage::WaitingRoomReady { .. } => {
                // Pre-game rendezvous "go" signal. The launcher's plain-JS lobby
                // socket consumes this, not the WASM game client — by the time
                // the game page (which owns this WASM client) connects, the
                // waiting-room handshake is already done. No-op here.
                log::debug!("WasmNetworkClient: WaitingRoomReady (launcher lobby layer)");
            }

            ServerMessage::ReconnectResult { success, game_name, .. } => {
                log::info!(
                    "WasmNetworkClient: ReconnectResult success={} game={:?}",
                    success,
                    game_name
                );
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

    /// Check if at least one unconsumed `OpponentChoice` is buffered.
    ///
    /// Phase 2 step 2: this is the cursor-vs-frontier test on
    /// `opponent_choices` — equivalent to the legacy `!is_empty()` check
    /// against the old `VecDeque`, but it derives the answer from the
    /// non-destructive `ActionLog` + cursor pair. See the field-level
    /// docs on `opponent_choices` / `next_opponent_choice_cursor`.
    pub fn has_opponent_choice(&self) -> bool {
        self.opponent_choices
            .frontier()
            .is_some_and(|f| f > self.next_opponent_choice_cursor)
    }

    /// Consume the next buffered `OpponentChoice` (the smallest
    /// `choice_seq` strictly greater than the read cursor).
    ///
    /// **Non-destructive read.** The underlying `ActionLog` is
    /// untouched; only the per-controller cursor advances. A subsequent
    /// `reset_opponent_choice_cursor()` re-exposes the entries to a
    /// replay pass without re-fetching from the server (invariant #3 of
    /// `docs/NETWORK_ACTION_LOG.md` § 8).
    pub fn pop_opponent_choice(&mut self) -> Option<OpponentChoiceData> {
        let (key, entry) = self.next_opponent_choice_unconsumed()?;
        // Clone before bumping the cursor — entry borrows `&self`, so we
        // materialise it BEFORE the `&mut self` cursor mutation below.
        let materialised = OpponentChoiceData::from_log_entry(entry);
        self.next_opponent_choice_cursor = key;
        Some(materialised)
    }

    /// Peek at the next buffered `OpponentChoice` without consuming it.
    ///
    /// Used by SMART damage assignment overrides in `WasmRemoteController`
    /// so the override can extract `target_card_ids` BEFORE calling the
    /// pop path. The peek-then-pop pattern is needed because the trait
    /// method receives no protocol context, only blocker lists.
    ///
    /// Returns the entry as an owned `OpponentChoiceData` (cheap clone of
    /// the structured fields). The previous API returned `Option<&_>`; the
    /// new API returns owned `Option<_>` because the `ActionLog`-backed
    /// storage materialises the carry-on `action_count` alongside the
    /// payload (the wire `OpponentChoiceData` shape persists across the
    /// refactor).
    pub fn peek_opponent_choice(&self) -> Option<OpponentChoiceData> {
        let (_key, entry) = self.next_opponent_choice_unconsumed()?;
        Some(OpponentChoiceData::from_log_entry(entry))
    }

    /// Internal helper: find the first `(choice_seq, &ChoiceEntry)` in
    /// `opponent_choices` with `choice_seq > next_opponent_choice_cursor`.
    /// Returns `None` if no unconsumed entry is available.
    ///
    /// Linear scan over the unconsumed suffix. The `ActionLog::iter` impl
    /// yields in ascending key (`choice_seq`) order, so the first match is the
    /// correct next entry. Several entries can be buffered at once: during
    /// multi-step combat damage assignment the server emits several
    /// OpponentChoices in a row (each its own `choice_seq`), so the cursor
    /// walks them one per consume.
    fn next_opponent_choice_unconsumed(&self) -> Option<(u64, &ChoiceEntry)> {
        self.opponent_choices
            .iter()
            .find(|(ac, _)| *ac > self.next_opponent_choice_cursor)
    }

    /// Reset the opponent-choice read cursor so the next `pop_opponent_choice`
    /// re-reads every buffered entry from scratch.
    ///
    /// Used by snapshot-resume / rewind code paths: when the engine
    /// rewinds `game.action_count`, the controller's choice consumption
    /// also rewinds, so the log's entries must be re-readable during the
    /// forward replay pass. The log itself stays intact — non-destructive
    /// reads are precisely what makes "rewind + replay" free
    /// (invariant #3 of `docs/NETWORK_ACTION_LOG.md` § 8).
    pub fn reset_opponent_choice_cursor(&mut self) {
        self.next_opponent_choice_cursor = 0;
    }

    /// Current opponent-choice read cursor (highest consumed `choice_seq`).
    ///
    /// Paired with [`set_opponent_choice_cursor`] to support the multi-step
    /// combat damage assignment checkpoint/restore (mtg-sfihb): the engine
    /// snapshots this value before the synchronous first pass and restores it
    /// if a mid-pass sub-choice has not yet arrived, so the re-run re-consumes
    /// the buffered sub-choices from the same starting point.
    pub fn opponent_choice_cursor(&self) -> u64 {
        self.next_opponent_choice_cursor
    }

    /// Restore the opponent-choice read cursor to a value previously obtained
    /// from [`opponent_choice_cursor`]. See that method (mtg-sfihb).
    pub fn set_opponent_choice_cursor(&mut self, cursor: u64) {
        self.next_opponent_choice_cursor = cursor;
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // STATE-SYNC LOG (Phase 2 step 1 — reveal/reorder via ActionLog<StateSyncEntry>)
    // ═══════════════════════════════════════════════════════════════════════════
    //
    // See `docs/NETWORK_ACTION_LOG.md` § 3.2 for design and
    // `docs/NETWORK_ACTION_LOG_MIGRATION.md` § 1 for the deletion list this
    // replaces. The legacy `drain_reveals` / `drain_library_reorders` /
    // `pop_reveal` / `has_pending_reveals` helpers and the
    // `pending_reveals` / `pending_library_reorders` VecDeques are gone.

    /// Append a `StateSyncEntry` to the shadow state-sync log at the next
    /// synthetic `action_count`.
    ///
    /// Sole appender: the WS receive handler (`on_message`). Per invariant
    /// #1 of the design doc (`ActionLog<T>` is append-only), nothing else
    /// in the codebase pushes here.
    fn push_state_sync(&mut self, entry: StateSyncEntry) {
        self.next_state_sync_ac += 1;
        self.state_sync.push(self.next_state_sync_ac, entry);
    }

    /// Apply every state-sync entry that has been received but not yet
    /// applied to `shadow`. **Non-destructive read** of `state_sync` —
    /// only the per-client cursor advances; the log itself is untouched.
    ///
    /// `local_player` is forwarded to `process_card_reveal` so reveals
    /// targeting our hand resolve correctly.
    ///
    /// Returns the number of entries applied (for diagnostics).
    pub fn apply_state_sync_up_to_frontier(
        &mut self,
        shadow: &mut crate::game::GameState,
        local_player: Option<PlayerId>,
    ) -> usize {
        let frontier = match self.state_sync.frontier() {
            Some(f) => f,
            None => return 0,
        };
        if frontier <= self.last_applied_state_sync_ac {
            return 0;
        }

        let mut applied = 0;
        // CRITICAL ORDERING (mtg-589): apply LibraryReorder entries BEFORE
        // RevealCard entries within each apply batch, even when the reveals
        // arrived earlier on the wire. The server guarantees the library
        // order BEFORE draws are processed (otherwise the shadow would
        // draw from a stale order and diverge on the first draw). The
        // legacy `drain_library_reorders → drain_reveals` two-step
        // preserved this; the new log preserves it via a two-pass apply
        // over the same cursor window. The cursor still moves
        // monotonically, so re-runs after a rewind replay the entries in
        // the same per-pass order and produce bit-identical shadow state.
        let to_apply: Vec<(u64, StateSyncEntry)> = self
            .state_sync
            .iter()
            .filter(|(ac, _)| *ac > self.last_applied_state_sync_ac && *ac <= frontier)
            .map(|(ac, entry)| (ac, entry.clone()))
            .collect();

        // Pass 1: library reorders. Protocol sends top-to-bottom; shadow
        // library Vec is bottom-to-top (draw pops the last element).
        for (ac, entry) in &to_apply {
            if let StateSyncEntry::LibraryReorder { player, new_order } = entry {
                log::debug!(
                    "apply_state_sync: library reorder ac={} player={:?} ({} cards)",
                    ac,
                    player,
                    new_order.len()
                );
                if let Some(zones) = shadow.get_player_zones_mut(*player) {
                    zones.library.cards = new_order.iter().rev().copied().collect();
                }
            }
        }

        // Pass 2: card reveals. Library order is now server-authoritative,
        // so reveal-side mutations that touch library order (e.g. moving a
        // revealed card from library to hand) see the correct positions.
        //
        // The cursor advances for EVERY entry in the window (reorder and
        // reveal alike) so that subsequent calls don't re-apply pass-1
        // entries. Pass-1 reorders that have no pass-2 counterpart still
        // need their cursor tick.
        for (ac, entry) in to_apply {
            if let StateSyncEntry::RevealCard { owner, card, reason } = entry {
                log::debug!(
                    "apply_state_sync: reveal ac={} owner={:?} card={}",
                    ac,
                    owner,
                    card.name
                );
                crate::wasm::network::game_init::process_card_reveal_wasm(shadow, owner, *card, reason, local_player);
            }
            self.last_applied_state_sync_ac = ac;
            applied += 1;
        }
        applied
    }

    /// Reset the state-sync cursor so the next
    /// `apply_state_sync_up_to_frontier` call re-applies every entry from
    /// scratch.
    ///
    /// Used by snapshot-resume / rewind code paths: when the engine rewinds
    /// `game.action_count`, the shadow state mutations also rewind, so the
    /// log's entries must be re-applied during the forward replay pass.
    /// The log itself stays intact — that's exactly what "non-destructive
    /// reads" buys us (invariant #3 of `docs/NETWORK_ACTION_LOG.md` § 8).
    pub fn reset_state_sync_cursor(&mut self) {
        self.last_applied_state_sync_ac = 0;
    }

    /// Reveal-only variant of [`apply_state_sync_up_to_frontier`].
    ///
    /// Applies `RevealCard` entries but **skips** `LibraryReorder` entries
    /// in the cursor window. The cursor still advances over BOTH so a
    /// re-call is idempotent; skipped reorders will NOT be re-applied.
    ///
    /// This preserves the legacy `run_network_mode_ai_v2` behaviour where
    /// only reveals were drained at each sync point. Reorders queued
    /// during the AI-v2 path were never drained, and adopting them now
    /// would diverge from the GameLoop's opening-hand `draw_card_silent`
    /// sequence (which expects the library in its as-initialised order).
    pub fn apply_state_sync_reveals_up_to_frontier(
        &mut self,
        shadow: &mut crate::game::GameState,
        local_player: Option<PlayerId>,
    ) -> usize {
        let frontier = match self.state_sync.frontier() {
            Some(f) => f,
            None => return 0,
        };
        if frontier <= self.last_applied_state_sync_ac {
            return 0;
        }

        let mut applied = 0;
        let to_apply: Vec<(u64, StateSyncEntry)> = self
            .state_sync
            .iter()
            .filter(|(ac, _)| *ac > self.last_applied_state_sync_ac && *ac <= frontier)
            .map(|(ac, entry)| (ac, entry.clone()))
            .collect();
        for (ac, entry) in to_apply {
            if let StateSyncEntry::RevealCard { owner, card, reason } = entry {
                log::debug!(
                    "apply_state_sync_reveals: reveal ac={} owner={:?} card={}",
                    ac,
                    owner,
                    card.name
                );
                crate::wasm::network::game_init::process_card_reveal_wasm(shadow, owner, *card, reason, local_player);
                applied += 1;
            }
            // Cursor advances for BOTH reveals and reorders. Reorders in
            // this window are intentionally skipped and will NOT be
            // re-applied by a later call to either apply method.
            self.last_applied_state_sync_ac = ac;
        }
        applied
    }

    /// True iff there is at least one unapplied state-sync entry. Cheap
    /// frontier comparison; preserves the "K > frontier ⇒ yield NeedsInput"
    /// semantics of invariant #5 in design-doc terms.
    pub fn has_unapplied_state_sync(&self) -> bool {
        self.state_sync
            .frontier()
            .is_some_and(|f| f > self.last_applied_state_sync_ac)
    }

    /// Diagnostic accessor: current state-sync apply cursor. Test-only use.
    #[cfg(test)]
    pub(crate) fn last_applied_state_sync_ac(&self) -> u64 {
        self.last_applied_state_sync_ac
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
    /// blocker lists in network mode (mtg-418).
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
        self.state_sync = ActionLog::new();
        self.next_state_sync_ac = 0;
        self.last_applied_state_sync_ac = 0;
        self.opponent_choices = ActionLog::new();
        self.next_opponent_choice_cursor = 0;
        self.current_choice_request = None;
        self.choice_acknowledged = true;
        self.outbound_queue.clear();
        self.last_error = None;
        self.winner = None;
        // Clear late-binding architecture data (mtg-254)
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

// ═══════════════════════════════════════════════════════════════════════════
// TESTS — state-sync log invariants (Phase 2 step 1)
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::CardId;
    use crate::network::{CardReveal, RevealReason};

    fn mk_reveal(name: &str) -> StateSyncEntry {
        StateSyncEntry::RevealCard {
            owner: PlayerId::new(0),
            card: Box::new(CardReveal {
                card_id: CardId::new(1),
                name: name.into(),
                card_def: None,
            }),
            reason: RevealReason::Draw,
        }
    }

    fn mk_reorder(player_id: u32) -> StateSyncEntry {
        StateSyncEntry::LibraryReorder {
            player: PlayerId::new(player_id),
            new_order: vec![CardId::new(7), CardId::new(8)],
        }
    }

    #[test]
    fn push_state_sync_assigns_strictly_increasing_acs() {
        // Invariant #2 of docs/NETWORK_ACTION_LOG.md § 8: strictly monotonic
        // action_count. The synthetic allocator must never violate this.
        let mut c = WasmNetworkClient::new();
        c.push_state_sync(mk_reveal("a"));
        c.push_state_sync(mk_reorder(0));
        c.push_state_sync(mk_reveal("b"));
        let acs: Vec<u64> = c.state_sync.iter().map(|(ac, _)| ac).collect();
        assert_eq!(acs, vec![1, 2, 3]);
        assert_eq!(c.state_sync.frontier(), Some(3));
    }

    #[test]
    fn cursor_starts_at_zero_and_advances_with_apply() {
        // The cursor MUST track applied entries so a second apply call
        // re-applies nothing. This is the property that replaces the
        // legacy drain_*() destructive semantics with a non-destructive
        // read (invariant #3, #4).
        //
        // We exercise the cursor mechanic via the helper that is the only
        // mutation point besides apply_state_sync_up_to_frontier. A real
        // GameState round-trip is exercised by the e2e regression test
        // (tests/robots42_state_sync_e2e.sh).
        let mut c = WasmNetworkClient::new();
        assert_eq!(c.last_applied_state_sync_ac(), 0);
        assert!(!c.has_unapplied_state_sync());

        c.push_state_sync(mk_reveal("a"));
        c.push_state_sync(mk_reveal("b"));
        assert!(c.has_unapplied_state_sync());
        assert_eq!(c.last_applied_state_sync_ac(), 0);
    }

    #[test]
    fn reset_state_sync_cursor_rewinds_for_replay() {
        // Snapshot/rewind support: after the engine rewinds, the next
        // forward pass must replay every entry. The log stays intact —
        // only the cursor resets. This is the property docs/NETWORK_ACTION_LOG.md
        // § 8 calls out as making rewind/replay free.
        let mut c = WasmNetworkClient::new();
        c.push_state_sync(mk_reveal("x"));
        c.push_state_sync(mk_reorder(1));
        c.last_applied_state_sync_ac = 2; // simulate post-apply
        assert!(!c.has_unapplied_state_sync());

        c.reset_state_sync_cursor();
        assert_eq!(c.last_applied_state_sync_ac(), 0);
        assert!(c.has_unapplied_state_sync());
        // Log is intact:
        assert_eq!(c.state_sync.len(), 2);
        assert_eq!(c.state_sync.frontier(), Some(2));
    }

    #[test]
    fn reset_clears_state_sync_log_and_cursor() {
        // A new game starts fresh: log empty, cursor at 0, allocator at 0.
        let mut c = WasmNetworkClient::new();
        c.push_state_sync(mk_reveal("x"));
        c.push_state_sync(mk_reorder(0));
        c.last_applied_state_sync_ac = 1;
        c.reset();
        assert_eq!(c.state_sync.len(), 0);
        assert_eq!(c.state_sync.frontier(), None);
        assert_eq!(c.last_applied_state_sync_ac(), 0);
        assert_eq!(c.next_state_sync_ac, 0);
        assert!(!c.has_unapplied_state_sync());
    }

    #[test]
    fn has_unapplied_state_sync_reflects_cursor_position() {
        // Verifies the frontier vs cursor comparison that gates the
        // "K > frontier ⇒ yield NeedsInput" signal at the wasm shim layer.
        let mut c = WasmNetworkClient::new();
        assert!(!c.has_unapplied_state_sync());
        c.push_state_sync(mk_reveal("a"));
        assert!(c.has_unapplied_state_sync());
        c.last_applied_state_sync_ac = 1;
        assert!(!c.has_unapplied_state_sync());
        c.push_state_sync(mk_reveal("b"));
        assert!(c.has_unapplied_state_sync());
    }

    // ─── Phase 2 step 2 — per-controller choice buffer ─────────────────

    /// Drive the on_message OpponentChoice path. Bypasses JSON parsing by
    /// directly calling the private handler; verifies the message lands in
    /// `opponent_choices` keyed by the wire `choice_seq` (mtg-sfihb), while
    /// the wire `action_count` is carried on the payload.
    fn push_opponent_choice(client: &mut WasmNetworkClient, ac: u64, choice_seq: u32, desc: &str) {
        client.handle_server_message(crate::network::ServerMessage::OpponentChoice {
            choice_seq,
            player: PlayerId::new(1),
            choice_type: crate::network::ChoiceType::Priority,
            choice_indices: vec![choice_seq as usize],
            description: desc.into(),
            action_count: ac,
            timestamp_ms: 0,
            spell_ability: None,
            library_search_result: None,
            target_card_ids: None,
            state_hash_after: None,
            debug_info: None,
        });
    }

    #[test]
    fn opponent_choice_appended_to_action_log_by_wire_choice_seq() {
        // Wire-protocol `choice_seq` is the ActionLog key (NOT action_count);
        // pushes in strictly-increasing choice_seq order satisfy
        // ActionLog::push's monotonicity invariant. This replaces the legacy
        // VecDeque::push_back.
        let mut c = WasmNetworkClient::new();
        push_opponent_choice(&mut c, 5, 1, "first");
        push_opponent_choice(&mut c, 12, 2, "second");
        push_opponent_choice(&mut c, 100, 3, "third");

        assert_eq!(c.opponent_choices.len(), 3);
        // Frontier is the highest choice_seq (3), not the highest action_count.
        assert_eq!(c.opponent_choices.frontier(), Some(3));
        // Non-destructive: same look-up (by choice_seq) returns the same
        // payload twice; the carried action_count is preserved.
        let a = c.opponent_choices.get(2).unwrap();
        assert_eq!(a.description, "second");
        assert_eq!(a.action_count, 12);
        let b = c.opponent_choices.get(2).unwrap();
        assert_eq!(b.description, "second");
    }

    #[test]
    fn opponent_choices_sharing_one_action_count_do_not_panic() {
        // mtg-sfihb regression: during multi-step combat damage assignment
        // the server emits two OpponentChoices
        // (choose_blocker_for_lethal_damage then
        // choose_blocker_for_remaining_damage for the same attacker) with NO
        // undoable action between them, so BOTH carry the same action_count.
        // Keying the log by action_count made the second push panic with
        // "action_count must be strictly increasing". Keying by choice_seq
        // (strictly unique per choice) is correct. This is the exact shape
        // observed at action_count=978, choice_seq 181 then 182.
        let mut c = WasmNetworkClient::new();
        push_opponent_choice(&mut c, 978, 181, "lethal damage assignment");
        push_opponent_choice(&mut c, 978, 182, "remaining damage assignment");

        assert_eq!(c.opponent_choices.len(), 2);
        assert_eq!(c.opponent_choices.frontier(), Some(182));

        // Both are consumable in choice_seq order, each carrying ac=978.
        let first = c.pop_opponent_choice().unwrap();
        assert_eq!(first.choice_seq, 181);
        assert_eq!(first.action_count, 978);
        let second = c.pop_opponent_choice().unwrap();
        assert_eq!(second.choice_seq, 182);
        assert_eq!(second.action_count, 978);
        assert!(c.pop_opponent_choice().is_none());
    }

    #[test]
    fn cursor_checkpoint_restore_re_consumes_combat_damage_subchoices() {
        // mtg-sfihb layer 2: multi-step combat damage assignment runs in a
        // SYNCHRONOUS first pass that cannot yield mid-loop. On a shadow
        // client, when the engine has consumed the FIRST sub-choice but the
        // SECOND has not yet arrived over the wire, the controller signals
        // NeedInput; `assign_combat_damage` then restores the choice cursor to
        // the pre-pass checkpoint so the re-entry re-consumes BOTH sub-choices
        // from the start. This test exercises the cursor get/set that
        // `WasmRemoteController::{mark,restore}_choice_checkpoint` use.
        let mut c = WasmNetworkClient::new();
        // The opponent's lethal-damage sub-choice arrives first; the
        // remaining-damage sub-choice is still in flight.
        push_opponent_choice(&mut c, 978, 181, "lethal damage assignment");

        // Engine begins the synchronous first pass: checkpoint, then consume
        // the first sub-choice.
        let checkpoint = c.opponent_choice_cursor();
        let first = c.pop_opponent_choice().unwrap();
        assert_eq!(first.choice_seq, 181);
        // Second sub-choice not yet buffered -> the controller would NeedInput.
        assert!(c.pop_opponent_choice().is_none());

        // Restore the cursor (what restore_choice_checkpoint does) and re-raise
        // NeedInput. The first sub-choice is exposed again for the re-run.
        c.set_opponent_choice_cursor(checkpoint);
        assert!(c.has_opponent_choice());

        // The remaining-damage sub-choice now arrives.
        push_opponent_choice(&mut c, 978, 182, "remaining damage assignment");

        // Re-entry: the first pass re-runs and re-consumes BOTH sub-choices in
        // order from the restored checkpoint — no double-consume, no skip.
        let re_first = c.pop_opponent_choice().unwrap();
        assert_eq!(re_first.choice_seq, 181);
        let re_second = c.pop_opponent_choice().unwrap();
        assert_eq!(re_second.choice_seq, 182);
        assert!(c.pop_opponent_choice().is_none());
    }

    #[test]
    fn pop_opponent_choice_advances_cursor_without_dropping_entries() {
        // Replaces the legacy `VecDeque::pop_front`. The log is untouched;
        // only the per-client cursor advances. After three pops, the log
        // still has all three entries — a `reset_opponent_choice_cursor`
        // would re-expose them (the rewind / replay property).
        let mut c = WasmNetworkClient::new();
        push_opponent_choice(&mut c, 3, 1, "a");
        push_opponent_choice(&mut c, 7, 2, "b");
        push_opponent_choice(&mut c, 9, 3, "c");

        assert!(c.has_opponent_choice());
        let first = c.pop_opponent_choice().unwrap();
        assert_eq!(first.choice_seq, 1);
        assert_eq!(first.action_count, 3);
        let second = c.pop_opponent_choice().unwrap();
        assert_eq!(second.choice_seq, 2);
        assert_eq!(second.action_count, 7);
        let third = c.pop_opponent_choice().unwrap();
        assert_eq!(third.choice_seq, 3);
        assert_eq!(third.action_count, 9);
        assert!(!c.has_opponent_choice());
        assert!(c.pop_opponent_choice().is_none());

        // Log itself is intact — non-destructive read invariant.
        assert_eq!(c.opponent_choices.len(), 3);
    }

    #[test]
    fn peek_opponent_choice_does_not_advance_cursor() {
        // Multi-call peek must yield the same entry; only pop advances
        // the cursor. The SMART damage assignment overrides in
        // WasmRemoteController rely on this: peek to grab target_card_ids,
        // then pop the same entry.
        let mut c = WasmNetworkClient::new();
        push_opponent_choice(&mut c, 4, 11, "block");

        let peek_a = c.peek_opponent_choice().unwrap();
        let peek_b = c.peek_opponent_choice().unwrap();
        assert_eq!(peek_a.choice_seq, 11);
        assert_eq!(peek_b.choice_seq, 11);
        assert_eq!(peek_a.action_count, 4);

        let popped = c.pop_opponent_choice().unwrap();
        assert_eq!(popped.choice_seq, 11);
        assert!(c.peek_opponent_choice().is_none());
    }

    #[test]
    fn reset_opponent_choice_cursor_re_exposes_consumed_entries() {
        // Snapshot rewind / replay: the engine rolls back action_count via
        // its undo log; the controller's choice consumption must also roll
        // back. The log is the persistent store, the cursor is what moves.
        let mut c = WasmNetworkClient::new();
        push_opponent_choice(&mut c, 2, 1, "first");
        push_opponent_choice(&mut c, 8, 2, "second");
        let _ = c.pop_opponent_choice();
        let _ = c.pop_opponent_choice();
        assert!(!c.has_opponent_choice());
        assert_eq!(c.opponent_choices.len(), 2);

        c.reset_opponent_choice_cursor();

        assert!(c.has_opponent_choice());
        let replay_first = c.pop_opponent_choice().unwrap();
        assert_eq!(replay_first.choice_seq, 1);
        let replay_second = c.pop_opponent_choice().unwrap();
        assert_eq!(replay_second.choice_seq, 2);
    }

    #[test]
    fn reset_clears_opponent_choice_log_and_cursor() {
        // A new game wipes the buffer and resets the cursor (parallel to
        // the state_sync reset case in step 1).
        let mut c = WasmNetworkClient::new();
        push_opponent_choice(&mut c, 5, 1, "a");
        let _ = c.pop_opponent_choice();
        c.reset();
        assert_eq!(c.opponent_choices.len(), 0);
        assert_eq!(c.opponent_choices.frontier(), None);
        assert!(!c.has_opponent_choice());
    }

    #[test]
    #[should_panic(expected = "strictly increasing")]
    fn duplicate_opponent_choice_seq_panics() {
        // Wire-protocol guarantee: choice_seq is strictly monotonic per
        // session (the server bumps it once per ChoiceRequest). A duplicate
        // choice_seq is a protocol bug; per NETWORK_ARCHITECTURE.md "Desync
        // is ALWAYS a Fatal Error" we crash rather than silently accept it.
        // ActionLog::push provides the panic; this test verifies the wiring
        // forwards it on the NEW (choice_seq) key.
        let mut c = WasmNetworkClient::new();
        push_opponent_choice(&mut c, 5, 1, "first");
        push_opponent_choice(&mut c, 9, 1, "duplicate choice_seq");
    }
}
