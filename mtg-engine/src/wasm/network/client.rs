//! WASM Network Client State Machine
//!
//! Manages connection state and message queues for browser-based network gameplay.
//! Unlike the native client which blocks on channels, this uses queues that
//! JavaScript can fill from WebSocket callbacks.

use crate::core::{PlayerId, SpellAbility};
use crate::network::{ActionLog, ChoiceType, ClientMessage, DeckSubmission, ServerMessage, StateSyncEntry};
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
        // Iterate in append (= action_count-ascending) order; this is the
        // ordering that matches the wire arrival sequence. Library reorders
        // and reveals are interleaved correctly because each receipt got its
        // own synthetic ac in receive order.
        let to_apply: Vec<(u64, StateSyncEntry)> = self
            .state_sync
            .iter()
            .filter(|(ac, _)| *ac > self.last_applied_state_sync_ac && *ac <= frontier)
            .map(|(ac, entry)| (ac, entry.clone()))
            .collect();
        for (ac, entry) in to_apply {
            match entry {
                StateSyncEntry::LibraryReorder { player, new_order } => {
                    // Protocol sends top-to-bottom; shadow library Vec is
                    // bottom-to-top (draw pops the last element).
                    log::debug!(
                        "apply_state_sync: library reorder ac={} player={:?} ({} cards)",
                        ac,
                        player,
                        new_order.len()
                    );
                    if let Some(zones) = shadow.get_player_zones_mut(player) {
                        zones.library.cards = new_order.into_iter().rev().collect();
                    }
                }
                StateSyncEntry::RevealCard { owner, card, reason } => {
                    log::debug!(
                        "apply_state_sync: reveal ac={} owner={:?} card={}",
                        ac,
                        owner,
                        card.name
                    );
                    crate::wasm::network::game_init::process_card_reveal_wasm(
                        shadow,
                        owner,
                        *card,
                        reason,
                        local_player,
                    );
                }
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
        self.opponent_choices.clear();
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
}
