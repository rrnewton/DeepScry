//! WebSocket game server for multiplayer MTG
//!
//! Implements a server that:
//! - Accepts client connections over WebSocket
//! - Handles authentication and deck submission
//! - Matches players (first waits for second)
//! - Runs authoritative game state with NetworkControllers
//! - Broadcasts card reveals and opponent choices

use crate::core::{CardId, PlayerId, SpellAbility};
use crate::game::{GameEndReason, GameLoop, GameResult, GameState};
use crate::loader::{AsyncCardDatabase, DeckEntry, DeckList, GameInitializer};
use crate::network::protocol::{
    now_ms, CardReveal, ChoiceType, ClientMessage, DeckListInfo, DeckSubmission, RevealReason, ServerMessage,
};
use crate::network::{CardRevealInfo, ChoiceRequest, ChoiceResponse, NetworkController, DEFAULT_PORT};
use crate::zones::Zone;
use anyhow::{anyhow, Result};
use futures_util::{SinkExt, StreamExt};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc as tokio_mpsc, oneshot, Mutex};
use tokio_tungstenite::{accept_async, tungstenite::Message, WebSocketStream};

// ═══════════════════════════════════════════════════════════════════════════
// SERVER CONFIGURATION
// ═══════════════════════════════════════════════════════════════════════════

/// Server configuration
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Port to listen on
    pub port: u16,
    /// Password required to join
    pub password: String,
    /// Maximum concurrent games (0 = unlimited)
    pub max_games: usize,
    /// Starting life total
    pub starting_life: i32,
    /// Whether to share deck lists between players (tournament mode)
    pub deck_visibility: bool,
    /// Path to cardsfolder for loading cards
    pub cardsfolder: PathBuf,
    /// Fixed seed for RNG (None = random seed each game)
    pub seed: Option<u64>,
    /// Tag official game action logs with [GAMELOG TurnN STEP] prefix
    pub tag_gamelogs: bool,
    /// Verbosity level for game output
    pub verbosity: crate::game::VerbosityLevel,
    /// Enable network debug mode - populates debug fields in protocol messages
    pub network_debug: bool,
    /// Disable ANSI colored log output
    pub no_color_logs: bool,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            port: DEFAULT_PORT,
            password: String::new(),
            max_games: 0,
            starting_life: 20,
            deck_visibility: false,
            cardsfolder: PathBuf::from("cardsfolder"),
            seed: None,
            tag_gamelogs: false,
            verbosity: crate::game::VerbosityLevel::Normal,
            network_debug: false,
            no_color_logs: false,
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// WAITING PLAYER
// ═══════════════════════════════════════════════════════════════════════════

/// A player waiting for an opponent
struct WaitingPlayer {
    /// Player's display name (None = server should assign default with suffix)
    name: Option<String>,
    /// Submitted deck
    deck: DeckSubmission,
    /// WebSocket connection
    ws_stream: WebSocketStream<TcpStream>,
}

// ═══════════════════════════════════════════════════════════════════════════
// PLAYER CONNECTION
// ═══════════════════════════════════════════════════════════════════════════

/// Connection handler for a single player
/// Information about how a game ended, sent to WebSocket handlers
#[derive(Clone, Debug)]
struct GameEndInfo {
    winner: Option<PlayerId>,
    reason: GameEndReason,
    final_hash: u64,
    action_count: u64,
}

/// Info about an opponent's choice, broadcast to the other player
#[derive(Clone, Debug)]
struct OpponentChoiceInfo {
    /// Choice sequence number
    choice_seq: u32,
    /// Which player made this choice (P1=0, P2=1)
    player: PlayerId,
    /// Type of choice
    choice_type: ChoiceType,
    /// Indices of the chosen options (multiple for attackers/blockers/discard)
    choice_indices: Vec<usize>,
    /// Human-readable description
    description: String,
    /// Action count at time of choice (for sync validation)
    action_count: u64,
    /// Wall-clock timestamp for debugging
    timestamp_ms: u64,
    /// The actual spell ability chosen (for Priority choices)
    /// Allows client to execute the ability directly without computing from hidden hand
    spell_ability: Option<SpellAbility>,
}

/// Card reveal info to broadcast to a player
/// NOTE: Currently unused - reveal broadcasts are disabled to prevent ordering issues.
/// Kept for potential future use when async reveal ordering is fixed.
#[derive(Clone)]
#[allow(dead_code)]
struct RevealBroadcast {
    /// Owner of the card
    owner: PlayerId,
    /// Card ID
    card_id: CardId,
    /// Zone the card moved to
    to_zone: Zone,
}

// ═══════════════════════════════════════════════════════════════════════════
// SINGLE-CHANNEL ARCHITECTURE (mtg-secqu)
// ═══════════════════════════════════════════════════════════════════════════
//
// Each player handler has exactly ONE channel from the game coordinator and
// ONE channel back. This eliminates all `tokio::select!` over multiple channels
// and ensures completely deterministic message ordering.
//
// Design principles:
// 1. Linear control transfer: At any moment, exactly ONE entity has "control"
// 2. Sequential handler loop: Waits for game_rx, then handles message
// 3. Opponent choices flow through game coordinator: Not directly between handlers
// 4. WebSocket I/O is naturally sequential: One message at a time
//
// Architecture:
//
// ┌─────────────────┐     sync      ┌─────────────────┐     async     ┌─────────────────┐
// │ NetworkController├──────────────►│   Coordinator   ├──────────────►│ PlayerHandler   │
// │   (P1)          │◄──────────────┤   Task          │◄──────────────┤   (P1)          │
// └─────────────────┘               │                 │               └─────────────────┘
//                                   │                 │
// ┌─────────────────┐     sync      │                 │     async     ┌─────────────────┐
// │ NetworkController├──────────────►│                 ├──────────────►│ PlayerHandler   │
// │   (P2)          │◄──────────────┤                 │◄──────────────┤   (P2)          │
// └─────────────────┘               └─────────────────┘               └─────────────────┘
//
// Handler loop:
//   loop {
//       // Select between game messages and websocket I/O
//       select! {
//           msg = game_rx.recv() => handle_game_message(msg),
//           ws_msg = ws_rx.next() => handle_ws_message(ws_msg),
//       }
//   }
//
// When ChoiceRequest arrives, handler:
//   1. Sends ChoiceRequest to client
//   2. Waits for SubmitChoice (or queues pending_choice if it arrived early)
//   3. Validates action_count/state_hash
//   4. Sends ChoiceResponse to coordinator
//   5. Coordinator sends ChoiceAccepted + OpponentMadeChoice

/// Messages from game coordinator to player handler.
///
/// All game state messages flow through this single channel, ensuring
/// total ordering and eliminating race conditions.
#[derive(Debug)]
enum GameToHandler {
    /// Server needs this player to make a choice.
    /// Handler should forward to client, wait for response, send back via HandlerToGame.
    ChoiceRequest(ChoiceRequest),
    /// Opponent made a choice - handler should forward to client.
    /// No response expected.
    OpponentMadeChoice(OpponentChoiceInfo),
    /// Acknowledge that player's choice was applied to game state.
    /// Handler should forward to client.
    ChoiceAccepted {
        choice_seq: u32,
        action_count: u64,
        timestamp_ms: u64,
    },
    /// Game has ended normally.
    /// Handler should forward to client and exit.
    GameEnded(GameEndInfo),
    /// Fatal error occurred (desync, disconnect, etc).
    /// Handler should forward to client and exit.
    FatalError(String),
}

/// Messages from player handler to game coordinator.
#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub enum HandlerToGame {
    /// Player submitted their choice response.
    ChoiceResponse {
        response: ChoiceResponse,
        /// Client-reported action count for validation
        client_action_count: u64,
        /// Client state hash (for network_debug validation)
        client_state_hash: Option<u64>,
        /// Debug info from client
        client_debug_info: Option<crate::network::protocol::DebugSyncInfo>,
    },
    /// Client disconnected gracefully or due to error.
    ClientDisconnected,
    /// Client sent invalid data.
    ClientError(String),
}

/// A choice that arrived before the corresponding ChoiceRequest.
/// In synchronized GameLoop mode, the client may compute and submit
/// their choice before the server's request arrives.
#[derive(Debug)]
struct PendingChoice {
    choice_seq: u32,
    choice_indices: Vec<usize>,
    action_count: u64,
    client_state_hash: Option<u64>,
    client_debug_info: Option<crate::network::protocol::DebugSyncInfo>,
    /// The actual spell ability chosen (for Priority choices)
    spell_ability: Option<SpellAbility>,
}

/// Player connection with single-channel architecture.
///
/// Each handler has exactly:
/// - One rx from game coordinator (game_rx)
/// - One tx to game coordinator (game_tx)
/// - WebSocket I/O (ws_tx, handled separately)
struct PlayerConnection {
    /// Player ID in the game
    player_id: PlayerId,
    /// WebSocket sender
    ws_tx: futures_util::stream::SplitSink<WebSocketStream<TcpStream>, Message>,
    /// SINGLE channel to receive all messages from game coordinator
    game_rx: tokio_mpsc::Receiver<GameToHandler>,
    /// SINGLE channel to send all messages to game coordinator
    game_tx: tokio_mpsc::Sender<HandlerToGame>,
    /// Current pending choice from client (arrived before ChoiceRequest)
    pending_choice: Option<PendingChoice>,
}

impl PlayerConnection {
    /// Send a server message to this player
    async fn send(&mut self, msg: &ServerMessage) -> Result<()> {
        let json = serde_json::to_string(msg)?;

        // Log at DEBUG level with truncation for long messages
        if log::log_enabled!(log::Level::Debug) {
            let truncated = if json.len() > 500 {
                format!("{}... ({} bytes total)", &json[..500], json.len())
            } else {
                json.clone()
            };
            log::debug!("[SERVER->P{}] {}", self.player_id, truncated);
        }

        self.ws_tx.send(Message::Text(json.into())).await?;
        Ok(())
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// GAME SERVER
// ═══════════════════════════════════════════════════════════════════════════

/// MTG game server (single-game: runs one game then exits)
pub struct GameServer {
    /// Server configuration
    config: ServerConfig,
    /// Currently waiting player (first to connect)
    waiting_player: Option<WaitingPlayer>,
    /// Game ID counter (used for logging)
    next_game_id: u64,
    /// Card database (shared across games)
    card_db: Option<Arc<AsyncCardDatabase>>,
}

impl GameServer {
    /// Create a new game server
    pub fn new(config: ServerConfig) -> Self {
        Self {
            config,
            waiting_player: None,
            next_game_id: 1,
            card_db: None,
        }
    }

    /// Run the server (blocking)
    ///
    /// This is a single-game server: it accepts exactly two players, runs one game,
    /// and then exits. For a multi-game lobby server, use a different implementation.
    ///
    /// # Errors
    ///
    /// Returns an error if card database loading or TCP binding fails.
    pub async fn run(&mut self) -> Result<()> {
        // Load card database
        log::info!("Loading card database from {:?}...", self.config.cardsfolder);
        let card_db = AsyncCardDatabase::new(self.config.cardsfolder.clone());
        card_db.eager_load().await?;
        log::info!("Card database loaded");
        self.card_db = Some(Arc::new(card_db));

        // Start listening
        let addr = format!("0.0.0.0:{}", self.config.port);
        let listener = TcpListener::bind(&addr).await?;
        log::info!("MTG Server listening on {}", addr);
        log::info!("Password required: {}", !self.config.password.is_empty());

        // Accept connections until we have two players and start a game
        loop {
            match listener.accept().await {
                Ok((stream, addr)) => {
                    log::info!("New connection from {}", addr);
                    match self.handle_connection(stream).await {
                        Ok(Some(game_handle)) => {
                            // Game started - wait for it to complete then exit
                            log::info!("Game started, waiting for completion...");
                            if let Err(e) = game_handle.await {
                                log::error!("Game task error: {}", e);
                            }
                            log::info!("Game completed, server exiting");
                            return Ok(());
                        }
                        Ok(None) => {
                            // First player connected, waiting for second
                        }
                        Err(e) => {
                            log::error!("Connection error: {}", e);
                        }
                    }
                }
                Err(e) => {
                    log::error!("Accept error: {}", e);
                }
            }
        }
    }

    /// Handle a new WebSocket connection
    ///
    /// Returns `Ok(Some(handle))` when a game was started (second player connected),
    /// or `Ok(None)` when still waiting for the second player.
    ///
    /// Note: Wildcard is intentional - ClientMessage has 4+ variants;
    /// we expect Authenticate at connection time, others are errors.
    #[allow(clippy::wildcard_enum_match_arm)]
    async fn handle_connection(&mut self, stream: TcpStream) -> Result<Option<tokio::task::JoinHandle<()>>> {
        let ws_stream = accept_async(stream).await?;
        let (ws_tx, mut ws_rx) = ws_stream.split();

        // Wait for authentication message
        let auth_msg = match ws_rx.next().await {
            Some(Ok(Message::Text(text))) => {
                // Log at DEBUG level with truncation for long messages
                if log::log_enabled!(log::Level::Debug) {
                    let truncated = if text.len() > 500 {
                        format!("{}... ({} bytes total)", &text[..500], text.len())
                    } else {
                        text.to_string()
                    };
                    log::debug!("[CLIENT->SERVER auth] {}", truncated);
                }
                serde_json::from_str::<ClientMessage>(&text)?
            }
            Some(Ok(_)) => return Err(anyhow!("Expected text message")),
            Some(Err(e)) => return Err(e.into()),
            None => return Err(anyhow!("Connection closed before auth")),
        };

        // Reunite the split stream for WaitingPlayer
        let ws_stream = ws_tx.reunite(ws_rx)?;

        match auth_msg {
            ClientMessage::Authenticate {
                password,
                player_name,
                deck,
            } => self.handle_auth(ws_stream, password, player_name, deck).await,
            _ => {
                let mut ws_stream = ws_stream;
                send_error(&mut ws_stream, "Expected authentication message", true).await?;
                Ok(None)
            }
        }
    }

    /// Handle authentication attempt
    ///
    /// Returns `Ok(Some(handle))` when a game was started (second player connected),
    /// or `Ok(None)` when still waiting for the second player or auth failed.
    ///
    /// Player naming logic:
    /// - If player provides a name (Some), use it exactly as-is (no suffix)
    /// - If player doesn't provide a name (None), generate "Player1" or "Player2"
    async fn handle_auth(
        &mut self,
        mut ws_stream: WebSocketStream<TcpStream>,
        password: String,
        player_name: Option<String>,
        deck: DeckSubmission,
    ) -> Result<Option<tokio::task::JoinHandle<()>>> {
        // Check password
        if !self.config.password.is_empty() && password != self.config.password {
            send_message(
                &mut ws_stream,
                &ServerMessage::AuthResult {
                    success: false,
                    error: Some("Invalid password".to_string()),
                    your_player_id: None,
                    your_name: None,
                },
            )
            .await?;
            return Ok(None);
        }

        // Validate deck
        if deck.main_deck_size() < 40 {
            send_message(
                &mut ws_stream,
                &ServerMessage::AuthResult {
                    success: false,
                    error: Some(format!("Deck too small: {} cards (minimum 40)", deck.main_deck_size())),
                    your_player_id: None,
                    your_name: None,
                },
            )
            .await?;
            return Ok(None);
        }

        log::info!(
            "Player '{}' authenticated with {} card deck",
            player_name.as_deref().unwrap_or("<auto>"),
            deck.main_deck_size()
        );

        // Check if we have a waiting player
        if let Some(waiting) = self.waiting_player.take() {
            // Generate player names:
            // - If explicitly provided, use as-is (no suffix)
            // - If None, generate "Player1" or "Player2"
            let p1_name = waiting.name.unwrap_or_else(|| "Player1".to_string());
            let p2_name = player_name.unwrap_or_else(|| "Player2".to_string());

            // Start game with both players
            log::info!("Starting game: {} vs {}", p1_name, p2_name);

            // Send auth success to player 2 with their assigned name
            send_message(
                &mut ws_stream,
                &ServerMessage::AuthResult {
                    success: true,
                    error: None,
                    your_player_id: Some(PlayerId::new(1)),
                    your_name: Some(p2_name.clone()),
                },
            )
            .await?;

            // Send P1's assigned name to them (they were waiting)
            // Note: P1 already received AuthResult when they first connected,
            // but we need to send them their final name now. We do this via
            // updating their name in the WaitingPlayer struct before starting the game.

            // Start the game and return the handle
            let handle = self
                .start_game(
                    WaitingPlayer {
                        name: Some(p1_name),
                        deck: waiting.deck,
                        ws_stream: waiting.ws_stream,
                    },
                    WaitingPlayer {
                        name: Some(p2_name),
                        deck,
                        ws_stream,
                    },
                )
                .await?;
            Ok(Some(handle))
        } else {
            // First player - send auth success and wait
            // Note: We can't assign final name yet because we don't know if they'll be P1 or P2
            // (though in current design, first to connect is always P1)
            // We'll generate their name when P2 connects
            send_message(
                &mut ws_stream,
                &ServerMessage::AuthResult {
                    success: true,
                    error: None,
                    your_player_id: Some(PlayerId::new(0)),
                    // Send the assigned name now if they provided one, otherwise None
                    // If None, their final name will be determined when P2 connects
                    your_name: player_name.clone(),
                },
            )
            .await?;

            send_message(&mut ws_stream, &ServerMessage::WaitingForOpponent).await?;

            self.waiting_player = Some(WaitingPlayer {
                name: player_name,
                deck,
                ws_stream,
            });

            log::info!("Player waiting for opponent...");
            Ok(None)
        }
    }

    /// Start a game between two players
    ///
    /// Returns the task handle for the game, allowing the caller to await completion.
    async fn start_game(&mut self, p1: WaitingPlayer, p2: WaitingPlayer) -> Result<tokio::task::JoinHandle<()>> {
        let game_id = self.next_game_id;
        self.next_game_id += 1;

        let card_db = self.card_db.clone().expect("Card DB not loaded");
        let config = self.config.clone();

        // Spawn game task
        let task_handle = tokio::spawn(async move {
            if let Err(e) = run_game(game_id, p1, p2, card_db, config).await {
                log::error!("Game {} error: {}", game_id, e);
            }
        });

        Ok(task_handle)
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// GAME EXECUTION
// ═══════════════════════════════════════════════════════════════════════════

/// Convert DeckSubmission to DeckList
fn submission_to_decklist(submission: &DeckSubmission) -> DeckList {
    DeckList {
        main_deck: submission
            .main_deck
            .iter()
            .map(|(name, count)| DeckEntry {
                card_name: name.clone(),
                count: *count,
            })
            .collect(),
        sideboard: submission
            .sideboard
            .iter()
            .map(|(name, count)| DeckEntry {
                card_name: name.clone(),
                count: *count,
            })
            .collect(),
    }
}

/// Run a single game between two players
///
/// Note: Wildcard is intentional - GameEndReason enum has several variants;
/// we handle PlayerDeath/Decking specially and derive from winner for others.
#[allow(clippy::wildcard_enum_match_arm)]
async fn run_game(
    game_id: u64,
    p1: WaitingPlayer,
    p2: WaitingPlayer,
    card_db: Arc<AsyncCardDatabase>,
    config: ServerConfig,
) -> Result<()> {
    // Extract final names (should always be Some at this point)
    let p1_name = p1.name.clone().unwrap_or_else(|| "Player1".to_string());
    let p2_name = p2.name.clone().unwrap_or_else(|| "Player2".to_string());
    log::info!("Game {}: Initializing {} vs {}", game_id, p1_name, p2_name);

    // ═══════════════════════════════════════════════════════════════════════
    // SINGLE-CHANNEL ARCHITECTURE (mtg-secqu)
    // ═══════════════════════════════════════════════════════════════════════
    //
    // The architecture has three layers:
    // 1. NetworkControllers (sync) - run in spawn_blocking, drive game loop
    // 2. Coordinator task (async) - bridges sync/async, routes messages
    // 3. Player handlers (async) - handle WebSocket I/O
    //
    // Messages flow:
    //   NetworkController --sync--> Coordinator --async--> Handler --WS--> Client
    //   NetworkController <--sync-- Coordinator <--async-- Handler <--WS-- Client

    // Create sync channels for NetworkControllers (used by game loop in blocking thread)
    let (p1_request_tx, p1_sync_request_rx) = std::sync::mpsc::channel::<ChoiceRequest>();
    let (p1_response_tx, p1_response_rx) = std::sync::mpsc::channel::<ChoiceResponse>();
    let (p2_request_tx, p2_sync_request_rx) = std::sync::mpsc::channel::<ChoiceRequest>();
    let (p2_response_tx, p2_response_rx) = std::sync::mpsc::channel::<ChoiceResponse>();

    // Create SINGLE async channel pairs for handler communication (mtg-secqu)
    // Each player has exactly one rx and one tx with the coordinator
    let (p1_to_handler_tx, p1_game_rx) = tokio_mpsc::channel::<GameToHandler>(16);
    let (p1_from_handler_tx, p1_handler_rx) = tokio_mpsc::channel::<HandlerToGame>(16);
    let (p2_to_handler_tx, p2_game_rx) = tokio_mpsc::channel::<GameToHandler>(16);
    let (p2_from_handler_tx, p2_handler_rx) = tokio_mpsc::channel::<HandlerToGame>(16);

    // Split WebSocket streams
    let (p1_ws_tx, p1_ws_rx) = p1.ws_stream.split();
    let (p2_ws_tx, p2_ws_rx) = p2.ws_stream.split();

    // Create PlayerConnections with single-channel architecture
    let mut p1_conn = PlayerConnection {
        player_id: PlayerId::new(0),
        ws_tx: p1_ws_tx,
        game_rx: p1_game_rx,
        game_tx: p1_from_handler_tx,
        pending_choice: None,
    };
    let mut p2_conn = PlayerConnection {
        player_id: PlayerId::new(1),
        ws_tx: p2_ws_tx,
        game_rx: p2_game_rx,
        game_tx: p2_from_handler_tx,
        pending_choice: None,
    };

    // Convert deck submissions to DeckList format
    let p1_decklist = submission_to_decklist(&p1.deck);
    let p2_decklist = submission_to_decklist(&p2.deck);

    // Debug: log deck order for entity ID verification
    log::debug!(
        "Server init: p1_deck entries={}, p2_deck entries={}",
        p1_decklist.main_deck.len(),
        p2_decklist.main_deck.len()
    );
    if log::log_enabled!(log::Level::Trace) {
        for (i, entry) in p1_decklist.main_deck.iter().enumerate() {
            log::trace!("P1 deck[{}]: {}x {}", i, entry.count, entry.card_name);
        }
        for (i, entry) in p2_decklist.main_deck.iter().enumerate() {
            log::trace!("P2 deck[{}]: {}x {}", i, entry.count, entry.card_name);
        }
    }

    // Seed RNG and shuffle libraries BEFORE assigning CardIDs (positional ID architecture)
    // This ensures CardID 0 = top card of P1's shuffled library, etc.
    let seed = config.seed.unwrap_or_else(rand::random::<u64>);
    log::info!("Game {}: Using seed {}", game_id, seed);

    // Create game state using GameInitializer with positional CardIDs
    // This shuffles decks BEFORE assigning CardIDs so that:
    // - P1's cards get CardIDs [0..P1_deck_size)
    // - P2's cards get CardIDs [P1_deck_size..total)
    // Clients use init_game_reserve_only with the same ranges
    let initializer = GameInitializer::new(&card_db);
    let mut game = initializer
        .init_game_with_positional_ids(
            p1_name.clone(),
            &p1_decklist,
            p2_name.clone(),
            &p2_decklist,
            config.starting_life,
            seed,
        )
        .await?;

    // Enable reveal logging for network games.
    // RevealCard actions are logged during draw_card, play_land, etc.
    // These are collected by NetworkController and sent to clients as CardRevealed messages.
    game.set_skip_reveals(false);

    let p1_id = game.players[0].id;
    let p2_id = game.players[1].id;

    // Peek at opening hands WITHOUT drawing
    // We don't draw yet because that would add actions to undo_log before GameLoop starts.
    // Both server and client GameLoops need to start with identical (empty) undo_logs
    // so they can draw synchronously and produce matching action sequences.
    let p1_hand = peek_opening_hand(&game, p1_id)?;
    let p2_hand = peek_opening_hand(&game, p2_id)?;

    // Compute initial state hash
    let initial_hash = compute_network_hash(&game);

    // ALWAYS send deck lists for synchronized GameLoop mode.
    // Clients need the opponent's deck list to create matching card IDs.
    // The deck_visibility config is a separate UI concern (whether players
    // can VIEW opponent's decklist), not whether we transmit it for sync.
    let p1_deck_info = Some(DeckListInfo::from_submission(&p1.deck));
    let p2_deck_info = Some(DeckListInfo::from_submission(&p2.deck));

    // Compute deck CardID ranges for late-binding architecture (Phase 3)
    // P1's deck: CardIDs [0, p1_deck_size)
    // P2's deck: CardIDs [p1_deck_size, p1_deck_size + p2_deck_size)
    let p1_deck_size = p1.deck.main_deck_size();
    let p2_deck_size = p2.deck.main_deck_size();
    let deck_card_ids = Some(crate::network::protocol::DeckCardIdRanges::from_deck_sizes(
        p1_deck_size,
        p2_deck_size,
    ));

    // Send GameStarted to both players
    let p1_lib_size = game.player_zones[0].1.library.len();
    let p2_lib_size = game.player_zones[1].1.library.len();

    // Build token_definitions map for network transmission
    // Convert from HashMap<String, Arc<CardDefinition>> to HashMap<String, CardDefinition>
    let token_definitions: std::collections::HashMap<String, crate::loader::CardDefinition> = game
        .token_definitions
        .iter()
        .map(|(k, v)| (k.clone(), (**v).clone()))
        .collect();

    // Serialize RNG state for clients to initialize their shadow RNG
    // This ensures subsequent shuffles (tutors, etc.) produce identical results
    let rng_state = {
        let rng = game.rng.borrow();
        bincode::serialize(&*rng).unwrap_or_else(|e| {
            log::error!("Failed to serialize RNG state: {}", e);
            Vec::new()
        })
    };
    log::debug!(
        "Game {}: Serialized RNG state ({} bytes) for client sync",
        game_id,
        rng_state.len()
    );

    p1_conn
        .send(&ServerMessage::GameStarted {
            your_player_id: p1_id,
            opponent_name: p2_name.clone(),
            opening_hand: p1_hand.clone(),
            opponent_hand_count: p2_hand.len(),
            library_size: p1_lib_size,
            opponent_library_size: p2_lib_size,
            opponent_decklist: p2_deck_info.clone(),
            starting_life: config.starting_life,
            initial_state_hash: initial_hash,
            network_debug: config.network_debug,
            deck_card_ids: deck_card_ids.clone(),
            token_definitions: token_definitions.clone(),
            rng_state: rng_state.clone(),
        })
        .await?;

    p2_conn
        .send(&ServerMessage::GameStarted {
            your_player_id: p2_id,
            opponent_name: p1_name.clone(),
            opening_hand: p2_hand.clone(),
            opponent_hand_count: p1_hand.len(),
            library_size: p2_lib_size,
            opponent_library_size: p1_lib_size,
            opponent_decklist: p1_deck_info.clone(),
            starting_life: config.starting_life,
            initial_state_hash: initial_hash,
            network_debug: config.network_debug,
            deck_card_ids: deck_card_ids.clone(),
            token_definitions: token_definitions.clone(),
            rng_state: rng_state.clone(),
        })
        .await?;

    log::info!("Game {}: Sent GameStarted to both players", game_id);

    // Send CardRevealed messages for opening hands (for synchronized GameLoop mode)
    // ALL players receive reveals for ALL cards to keep action_count in sync.
    // But opponent's cards are sent as "dummy reveals" with name stripped.
    //
    // HIDDEN INFO ARCHITECTURE (mtg-qtqcr):
    // - Own cards: real reveal with name (player can see their hand)
    // - Opponent cards: dummy reveal with empty name (keeps count synced, reveals nothing)

    // P1 receives: own hand (real reveals) + P2's hand (dummy reveals)
    for card in &p1_hand {
        p1_conn
            .send(&ServerMessage::CardRevealed {
                owner: p1_id,
                card: card.clone(), // Real reveal - P1 sees own cards
                reason: RevealReason::Draw,
            })
            .await?;
    }
    for card in &p2_hand {
        // Dummy reveal: strip name so P1 can't see P2's hand
        let dummy_reveal = CardReveal {
            card_id: card.card_id,
            name: String::new(), // Hidden - P1 doesn't know what card this is
            card_def: None,      // No definition for dummy reveals
        };
        p1_conn
            .send(&ServerMessage::CardRevealed {
                owner: p2_id,
                card: dummy_reveal,
                reason: RevealReason::Draw,
            })
            .await?;
    }

    // P2 receives: P1's hand (dummy reveals) + own hand (real reveals)
    for card in &p1_hand {
        // Dummy reveal: strip name so P2 can't see P1's hand
        let dummy_reveal = CardReveal {
            card_id: card.card_id,
            name: String::new(), // Hidden - P2 doesn't know what card this is
            card_def: None,      // No definition for dummy reveals
        };
        p2_conn
            .send(&ServerMessage::CardRevealed {
                owner: p1_id,
                card: dummy_reveal,
                reason: RevealReason::Draw,
            })
            .await?;
    }
    for card in &p2_hand {
        p2_conn
            .send(&ServerMessage::CardRevealed {
                owner: p2_id,
                card: card.clone(), // Real reveal - P2 sees own cards
                reason: RevealReason::Draw,
            })
            .await?;
    }

    log::info!("Game {}: Sent opening hand CardRevealed messages", game_id);

    // Calculate baseline reveal index to skip the opening hand draws
    // The undo_log will have p1_hand.len() + p2_hand.len() MoveCard entries
    // after GameLoop draws the opening hands. We've already sent these reveals.
    let opening_hand_count = p1_hand.len() + p2_hand.len();
    log::debug!(
        "Game {}: Opening hand reveal count = {} (will skip in first ChoiceRequest)",
        game_id,
        opening_hand_count
    );

    // Create shared reveal indices for NetworkControllers
    // Initialize to opening_hand_count to skip opening hand reveals (already sent)
    let p1_reveal_index = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(opening_hand_count));
    let p2_reveal_index = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(opening_hand_count));

    // Create NetworkControllers with shared reveal indices
    let mut p1_controller = NetworkController::new(p1_id, p1_request_tx, p1_response_rx, Arc::clone(&p1_reveal_index));
    let mut p2_controller = NetworkController::new(p2_id, p2_request_tx, p2_response_rx, Arc::clone(&p2_reveal_index));
    p1_controller.set_network_debug(config.network_debug);
    p2_controller.set_network_debug(config.network_debug);

    // Wrap game state for sharing between tasks
    let game = Arc::new(Mutex::new(game));

    // Spawn WebSocket handlers for each player
    let game_clone = Arc::clone(&game);
    let mut p1_handler = tokio::spawn(async move { handle_player_websocket(p1_conn, p1_ws_rx, game_clone).await });

    let game_clone = Arc::clone(&game);
    let mut p2_handler = tokio::spawn(async move { handle_player_websocket(p2_conn, p2_ws_rx, game_clone).await });

    // Create channel for game end notification to coordinator
    let (game_end_tx, game_end_rx) = oneshot::channel::<GameEndInfo>();

    // Spawn coordinator task that bridges sync NetworkControllers to async handlers
    // The coordinator receives ChoiceRequests from NetworkControllers (via sync channels),
    // forwards them to handlers (via async channels), and routes responses back.
    let coordinator_network_debug = config.network_debug;
    let mut coordinator_handle = tokio::spawn(async move {
        run_coordinator(
            p1_sync_request_rx,
            p1_response_tx,
            p1_to_handler_tx,
            p1_handler_rx,
            p2_sync_request_rx,
            p2_response_tx,
            p2_to_handler_tx,
            p2_handler_rx,
            coordinator_network_debug,
            game_end_rx,
        )
        .await
    });

    // Run game loop in blocking thread (uses sync channels)
    let game_clone = Arc::clone(&game);
    let tag_gamelogs = config.tag_gamelogs;
    let verbosity = config.verbosity;
    let no_color_logs = config.no_color_logs;
    let game_loop_handle = tokio::task::spawn_blocking(move || {
        run_game_loop(
            game_clone,
            p1_controller,
            p2_controller,
            tag_gamelogs,
            verbosity,
            no_color_logs,
        )
    });

    // Wait for game to complete, OR for any critical task to fail
    // This prevents the server from hanging when a desync is detected
    let result: Result<GameResult> = tokio::select! {
        // Game loop completed (success or error)
        game_result = game_loop_handle => {
            match game_result {
                Ok(r) => r,
                Err(e) => Err(anyhow!("Game loop panic: {}", e)),
            }
        }
        // Coordinator exited (error means fatal issue)
        coord_result = &mut coordinator_handle => {
            log::error!("Game {}: Coordinator exited unexpectedly: {:?}", game_id, coord_result);
            Err(anyhow!("Coordinator terminated unexpectedly"))
        }
        // P1 WebSocket handler exited (error means fatal issue like desync)
        p1_result = &mut p1_handler => {
            log::error!("Game {}: P1 handler exited unexpectedly: {:?}", game_id, p1_result);
            Err(anyhow!("P1 connection terminated unexpectedly"))
        }
        // P2 WebSocket handler exited (error means fatal issue like desync)
        p2_result = &mut p2_handler => {
            log::error!("Game {}: P2 handler exited unexpectedly: {:?}", game_id, p2_result);
            Err(anyhow!("P2 connection terminated unexpectedly"))
        }
    };

    // Get final state hash for the GameEnded message
    let final_hash = {
        let game_guard = game.lock().await;
        compute_network_hash(&game_guard)
    };

    // Send game end notification to coordinator, which will forward to handlers
    match &result {
        Ok(game_result) => {
            log::info!(
                "Game {}: Completed, winner = {:?}, action_count = {}",
                game_id,
                game_result.winner,
                game_result.action_count
            );

            // Derive reason from game result
            let reason = match game_result.end_reason {
                GameEndReason::PlayerDeath(_) | GameEndReason::Decking(_) => game_result.end_reason.clone(),
                _ => match game_result.winner {
                    Some(winner_id) => {
                        let loser_id = if winner_id == PlayerId::new(0) {
                            PlayerId::new(1)
                        } else {
                            PlayerId::new(0)
                        };
                        GameEndReason::PlayerDeath(loser_id)
                    }
                    None => GameEndReason::Draw,
                },
            };

            let end_info = GameEndInfo {
                winner: game_result.winner,
                reason,
                final_hash,
                action_count: game_result.action_count,
            };
            let _ = game_end_tx.send(end_info);
        }
        Err(e) => {
            log::error!("Game {}: Error - {}", game_id, e);
            let end_info = GameEndInfo {
                winner: None,
                reason: GameEndReason::Draw,
                final_hash,
                action_count: 0,
            };
            let _ = game_end_tx.send(end_info);
        }
    }

    // Wait for coordinator and handlers to process GameEnded
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    coordinator_handle.abort();
    p1_handler.abort();
    p2_handler.abort();

    // Return the final result
    result.map(|_| ())
}

// ═══════════════════════════════════════════════════════════════════════════
// COORDINATOR TASK (mtg-secqu)
// ═══════════════════════════════════════════════════════════════════════════
//
// The coordinator bridges sync NetworkControllers to async WebSocket handlers.
// It runs a blocking receiver in spawn_blocking, and routes messages to handlers.
//
// Key responsibilities:
// 1. Receive ChoiceRequest from NetworkController (sync) via spawn_blocking bridge
// 2. Forward to appropriate handler (async)
// 3. Receive ChoiceResponse from handler (async)
// 4. Send response to NetworkController (sync)
// 5. Route OpponentMadeChoice to the other handler
// 6. Send ChoiceAccepted to the originating handler

/// Run the coordinator task that bridges sync NetworkControllers to async handlers.
///
/// This task uses spawn_blocking to receive from sync channels, then routes
/// messages through the single async channel to each handler.
#[allow(clippy::too_many_arguments)]
async fn run_coordinator(
    p1_sync_request_rx: std::sync::mpsc::Receiver<ChoiceRequest>,
    p1_response_tx: std::sync::mpsc::Sender<ChoiceResponse>,
    p1_to_handler_tx: tokio_mpsc::Sender<GameToHandler>,
    mut p1_handler_rx: tokio_mpsc::Receiver<HandlerToGame>,
    p2_sync_request_rx: std::sync::mpsc::Receiver<ChoiceRequest>,
    p2_response_tx: std::sync::mpsc::Sender<ChoiceResponse>,
    p2_to_handler_tx: tokio_mpsc::Sender<GameToHandler>,
    mut p2_handler_rx: tokio_mpsc::Receiver<HandlerToGame>,
    network_debug: bool,
    mut game_end_rx: oneshot::Receiver<GameEndInfo>,
) -> Result<()> {
    // Spawn blocking bridge tasks that convert sync channel messages to async
    let (p1_bridge_tx, mut p1_bridge_rx) = tokio_mpsc::channel::<ChoiceRequest>(4);
    let (p2_bridge_tx, mut p2_bridge_rx) = tokio_mpsc::channel::<ChoiceRequest>(4);

    let _p1_bridge = tokio::task::spawn_blocking(move || {
        while let Ok(request) = p1_sync_request_rx.recv() {
            if p1_bridge_tx.blocking_send(request).is_err() {
                break;
            }
        }
    });

    let _p2_bridge = tokio::task::spawn_blocking(move || {
        while let Ok(request) = p2_sync_request_rx.recv() {
            if p2_bridge_tx.blocking_send(request).is_err() {
                break;
            }
        }
    });

    log::debug!("Coordinator: Started, network_debug={}", network_debug);

    // Main coordinator loop: wait for choice requests from either player OR game end
    loop {
        tokio::select! {
            // Game ended - forward to both handlers and exit
            end_info = &mut game_end_rx => {
                match end_info {
                    Ok(info) => {
                        log::info!("Coordinator: Received GameEnded, winner={:?}", info.winner);
                        // Send GameEnded to both handlers
                        let _ = p1_to_handler_tx.send(GameToHandler::GameEnded(info.clone())).await;
                        let _ = p2_to_handler_tx.send(GameToHandler::GameEnded(info)).await;
                        return Ok(());
                    }
                    Err(_) => {
                        log::debug!("Coordinator: Game end channel closed unexpectedly");
                        return Ok(());
                    }
                }
            }

            // P1 NetworkController sent a ChoiceRequest
            request = p1_bridge_rx.recv() => {
                match request {
                    Some(choice_request) => {
                        log::debug!(
                            "Coordinator: P1 ChoiceRequest seq={} action_count={}",
                            choice_request.choice_seq, choice_request.action_count
                        );

                        // Store request info for later (for OpponentMadeChoice)
                        let choice_seq = choice_request.choice_seq;
                        let choice_type = choice_request.choice_type.clone();
                        let action_count = choice_request.action_count;
                        let abilities = choice_request.abilities.clone();

                        // Forward to P1 handler
                        if p1_to_handler_tx.send(GameToHandler::ChoiceRequest(choice_request)).await.is_err() {
                            log::error!("Coordinator: Failed to send ChoiceRequest to P1 handler");
                            return Err(anyhow!("P1 handler channel closed"));
                        }

                        // Wait for P1's response
                        match p1_handler_rx.recv().await {
                            Some(HandlerToGame::ChoiceResponse { response, client_action_count, client_state_hash, client_debug_info }) => {
                                log::debug!(
                                    "Coordinator: P1 ChoiceResponse seq={} indices={:?}",
                                    response.choice_seq, response.choice_indices
                                );

                                // Validate action_count
                                if client_action_count != action_count {
                                    let error_msg = format!(
                                        "FATAL: P1 action_count mismatch! client={} expected={}",
                                        client_action_count, action_count
                                    );
                                    log::error!("Coordinator: {}", error_msg);
                                    // Send fatal error to both handlers
                                    let _ = p1_to_handler_tx.send(GameToHandler::FatalError(error_msg.clone())).await;
                                    let _ = p2_to_handler_tx.send(GameToHandler::FatalError(error_msg.clone())).await;
                                    return Err(anyhow!("{}", error_msg));
                                }

                                // Validate state hash in network debug mode
                                // TODO(mtg-secqu): Add state hash validation here
                                let _ = (network_debug, client_state_hash, client_debug_info);

                                // Send response to NetworkController
                                if p1_response_tx.send(response.clone()).is_err() {
                                    log::error!("Coordinator: Failed to send response to P1 NetworkController");
                                    return Err(anyhow!("P1 NetworkController channel closed"));
                                }

                                // Send ChoiceAccepted to P1
                                let _ = p1_to_handler_tx.send(GameToHandler::ChoiceAccepted {
                                    choice_seq,
                                    action_count,
                                    timestamp_ms: now_ms(),
                                }).await;

                                // Extract spell_ability for Priority choices
                                let spell_ability = if matches!(choice_type, ChoiceType::Priority { .. }) {
                                    let idx = response.choice_indices.first().copied().unwrap_or(0);
                                    abilities.as_ref()
                                        .and_then(|abs| abs.get(idx).cloned())
                                        .flatten()
                                } else {
                                    None
                                };

                                // Send OpponentMadeChoice to P2
                                let opponent_info = OpponentChoiceInfo {
                                    choice_seq,
                                    player: PlayerId::new(0),
                                    choice_type,
                                    choice_indices: response.choice_indices,
                                    description: format!("P1 choice #{}", choice_seq),
                                    action_count,
                                    timestamp_ms: now_ms(),
                                    spell_ability,
                                };
                                let _ = p2_to_handler_tx.send(GameToHandler::OpponentMadeChoice(opponent_info)).await;
                            }
                            Some(HandlerToGame::ClientDisconnected) => {
                                log::info!("Coordinator: P1 disconnected");
                                let _ = p2_to_handler_tx.send(GameToHandler::FatalError("Opponent disconnected".to_string())).await;
                                return Err(anyhow!("P1 disconnected"));
                            }
                            Some(HandlerToGame::ClientError(msg)) => {
                                log::error!("Coordinator: P1 client error: {}", msg);
                                let _ = p2_to_handler_tx.send(GameToHandler::FatalError(format!("Opponent error: {}", msg))).await;
                                return Err(anyhow!("P1 client error: {}", msg));
                            }
                            None => {
                                log::error!("Coordinator: P1 handler channel closed");
                                return Err(anyhow!("P1 handler channel closed unexpectedly"));
                            }
                        }
                    }
                    None => {
                        // Bridge channel closed - game loop has ended
                        // Don't break immediately, wait for game_end_rx which will arrive shortly
                        log::debug!("Coordinator: P1 bridge closed, waiting for game_end");
                    }
                }
            }

            // P2 NetworkController sent a ChoiceRequest
            request = p2_bridge_rx.recv() => {
                match request {
                    Some(choice_request) => {
                        log::debug!(
                            "Coordinator: P2 ChoiceRequest seq={} action_count={}",
                            choice_request.choice_seq, choice_request.action_count
                        );

                        // Store request info for later
                        let choice_seq = choice_request.choice_seq;
                        let choice_type = choice_request.choice_type.clone();
                        let action_count = choice_request.action_count;
                        let abilities = choice_request.abilities.clone();

                        // Forward to P2 handler
                        if p2_to_handler_tx.send(GameToHandler::ChoiceRequest(choice_request)).await.is_err() {
                            log::error!("Coordinator: Failed to send ChoiceRequest to P2 handler");
                            return Err(anyhow!("P2 handler channel closed"));
                        }

                        // Wait for P2's response
                        match p2_handler_rx.recv().await {
                            Some(HandlerToGame::ChoiceResponse { response, client_action_count, client_state_hash, client_debug_info }) => {
                                log::debug!(
                                    "Coordinator: P2 ChoiceResponse seq={} indices={:?}",
                                    response.choice_seq, response.choice_indices
                                );

                                // Validate action_count
                                if client_action_count != action_count {
                                    let error_msg = format!(
                                        "FATAL: P2 action_count mismatch! client={} expected={}",
                                        client_action_count, action_count
                                    );
                                    log::error!("Coordinator: {}", error_msg);
                                    // Send fatal error to both handlers
                                    let _ = p1_to_handler_tx.send(GameToHandler::FatalError(error_msg.clone())).await;
                                    let _ = p2_to_handler_tx.send(GameToHandler::FatalError(error_msg.clone())).await;
                                    return Err(anyhow!("{}", error_msg));
                                }

                                // Validate state hash in network debug mode
                                let _ = (network_debug, client_state_hash, client_debug_info);

                                // Send response to NetworkController
                                if p2_response_tx.send(response.clone()).is_err() {
                                    log::error!("Coordinator: Failed to send response to P2 NetworkController");
                                    return Err(anyhow!("P2 NetworkController channel closed"));
                                }

                                // Send ChoiceAccepted to P2
                                let _ = p2_to_handler_tx.send(GameToHandler::ChoiceAccepted {
                                    choice_seq,
                                    action_count,
                                    timestamp_ms: now_ms(),
                                }).await;

                                // Extract spell_ability for Priority choices
                                let spell_ability = if matches!(choice_type, ChoiceType::Priority { .. }) {
                                    let idx = response.choice_indices.first().copied().unwrap_or(0);
                                    abilities.as_ref()
                                        .and_then(|abs| abs.get(idx).cloned())
                                        .flatten()
                                } else {
                                    None
                                };

                                // Send OpponentMadeChoice to P1
                                let opponent_info = OpponentChoiceInfo {
                                    choice_seq,
                                    player: PlayerId::new(1),
                                    choice_type,
                                    choice_indices: response.choice_indices,
                                    description: format!("P2 choice #{}", choice_seq),
                                    action_count,
                                    timestamp_ms: now_ms(),
                                    spell_ability,
                                };
                                let _ = p1_to_handler_tx.send(GameToHandler::OpponentMadeChoice(opponent_info)).await;
                            }
                            Some(HandlerToGame::ClientDisconnected) => {
                                log::info!("Coordinator: P2 disconnected");
                                let _ = p1_to_handler_tx.send(GameToHandler::FatalError("Opponent disconnected".to_string())).await;
                                return Err(anyhow!("P2 disconnected"));
                            }
                            Some(HandlerToGame::ClientError(msg)) => {
                                log::error!("Coordinator: P2 client error: {}", msg);
                                let _ = p1_to_handler_tx.send(GameToHandler::FatalError(format!("Opponent error: {}", msg))).await;
                                return Err(anyhow!("P2 client error: {}", msg));
                            }
                            None => {
                                log::error!("Coordinator: P2 handler channel closed");
                                return Err(anyhow!("P2 handler channel closed unexpectedly"));
                            }
                        }
                    }
                    None => {
                        // Bridge channel closed - game loop has ended
                        // Don't break immediately, wait for game_end_rx which will arrive shortly
                        log::debug!("Coordinator: P2 bridge closed, waiting for game_end");
                    }
                }
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// PLAYER WEBSOCKET HANDLER (mtg-secqu)
// ═══════════════════════════════════════════════════════════════════════════
//
// New single-channel architecture:
// - All game state messages come through conn.game_rx (GameToHandler)
// - All responses go through conn.game_tx (HandlerToGame)
// - WebSocket is just for client I/O
// - No direct handler-to-handler communication

/// Handle WebSocket messages for a player using single-channel architecture.
///
/// The handler loop is simple:
/// 1. Wait for game_rx message (from coordinator) OR websocket message (from client)
/// 2. Handle appropriately:
///    - ChoiceRequest: forward to client, wait for response
///    - OpponentMadeChoice: forward to client
///    - ChoiceAccepted: forward to client
///    - GameEnded/FatalError: forward and exit
///    - Client messages: process or queue
async fn handle_player_websocket(
    mut conn: PlayerConnection,
    mut ws_rx: futures_util::stream::SplitStream<WebSocketStream<TcpStream>>,
    game: Arc<Mutex<GameState>>,
) -> Result<()> {
    log::debug!("Handler P{}: Started", conn.player_id);

    // Track if we're currently waiting for a choice from the client
    let mut waiting_for_choice: Option<ChoiceRequest> = None;

    loop {
        tokio::select! {
            // Messages from coordinator (game state)
            game_msg = conn.game_rx.recv() => {
                match game_msg {
                    Some(GameToHandler::ChoiceRequest(choice_request)) => {
                        log::debug!(
                            "Handler P{}: Received ChoiceRequest seq={} action_count={}",
                            conn.player_id, choice_request.choice_seq, choice_request.action_count
                        );

                        // Send CardRevealed messages for reveals in this request
                        if !choice_request.reveals.is_empty() {
                            let game_guard = game.lock().await;
                            for reveal_info in &choice_request.reveals {
                                if let Some(card_reveal) = build_card_reveal(&game_guard, reveal_info) {
                                    let reason = zone_to_reveal_reason(reveal_info.to_zone);
                                    conn.send(&ServerMessage::CardRevealed {
                                        owner: reveal_info.owner,
                                        card: card_reveal,
                                        reason,
                                    }).await?;
                                }
                            }
                        }

                        // Check if client already sent a choice (pending_choice)
                        if let Some(pending) = conn.pending_choice.take() {
                            log::debug!(
                                "Handler P{}: Processing pending choice {} (arrived before ChoiceRequest)",
                                conn.player_id, pending.choice_seq
                            );

                            // Send response to coordinator, including spell_ability for robust matching
                            let response = ChoiceResponse {
                                choice_seq: pending.choice_seq,
                                choice_indices: pending.choice_indices,
                                spell_ability: pending.spell_ability,
                            };
                            conn.game_tx.send(HandlerToGame::ChoiceResponse {
                                response,
                                client_action_count: pending.action_count,
                                client_state_hash: pending.client_state_hash,
                                client_debug_info: pending.client_debug_info,
                            }).await?;
                        } else {
                            // Normal case: send ChoiceRequest to client and wait
                            conn.send(&ServerMessage::ChoiceRequest {
                                choice_seq: choice_request.choice_seq,
                                for_player: conn.player_id,
                                choice_type: choice_request.choice_type.clone(),
                                options: choice_request.options.clone(),
                                state_hash: choice_request.state_hash,
                                action_count: choice_request.action_count,
                                timestamp_ms: now_ms(),
                                context: None,
                                debug_info: choice_request.debug_info.clone(),
                                abilities: choice_request.abilities.clone(),
                            }).await?;

                            // Mark that we're waiting for this choice
                            waiting_for_choice = Some(choice_request);
                        }
                    }

                    Some(GameToHandler::OpponentMadeChoice(info)) => {
                        log::debug!(
                            "Handler P{}: Forwarding opponent choice seq={}",
                            conn.player_id, info.choice_seq
                        );

                        // If opponent played a card, send CardRevealed first
                        if let Some(ref ability) = info.spell_ability {
                            let card_id = ability.card_id();
                            let game_guard = game.lock().await;
                            if let Some(card) = game_guard.cards.try_get(card_id) {
                                let card_def = game_guard.card_definitions.get(&card.name).cloned();
                                let card_reveal = CardReveal {
                                    card_id,
                                    name: card.name.to_string(),
                                    card_def,
                                };
                                conn.send(&ServerMessage::CardRevealed {
                                    owner: info.player,
                                    card: card_reveal,
                                    reason: RevealReason::Played,
                                }).await?;
                            }
                        }

                        conn.send(&ServerMessage::OpponentChoice {
                            choice_seq: info.choice_seq,
                            player: info.player,
                            choice_type: info.choice_type,
                            choice_indices: info.choice_indices,
                            description: info.description,
                            action_count: info.action_count,
                            timestamp_ms: info.timestamp_ms,
                            spell_ability: info.spell_ability,
                            state_hash_after: None,
                            debug_info: None,
                        }).await?;
                    }

                    Some(GameToHandler::ChoiceAccepted { choice_seq, action_count, timestamp_ms }) => {
                        log::debug!(
                            "Handler P{}: Forwarding ChoiceAccepted seq={}",
                            conn.player_id, choice_seq
                        );
                        conn.send(&ServerMessage::ChoiceAccepted {
                            choice_seq,
                            action_count,
                            timestamp_ms,
                        }).await?;
                    }

                    Some(GameToHandler::GameEnded(info)) => {
                        log::info!(
                            "Handler P{}: Sending GameEnded winner={:?}",
                            conn.player_id, info.winner
                        );
                        conn.send(&ServerMessage::GameEnded {
                            winner: info.winner,
                            reason: info.reason,
                            final_state_hash: info.final_hash,
                            action_count: info.action_count,
                        }).await?;
                        break;
                    }

                    Some(GameToHandler::FatalError(msg)) => {
                        log::error!("Handler P{}: Fatal error: {}", conn.player_id, msg);
                        conn.send(&ServerMessage::Error {
                            message: msg,
                            fatal: true,
                        }).await?;
                        break;
                    }

                    None => {
                        // Coordinator channel closed
                        log::debug!("Handler P{}: Coordinator channel closed", conn.player_id);
                        break;
                    }
                }
            }

            // Messages from client (WebSocket)
            ws_msg = ws_rx.next() => {
                match ws_msg {
                    Some(Ok(Message::Text(text))) => {
                        if log::log_enabled!(log::Level::Debug) {
                            let truncated = if text.len() > 500 {
                                format!("{}... ({} bytes)", &text[..500], text.len())
                            } else {
                                text.to_string()
                            };
                            log::debug!("[P{}->SERVER] {}", conn.player_id, truncated);
                        }

                        match serde_json::from_str::<ClientMessage>(&text) {
                            Ok(ClientMessage::SubmitChoice {
                                choice_seq,
                                choice_indices,
                                action_count,
                                client_state_hash,
                                debug_info,
                                spell_ability,
                                ..
                            }) => {
                                if waiting_for_choice.take().is_some() {
                                    // Normal case: we sent ChoiceRequest and client is responding
                                    log::debug!(
                                        "Handler P{}: Received choice seq={} action_count={} spell_ability={:?}",
                                        conn.player_id, choice_seq, action_count, spell_ability.as_ref().map(|a| format!("{:?}", a))
                                    );

                                    // Send response to coordinator, including spell_ability for robust matching
                                    let response = ChoiceResponse { choice_seq, choice_indices, spell_ability };
                                    conn.game_tx.send(HandlerToGame::ChoiceResponse {
                                        response,
                                        client_action_count: action_count,
                                        client_state_hash,
                                        client_debug_info: debug_info,
                                    }).await?;
                                } else {
                                    // Client is ahead: queue for later
                                    log::debug!(
                                        "Handler P{}: Queueing early choice seq={} (waiting for ChoiceRequest)",
                                        conn.player_id, choice_seq
                                    );
                                    conn.pending_choice = Some(PendingChoice {
                                        choice_seq,
                                        choice_indices,
                                        action_count,
                                        client_state_hash,
                                        client_debug_info: debug_info,
                                        spell_ability,
                                    });
                                }
                            }

                            Ok(ClientMessage::Ping { timestamp_ms }) => {
                                conn.send(&ServerMessage::Pong { timestamp_ms }).await?;
                            }

                            Ok(ClientMessage::Disconnect) => {
                                log::info!("Handler P{}: Client disconnected gracefully", conn.player_id);
                                conn.game_tx.send(HandlerToGame::ClientDisconnected).await?;
                                break;
                            }

                            Ok(ClientMessage::Authenticate { .. }) => {
                                conn.send(&ServerMessage::Error {
                                    message: "Already authenticated".to_string(),
                                    fatal: false,
                                }).await?;
                            }

                            Err(e) => {
                                log::error!("Handler P{}: Failed to parse: {}", conn.player_id, e);
                                conn.send(&ServerMessage::Error {
                                    message: format!("Invalid message: {}", e),
                                    fatal: false,
                                }).await?;
                            }
                        }
                    }

                    Some(Ok(Message::Close(_))) => {
                        log::info!("Handler P{}: WebSocket closed", conn.player_id);
                        let _ = conn.game_tx.send(HandlerToGame::ClientDisconnected).await;
                        break;
                    }

                    Some(Ok(_)) => {
                        // Ignore binary/ping/pong
                    }

                    Some(Err(e)) => {
                        log::error!("Handler P{}: WebSocket error: {}", conn.player_id, e);
                        let _ = conn.game_tx.send(HandlerToGame::ClientError(e.to_string())).await;
                        break;
                    }

                    None => {
                        log::debug!("Handler P{}: WebSocket stream ended", conn.player_id);
                        let _ = conn.game_tx.send(HandlerToGame::ClientDisconnected).await;
                        break;
                    }
                }
            }
        }
    }

    log::debug!("Handler P{}: Exiting", conn.player_id);
    Ok(())
}

/// Run the game loop with NetworkControllers
fn run_game_loop(
    game: Arc<Mutex<GameState>>,
    mut p1_controller: NetworkController,
    mut p2_controller: NetworkController,
    tag_gamelogs: bool,
    verbosity: crate::game::VerbosityLevel,
    no_color_logs: bool,
) -> Result<GameResult> {
    // Take ownership of game for the game loop
    let mut game = {
        // We need to get the game out of the mutex for the game loop
        // This is safe because the WebSocket handlers only read game state
        let guard = game.blocking_lock();
        guard.clone()
    };

    // Configure the game logger with server settings
    game.logger.set_verbosity(verbosity);
    game.logger.set_tag_gamelogs(tag_gamelogs);
    // Disable colors if --no-color-logs flag or NO_COLOR env var is set
    let color_enabled = !no_color_logs && std::env::var("NO_COLOR").is_err();
    game.logger.set_color_enabled(color_enabled);

    log::debug!(
        "Server GameLoop: undo_log.len() = {} (should be 0 for synchronized mode)",
        game.undo_log.len()
    );

    // Create game loop with skip_opening_hands() to match client behavior.
    // Both server and client will draw opening hands during GameLoop::setup_game(),
    // ensuring identical undo_log entries and synchronized action_counts.
    //
    // All reveals are sent synchronously via ChoiceRequest messages,
    // ensuring strict ordering and preventing desync issues.
    let mut game_loop = GameLoop::new(&mut game).skip_opening_hands();

    // Run until game ends
    let result = game_loop.run_game(&mut p1_controller, &mut p2_controller);

    match result {
        Ok(game_result) => Ok(game_result),
        Err(e) => Err(anyhow!("Game loop error: {}", e)),
    }
}

/// Peek at opening hand for a player and return CardReveals WITHOUT drawing
///
/// This looks at the top 7 cards of the library without actually drawing them.
/// The drawing will happen later when the GameLoop runs with skip_opening_hands().
/// This ensures both server and client GameLoops start with identical empty undo_logs.
fn peek_opening_hand(game: &GameState, player_id: PlayerId) -> Result<Vec<CardReveal>> {
    let mut hand = Vec::new();

    // Get the library for this player
    let zones = game
        .get_player_zones(player_id)
        .ok_or_else(|| anyhow!("Player {:?} not found", player_id))?;

    // Peek at top 7 cards (library.cards stores bottom-to-top, so we take from the end)
    let lib_cards = &zones.library.cards;
    let start_idx = lib_cards.len().saturating_sub(7);

    for &card_id in lib_cards[start_idx..].iter().rev() {
        // Get card info for reveal
        if let Ok(card) = game.cards.get(card_id) {
            let card_def = game.card_definitions.get(&card.name).cloned();
            hand.push(CardReveal {
                card_id,
                name: card.name.to_string(),
                card_def,
            });
        }
    }

    Ok(hand)
}

/// Compute network-safe state hash
fn compute_network_hash(game: &GameState) -> u64 {
    // FIXME-UNFINISHED: Use proper network hash from state_hash::compute_hash with HashMode::Network
    // Currently only hashes turn number and life totals, missing battlefield state etc.
    let mut hash: u64 = u64::from(game.turn.turn_number);
    for player in &game.players {
        hash = hash.wrapping_mul(31).wrapping_add(player.life as u64);
    }
    hash
}

// ═══════════════════════════════════════════════════════════════════════════
// REVEAL HELPERS
// ═══════════════════════════════════════════════════════════════════════════

/// Build a CardReveal from a CardRevealInfo by looking up card details in game state
fn build_card_reveal(game: &GameState, info: &CardRevealInfo) -> Option<CardReveal> {
    let card = game.cards.try_get(info.card_id)?;

    // Look up CardDefinition from game.card_definitions
    let card_def = game.card_definitions.get(&card.name).cloned();

    Some(CardReveal {
        card_id: info.card_id,
        name: card.name.to_string(),
        card_def,
    })
}

/// Convert a zone to the appropriate RevealReason
fn zone_to_reveal_reason(zone: Zone) -> RevealReason {
    match zone {
        Zone::Hand => RevealReason::Draw,
        Zone::Battlefield | Zone::Stack => RevealReason::Played,
        Zone::Graveyard => RevealReason::Effect, // Mill or other effects
        Zone::Exile => RevealReason::Effect,
        Zone::Library => RevealReason::Searched, // Returned to library (unusual)
        Zone::Command => RevealReason::Effect,
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// HELPER FUNCTIONS
// ═══════════════════════════════════════════════════════════════════════════

/// Send a message over WebSocket
async fn send_message(ws: &mut WebSocketStream<TcpStream>, msg: &ServerMessage) -> Result<()> {
    let json = serde_json::to_string(msg)?;
    ws.send(Message::Text(json.into())).await?;
    Ok(())
}

/// Send an error message
async fn send_error(ws: &mut WebSocketStream<TcpStream>, message: &str, fatal: bool) -> Result<()> {
    send_message(
        ws,
        &ServerMessage::Error {
            message: message.to_string(),
            fatal,
        },
    )
    .await
}

// ═══════════════════════════════════════════════════════════════════════════
// TESTS
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_server_config_default() {
        let config = ServerConfig::default();
        assert_eq!(config.port, DEFAULT_PORT);
        assert_eq!(config.starting_life, 20);
        assert!(!config.deck_visibility);
    }

    #[test]
    fn test_deck_submission_sizes() {
        let deck = DeckSubmission::new(
            vec![("Lightning Bolt".to_string(), 4), ("Mountain".to_string(), 56)],
            vec![("Pyroclasm".to_string(), 2)],
        );
        assert_eq!(deck.main_deck_size(), 60);
        assert_eq!(deck.sideboard_size(), 2);
    }

    #[test]
    fn test_submission_to_decklist() {
        let submission = DeckSubmission::new(
            vec![("Lightning Bolt".to_string(), 4), ("Mountain".to_string(), 20)],
            vec![("Shock".to_string(), 2)],
        );

        let decklist = submission_to_decklist(&submission);

        assert_eq!(decklist.main_deck.len(), 2);
        assert_eq!(decklist.main_deck[0].card_name, "Lightning Bolt");
        assert_eq!(decklist.main_deck[0].count, 4);
        assert_eq!(decklist.sideboard.len(), 1);
        assert_eq!(decklist.sideboard[0].card_name, "Shock");
    }
}
