// TODO(mtg-0et0f): Remove this file-level allow once wildcards are fixed
#![allow(clippy::wildcard_enum_match_arm)]
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
use crate::network::{
    CardRevealInfo, ChoiceRequest, ChoiceResponse, ChosenAbilityInfo, NetworkController, DEFAULT_PORT,
};
use crate::undo::GameAction;
use crate::zones::Zone;
use anyhow::{anyhow, Result};
use futures_util::{SinkExt, StreamExt};
use std::collections::HashMap;
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
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// WAITING PLAYER
// ═══════════════════════════════════════════════════════════════════════════

/// A player waiting for an opponent
struct WaitingPlayer {
    /// Player's display name
    name: String,
    /// Submitted deck
    deck: DeckSubmission,
    /// WebSocket connection
    ws_stream: WebSocketStream<TcpStream>,
}

// ═══════════════════════════════════════════════════════════════════════════
// ACTIVE GAME
// ═══════════════════════════════════════════════════════════════════════════

/// An active game in progress
struct ActiveGame {
    /// Unique game ID
    #[allow(dead_code)]
    game_id: u64,
    /// Join handle for the game task
    #[allow(dead_code)]
    task_handle: tokio::task::JoinHandle<()>,
}

// ═══════════════════════════════════════════════════════════════════════════
// PLAYER CONNECTION
// ═══════════════════════════════════════════════════════════════════════════

/// Connection handler for a single player
/// Information about how a game ended, sent to WebSocket handlers
#[derive(Clone)]
struct GameEndInfo {
    winner: Option<PlayerId>,
    reason: GameEndReason,
    final_hash: u64,
    action_count: u64,
}

/// Info about an opponent's choice, broadcast to the other player
#[derive(Clone)]
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
#[derive(Clone)]
struct RevealBroadcast {
    /// Owner of the card
    owner: PlayerId,
    /// Card ID
    card_id: CardId,
    /// Zone the card moved to
    to_zone: Zone,
}

/// A SubmitChoice that arrived before the corresponding ChoiceRequest.
/// This can happen in synchronized GameLoop mode due to timing differences.
/// We store it and process it once the ChoiceRequest arrives.
#[derive(Debug)]
struct PendingChoice {
    /// Choice sequence number from the client
    choice_seq: u32,
    /// Indices of the chosen options (multiple for attackers/blockers/discard)
    choice_indices: Vec<usize>,
    /// Action count the client claims (for validation)
    action_count: u64,
}

struct PlayerConnection {
    /// Player ID in the game
    player_id: PlayerId,
    /// WebSocket sender
    ws_tx: futures_util::stream::SplitSink<WebSocketStream<TcpStream>, Message>,
    /// Channel to receive choice requests (bridged from sync NetworkController channel)
    request_rx: tokio_mpsc::Receiver<ChoiceRequest>,
    /// Channel to send choice responses to NetworkController
    response_tx: std::sync::mpsc::Sender<ChoiceResponse>,
    /// Channel to receive game end notification
    game_end_rx: oneshot::Receiver<GameEndInfo>,
    /// Channel to send our choices to opponent (for run_game mode)
    opponent_choice_tx: tokio_mpsc::Sender<OpponentChoiceInfo>,
    /// Channel to receive opponent's choices (for run_game mode)
    opponent_choice_rx: tokio_mpsc::Receiver<OpponentChoiceInfo>,
    /// Channel to broadcast reveals to this player (receives from opponent's ChoiceRequest)
    reveal_rx: tokio_mpsc::Receiver<Vec<RevealBroadcast>>,
    /// Channel to send reveals to the opponent (when we receive ChoiceRequest with reveals)
    opponent_reveal_tx: tokio_mpsc::Sender<Vec<RevealBroadcast>>,
    /// Channel to receive immediate reveals from the game thread (after automatic actions like draws)
    immediate_reveal_rx: tokio_mpsc::Receiver<Vec<RevealBroadcast>>,
    /// Channel to receive chosen abilities from NetworkController (for Priority choices)
    /// This allows the WebSocket handler to include the actual ability in OpponentChoice
    ability_rx: tokio_mpsc::Receiver<ChosenAbilityInfo>,
    /// Current choice type being requested (for broadcasting)
    current_choice_type: Option<ChoiceType>,
    /// Expected action_count from the last ChoiceRequest sent to this client.
    /// Used for sync validation when the client responds with SubmitChoice.
    /// This is the authoritative source - NOT the shared game state (which is stale).
    expected_action_count: Option<u64>,
    /// Expected state_hash from the last ChoiceRequest sent to this client.
    /// Used for sync validation in network debug mode.
    expected_state_hash: Option<u64>,
    /// Server's DebugSyncInfo from the last ChoiceRequest sent to this client.
    /// Used for detailed diff logging when hashes mismatch.
    expected_debug_info: Option<crate::network::DebugSyncInfo>,
    /// A choice that arrived before the corresponding ChoiceRequest.
    /// In synchronized GameLoop mode, the client may submit a choice before
    /// the server's NetworkController sends its ChoiceRequest due to timing.
    /// We store it here and process it once the ChoiceRequest arrives.
    pending_choice: Option<PendingChoice>,
    /// The index into the undo_log where we last sent reveals.
    /// Initialized to the number of opening hand draw actions (14 for 7+7).
    /// This prevents re-sending opening hand reveals that were already sent
    /// during the GameStarted handshake phase.
    last_reveal_index: usize,
    /// Network debug mode - when enabled, validate client state hashes
    network_debug: bool,
}

impl PlayerConnection {
    /// Send a server message to this player
    async fn send(&mut self, msg: &ServerMessage) -> Result<()> {
        let json = serde_json::to_string(msg)?;
        self.ws_tx.send(Message::Text(json.into())).await?;
        Ok(())
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// GAME SERVER
// ═══════════════════════════════════════════════════════════════════════════

/// MTG game server
pub struct GameServer {
    /// Server configuration
    config: ServerConfig,
    /// Currently waiting player (first to connect)
    waiting_player: Option<WaitingPlayer>,
    /// Active games by ID
    games: HashMap<u64, ActiveGame>,
    /// Next game ID
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
            games: HashMap::new(),
            next_game_id: 1,
            card_db: None,
        }
    }

    /// Run the server (blocking)
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

        // Accept connections
        loop {
            match listener.accept().await {
                Ok((stream, addr)) => {
                    log::info!("New connection from {}", addr);
                    if let Err(e) = self.handle_connection(stream).await {
                        log::error!("Connection error: {}", e);
                    }
                }
                Err(e) => {
                    log::error!("Accept error: {}", e);
                }
            }
        }
    }

    /// Handle a new WebSocket connection
    async fn handle_connection(&mut self, stream: TcpStream) -> Result<()> {
        let ws_stream = accept_async(stream).await?;
        let (ws_tx, mut ws_rx) = ws_stream.split();

        // Wait for authentication message
        let auth_msg = match ws_rx.next().await {
            Some(Ok(Message::Text(text))) => serde_json::from_str::<ClientMessage>(&text)?,
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
                Ok(())
            }
        }
    }

    /// Handle authentication attempt
    async fn handle_auth(
        &mut self,
        mut ws_stream: WebSocketStream<TcpStream>,
        password: String,
        player_name: String,
        deck: DeckSubmission,
    ) -> Result<()> {
        // Check password
        if !self.config.password.is_empty() && password != self.config.password {
            send_message(
                &mut ws_stream,
                &ServerMessage::AuthResult {
                    success: false,
                    error: Some("Invalid password".to_string()),
                    your_player_id: None,
                },
            )
            .await?;
            return Ok(());
        }

        // Validate deck
        if deck.main_deck_size() < 40 {
            send_message(
                &mut ws_stream,
                &ServerMessage::AuthResult {
                    success: false,
                    error: Some(format!("Deck too small: {} cards (minimum 40)", deck.main_deck_size())),
                    your_player_id: None,
                },
            )
            .await?;
            return Ok(());
        }

        log::info!(
            "Player '{}' authenticated with {} card deck",
            player_name,
            deck.main_deck_size()
        );

        // Check if we have a waiting player
        if let Some(waiting) = self.waiting_player.take() {
            // Start game with both players
            log::info!("Starting game: {} vs {}", waiting.name, player_name);

            // Send auth success to player 2
            send_message(
                &mut ws_stream,
                &ServerMessage::AuthResult {
                    success: true,
                    error: None,
                    your_player_id: Some(PlayerId::new(1)),
                },
            )
            .await?;

            // Start the game
            self.start_game(
                waiting,
                WaitingPlayer {
                    name: player_name,
                    deck,
                    ws_stream,
                },
            )
            .await?;
        } else {
            // First player - send auth success and wait
            send_message(
                &mut ws_stream,
                &ServerMessage::AuthResult {
                    success: true,
                    error: None,
                    your_player_id: Some(PlayerId::new(0)),
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
        }

        Ok(())
    }

    /// Start a game between two players
    async fn start_game(&mut self, p1: WaitingPlayer, p2: WaitingPlayer) -> Result<()> {
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

        self.games.insert(game_id, ActiveGame { game_id, task_handle });

        Ok(())
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
async fn run_game(
    game_id: u64,
    p1: WaitingPlayer,
    p2: WaitingPlayer,
    card_db: Arc<AsyncCardDatabase>,
    config: ServerConfig,
) -> Result<()> {
    log::info!("Game {}: Initializing {} vs {}", game_id, p1.name, p2.name);

    // Create sync channels for NetworkControllers (used by game loop in blocking thread)
    let (p1_request_tx, p1_sync_request_rx) = std::sync::mpsc::channel::<ChoiceRequest>();
    let (p1_response_tx, p1_response_rx) = std::sync::mpsc::channel::<ChoiceResponse>();
    let (p2_request_tx, p2_sync_request_rx) = std::sync::mpsc::channel::<ChoiceRequest>();
    let (p2_response_tx, p2_response_rx) = std::sync::mpsc::channel::<ChoiceResponse>();

    // Create tokio channels for bridging to async WebSocket handlers
    let (p1_async_request_tx, p1_async_request_rx) = tokio_mpsc::channel::<ChoiceRequest>(16);
    let (p2_async_request_tx, p2_async_request_rx) = tokio_mpsc::channel::<ChoiceRequest>(16);

    // Spawn bridge tasks that forward from sync to async channels
    // Each bridge runs a blocking loop in a spawn_blocking task
    let p1_bridge = tokio::task::spawn_blocking(move || {
        while let Ok(request) = p1_sync_request_rx.recv() {
            // Use try_send to avoid blocking on the async side
            // If the channel is full or closed, just break
            if p1_async_request_tx.blocking_send(request).is_err() {
                break;
            }
        }
    });

    let p2_bridge = tokio::task::spawn_blocking(move || {
        while let Ok(request) = p2_sync_request_rx.recv() {
            if p2_async_request_tx.blocking_send(request).is_err() {
                break;
            }
        }
    });

    // Split WebSocket streams
    let (p1_ws_tx, p1_ws_rx) = p1.ws_stream.split();
    let (p2_ws_tx, p2_ws_rx) = p2.ws_stream.split();

    // Create oneshot channels to notify handlers when game ends
    let (p1_game_end_tx, p1_game_end_rx) = oneshot::channel::<GameEndInfo>();
    let (p2_game_end_tx, p2_game_end_rx) = oneshot::channel::<GameEndInfo>();

    // Create cross-player channels for opponent choice broadcasting (for run_game mode)
    // When P1 makes a choice, it gets sent to P2. When P2 makes a choice, it gets sent to P1.
    // Channel naming: (sender_tx, receiver_rx) - sender writes, receiver reads
    let (p1_choice_tx, p1_choice_rx) = tokio_mpsc::channel::<OpponentChoiceInfo>(16); // P1's choices
    let (p2_choice_tx, p2_choice_rx) = tokio_mpsc::channel::<OpponentChoiceInfo>(16); // P2's choices

    // Create cross-player channels for reveal broadcasting
    // When P1 receives a ChoiceRequest with reveals, those reveals are sent to P2 immediately
    // (and vice versa). This ensures both clients have reveals before they need them.
    let (p1_reveal_tx, p1_reveal_rx) = tokio_mpsc::channel::<Vec<RevealBroadcast>>(32);
    let (p2_reveal_tx, p2_reveal_rx) = tokio_mpsc::channel::<Vec<RevealBroadcast>>(32);

    // Create channels for immediate reveals from the game thread (after automatic actions)
    // These are sync channels (for the blocking game thread) bridged to tokio channels
    let (p1_immed_sync_tx, p1_immed_sync_rx) = std::sync::mpsc::channel::<Vec<RevealBroadcast>>();
    let (p2_immed_sync_tx, p2_immed_sync_rx) = std::sync::mpsc::channel::<Vec<RevealBroadcast>>();
    let (p1_immed_async_tx, p1_immed_async_rx) = tokio_mpsc::channel::<Vec<RevealBroadcast>>(32);
    let (p2_immed_async_tx, p2_immed_async_rx) = tokio_mpsc::channel::<Vec<RevealBroadcast>>(32);

    // Bridge immediate reveal channels from sync to async
    let p1_immed_bridge = tokio::task::spawn_blocking(move || {
        while let Ok(reveals) = p1_immed_sync_rx.recv() {
            if p1_immed_async_tx.blocking_send(reveals).is_err() {
                break;
            }
        }
    });
    let p2_immed_bridge = tokio::task::spawn_blocking(move || {
        while let Ok(reveals) = p2_immed_sync_rx.recv() {
            if p2_immed_async_tx.blocking_send(reveals).is_err() {
                break;
            }
        }
    });
    let _p1_immed_bridge = p1_immed_bridge; // Keep the handle alive
    let _p2_immed_bridge = p2_immed_bridge;

    // Create channels for chosen abilities from NetworkController (for OpponentChoice)
    // These sync channels are used by NetworkController to send the ability after a priority choice
    let (p1_ability_sync_tx, p1_ability_sync_rx) = std::sync::mpsc::channel::<ChosenAbilityInfo>();
    let (p2_ability_sync_tx, p2_ability_sync_rx) = std::sync::mpsc::channel::<ChosenAbilityInfo>();
    let (p1_ability_async_tx, p1_ability_async_rx) = tokio_mpsc::channel::<ChosenAbilityInfo>(16);
    let (p2_ability_async_tx, p2_ability_async_rx) = tokio_mpsc::channel::<ChosenAbilityInfo>(16);

    // Bridge ability channels from sync to async
    let p1_ability_bridge = tokio::task::spawn_blocking(move || {
        while let Ok(ability_info) = p1_ability_sync_rx.recv() {
            if p1_ability_async_tx.blocking_send(ability_info).is_err() {
                break;
            }
        }
    });
    let p2_ability_bridge = tokio::task::spawn_blocking(move || {
        while let Ok(ability_info) = p2_ability_sync_rx.recv() {
            if p2_ability_async_tx.blocking_send(ability_info).is_err() {
                break;
            }
        }
    });
    let _p1_ability_bridge = p1_ability_bridge; // Keep the handle alive
    let _p2_ability_bridge = p2_ability_bridge;

    // Create PlayerConnections with tokio receivers
    // Note: last_reveal_index will be set after we determine the opening hand sizes
    let mut p1_conn = PlayerConnection {
        player_id: PlayerId::new(0),
        ws_tx: p1_ws_tx,
        request_rx: p1_async_request_rx,
        response_tx: p1_response_tx,
        game_end_rx: p1_game_end_rx,
        opponent_choice_tx: p1_choice_tx,       // P1 sends their choices on this channel
        opponent_choice_rx: p2_choice_rx,       // P1 receives P2's choices from this channel
        reveal_rx: p2_reveal_rx,                // P1 receives reveals from P2's ChoiceRequest
        opponent_reveal_tx: p1_reveal_tx,       // P1 sends reveals to P2 (when P1 gets ChoiceRequest)
        immediate_reveal_rx: p1_immed_async_rx, // P1 receives immediate reveals from game thread
        ability_rx: p1_ability_async_rx,        // P1 receives ability info from NetworkController
        current_choice_type: None,
        expected_action_count: None,
        expected_state_hash: None,
        expected_debug_info: None,
        pending_choice: None,
        last_reveal_index: 0, // Will be set after opening hands are determined
        network_debug: config.network_debug,
    };
    let mut p2_conn = PlayerConnection {
        player_id: PlayerId::new(1),
        ws_tx: p2_ws_tx,
        request_rx: p2_async_request_rx,
        response_tx: p2_response_tx,
        game_end_rx: p2_game_end_rx,
        opponent_choice_tx: p2_choice_tx,       // P2 sends their choices on this channel
        opponent_choice_rx: p1_choice_rx,       // P2 receives P1's choices from this channel
        reveal_rx: p1_reveal_rx,                // P2 receives reveals from P1's ChoiceRequest
        opponent_reveal_tx: p2_reveal_tx,       // P2 sends reveals to P1 (when P2 gets ChoiceRequest)
        immediate_reveal_rx: p2_immed_async_rx, // P2 receives immediate reveals from game thread
        ability_rx: p2_ability_async_rx,        // P2 receives ability info from NetworkController
        current_choice_type: None,
        expected_action_count: None,
        expected_state_hash: None,
        expected_debug_info: None,
        pending_choice: None,
        last_reveal_index: 0, // Will be set after opening hands are determined
        network_debug: config.network_debug,
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

    // Create game state using GameInitializer
    let initializer = GameInitializer::new(&card_db);
    let mut game = initializer
        .init_game(
            p1.name.clone(),
            &p1_decklist,
            p2.name.clone(),
            &p2_decklist,
            config.starting_life,
        )
        .await?;

    // Seed RNG and shuffle libraries
    let seed = config.seed.unwrap_or_else(rand::random::<u64>);
    game.seed_rng(seed);
    log::info!("Game {}: Using seed {}", game_id, seed);

    let p1_id = game.players[0].id;
    let p2_id = game.players[1].id;
    game.shuffle_library(p1_id);
    game.shuffle_library(p2_id);

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

    // Send GameStarted to both players
    let p1_lib_size = game.player_zones[0].1.library.len();
    let p2_lib_size = game.player_zones[1].1.library.len();

    p1_conn
        .send(&ServerMessage::GameStarted {
            your_player_id: p1_id,
            opponent_name: p2.name.clone(),
            opening_hand: p1_hand.clone(),
            opponent_hand_count: p2_hand.len(),
            library_size: p1_lib_size,
            opponent_library_size: p2_lib_size,
            opponent_decklist: p2_deck_info.clone(),
            starting_life: config.starting_life,
            initial_state_hash: initial_hash,
            network_debug: config.network_debug,
        })
        .await?;

    p2_conn
        .send(&ServerMessage::GameStarted {
            your_player_id: p2_id,
            opponent_name: p1.name.clone(),
            opening_hand: p2_hand.clone(),
            opponent_hand_count: p1_hand.len(),
            library_size: p2_lib_size,
            opponent_library_size: p1_lib_size,
            opponent_decklist: p1_deck_info.clone(),
            starting_life: config.starting_life,
            initial_state_hash: initial_hash,
            network_debug: config.network_debug,
        })
        .await?;

    log::info!("Game {}: Sent GameStarted to both players", game_id);

    // Send CardRevealed messages for opening hands (for synchronized GameLoop mode)
    // Each player needs to know BOTH players' opening hand card IDs so they can
    // queue them in their shadow state before the local GameLoop draws them.

    // P1 needs reveals for: own hand (p1_hand) + opponent's hand (p2_hand)
    for card in &p1_hand {
        p1_conn
            .send(&ServerMessage::CardRevealed {
                owner: p1_id,
                card: card.clone(),
                reason: RevealReason::Draw,
            })
            .await?;
    }
    for card in &p2_hand {
        p1_conn
            .send(&ServerMessage::CardRevealed {
                owner: p2_id,
                card: card.clone(),
                reason: RevealReason::Draw,
            })
            .await?;
    }

    // P2 needs reveals for: own hand (p2_hand) + opponent's hand (p1_hand)
    for card in &p2_hand {
        p2_conn
            .send(&ServerMessage::CardRevealed {
                owner: p2_id,
                card: card.clone(),
                reason: RevealReason::Draw,
            })
            .await?;
    }
    for card in &p1_hand {
        p2_conn
            .send(&ServerMessage::CardRevealed {
                owner: p1_id,
                card: card.clone(),
                reason: RevealReason::Draw,
            })
            .await?;
    }

    log::info!("Game {}: Sent opening hand CardRevealed messages", game_id);

    // Set the baseline reveal index to skip the opening hand draws
    // The undo_log will have p1_hand.len() + p2_hand.len() MoveCard entries
    // after GameLoop draws the opening hands. We've already sent these reveals,
    // so we need to skip them when collecting reveals for the first choice.
    let opening_hand_count = p1_hand.len() + p2_hand.len();
    p1_conn.last_reveal_index = opening_hand_count;
    p2_conn.last_reveal_index = opening_hand_count;
    log::debug!(
        "Game {}: Set last_reveal_index to {} for both players",
        game_id,
        opening_hand_count
    );

    // Create NetworkControllers and set their baseline reveal index
    let mut p1_controller = NetworkController::new(p1_id, p1_request_tx, p1_response_rx);
    let mut p2_controller = NetworkController::new(p2_id, p2_request_tx, p2_response_rx);
    p1_controller.set_last_reveal_index(opening_hand_count);
    p2_controller.set_last_reveal_index(opening_hand_count);
    p1_controller.set_network_debug(config.network_debug);
    p2_controller.set_network_debug(config.network_debug);
    // Wire up ability channels so NetworkControllers can report chosen abilities
    p1_controller.set_ability_tx(p1_ability_sync_tx);
    p2_controller.set_ability_tx(p2_ability_sync_tx);

    // Wrap game state for sharing between tasks
    let game = Arc::new(Mutex::new(game));

    // Spawn WebSocket handlers for each player
    let game_clone = game.clone();
    let p1_handler =
        tokio::spawn(async move { handle_player_websocket(p1_conn, p1_ws_rx, game_clone, PlayerId::new(1)).await });

    let game_clone = game.clone();
    let p2_handler =
        tokio::spawn(async move { handle_player_websocket(p2_conn, p2_ws_rx, game_clone, PlayerId::new(0)).await });

    // Run game loop in blocking thread (uses sync channels)
    let game_clone = game.clone();
    let tag_gamelogs = config.tag_gamelogs;
    let verbosity = config.verbosity;
    let game_loop_handle = tokio::task::spawn_blocking(move || {
        run_game_loop(
            game_clone,
            p1_controller,
            p2_controller,
            tag_gamelogs,
            verbosity,
            p1_immed_sync_tx,
            p2_immed_sync_tx,
        )
    });

    // Wait for game to complete
    let result = game_loop_handle.await?;

    // Get final state hash for the GameEnded message
    // Note: We use final_hash from the stale mutex (for hash continuity) but
    // action_count comes from the GameResult (which has the actual count from the game loop)
    let final_hash = {
        let game_guard = game.lock().await;
        compute_network_hash(&game_guard)
    };

    // Send game end notification to both handlers
    match &result {
        Ok(game_result) => {
            log::info!(
                "Game {}: Completed, winner = {:?}, action_count = {}",
                game_id,
                game_result.winner,
                game_result.action_count
            );

            // Use the end_reason from GameResult, or derive from winner
            let reason = match game_result.end_reason {
                // Use the actual reason if it's meaningful
                GameEndReason::PlayerDeath(_) | GameEndReason::Decking(_) => game_result.end_reason.clone(),
                // Otherwise derive from winner
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

            // Send to both players (ignore errors if handlers already closed)
            let _ = p1_game_end_tx.send(end_info.clone());
            let _ = p2_game_end_tx.send(end_info);
        }
        Err(e) => {
            log::error!("Game {}: Error - {}", game_id, e);
            // Still try to notify players of the error
            // Use 0 for action_count on error (we don't have a valid count)
            let end_info = GameEndInfo {
                winner: None,
                reason: GameEndReason::Draw, // Use draw for errors
                final_hash,
                action_count: 0,
            };
            let _ = p1_game_end_tx.send(end_info.clone());
            let _ = p2_game_end_tx.send(end_info);
        }
    }

    // Wait briefly for handlers to send GameEnded before aborting
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Cancel all handlers
    p1_handler.abort();
    p2_handler.abort();
    p1_bridge.abort();
    p2_bridge.abort();

    Ok(())
}

/// Handle WebSocket messages for a player
async fn handle_player_websocket(
    mut conn: PlayerConnection,
    mut ws_rx: futures_util::stream::SplitStream<WebSocketStream<TcpStream>>,
    game: Arc<Mutex<GameState>>,
    _opponent_id: PlayerId,
) -> Result<()> {
    loop {
        tokio::select! {
            // Check for game end notification
            end_info = &mut conn.game_end_rx => {
                match end_info {
                    Ok(info) => {
                        log::info!("Player {:?}: Sending GameEnded (action_count={})", conn.player_id, info.action_count);
                        conn.send(&ServerMessage::GameEnded {
                            winner: info.winner,
                            reason: info.reason,
                            final_state_hash: info.final_hash,
                            action_count: info.action_count,
                        }).await?;
                    }
                    Err(_) => {
                        // Channel closed without sending - unusual but not fatal
                        log::warn!("Player {:?}: Game end channel closed", conn.player_id);
                    }
                }
                break;
            }

            // Check for choice requests from NetworkController (via bridge)
            request = conn.request_rx.recv() => {
                match request {
                    Some(choice_request) => {
                        // Send CardRevealed messages for each reveal before the choice
                        // AND broadcast them to the opponent so they have reveals before their game loop needs them
                        if !choice_request.reveals.is_empty() {
                            let game_guard = game.lock().await;

                            // Collect reveals to broadcast to opponent
                            let mut reveals_to_broadcast = Vec::new();

                            for reveal_info in &choice_request.reveals {
                                if let Some(card_reveal) = build_card_reveal(&game_guard, reveal_info) {
                                    let reason = zone_to_reveal_reason(reveal_info.to_zone);
                                    conn.send(&ServerMessage::CardRevealed {
                                        owner: reveal_info.owner,
                                        card: card_reveal,
                                        reason,
                                    }).await?;

                                    // Add to broadcast list
                                    reveals_to_broadcast.push(RevealBroadcast {
                                        owner: reveal_info.owner,
                                        card_id: reveal_info.card_id,
                                        to_zone: reveal_info.to_zone,
                                    });
                                }
                            }

                            // Broadcast reveals to the opponent immediately
                            // This ensures the opponent's client has reveals before its game loop needs them
                            if !reveals_to_broadcast.is_empty() {
                                log::debug!(
                                    "Player {:?}: Broadcasting {} reveals to opponent",
                                    conn.player_id, reveals_to_broadcast.len()
                                );
                                if let Err(e) = conn.opponent_reveal_tx.send(reveals_to_broadcast).await {
                                    log::error!("Failed to broadcast reveals: {:?}", e);
                                }
                            }
                        }

                        // Track the choice type for broadcasting to opponent
                        conn.current_choice_type = Some(choice_request.choice_type.clone());

                        // Track the action_count from this ChoiceRequest for validation
                        // when the client responds with SubmitChoice.
                        // This is the authoritative source - NOT the stale game state mutex.
                        conn.expected_action_count = Some(choice_request.action_count);

                        // Track state_hash and debug_info for network debug validation
                        conn.expected_state_hash = Some(choice_request.state_hash);
                        conn.expected_debug_info = choice_request.debug_info.clone();

                        // Check if client already sent a choice (pending_choice)
                        // This happens in synchronized GameLoop mode when client is faster
                        if let Some(pending) = conn.pending_choice.take() {
                            log::debug!(
                                "Player {:?}: Processing pending choice {} (arrived before ChoiceRequest)",
                                conn.player_id, pending.choice_seq
                            );

                            // Validate action_count - FATAL if mismatch
                            if pending.action_count != choice_request.action_count {
                                log::error!(
                                    "FATAL SYNC ERROR: Player {:?} pending choice action_count mismatch! pending={} expected={}",
                                    conn.player_id, pending.action_count, choice_request.action_count
                                );
                                conn.send(&ServerMessage::Error {
                                    message: format!(
                                        "FATAL: action_count mismatch! client={} expected={}",
                                        pending.action_count, choice_request.action_count
                                    ),
                                    fatal: true,
                                }).await?;
                                break;
                            }

                            // Send response to NetworkController
                            let response = ChoiceResponse {
                                choice_seq: pending.choice_seq,
                                choice_indices: pending.choice_indices.clone(),
                            };
                            if conn.response_tx.send(response).is_err() {
                                log::error!("Failed to send choice response for pending choice");
                                break;
                            }

                            // Send acknowledgment back to client
                            conn.send(&ServerMessage::ChoiceAccepted {
                                choice_seq: pending.choice_seq,
                                action_count: choice_request.action_count,
                                timestamp_ms: now_ms(),
                            }).await?;

                            // Broadcast to opponent with proper choice_type
                            let choice_type = conn.current_choice_type.take().unwrap();

                            // For Priority choices, wait for the actual ability from NetworkController
                            let spell_ability = if matches!(choice_type, ChoiceType::Priority { .. }) {
                                match tokio::time::timeout(
                                    std::time::Duration::from_millis(100),
                                    conn.ability_rx.recv()
                                ).await {
                                    Ok(Some(ability_info)) => {
                                        log::debug!(
                                            "Player {:?}: Received ability info for pending choice {}: {:?}",
                                            conn.player_id, ability_info.choice_seq, ability_info.ability
                                        );
                                        ability_info.ability
                                    }
                                    Ok(None) => {
                                        log::warn!("Player {:?}: ability channel closed (pending)", conn.player_id);
                                        None
                                    }
                                    Err(_) => {
                                        log::warn!(
                                            "Player {:?}: timeout waiting for ability info for pending choice {}",
                                            conn.player_id, pending.choice_seq
                                        );
                                        None
                                    }
                                }
                            } else {
                                None
                            };

                            let opponent_info = OpponentChoiceInfo {
                                choice_seq: pending.choice_seq,
                                player: conn.player_id,
                                choice_type,
                                choice_indices: pending.choice_indices.clone(),
                                description: format!("Choice #{}", pending.choice_seq),
                                action_count: choice_request.action_count,
                                timestamp_ms: now_ms(),
                                spell_ability,
                            };
                            log::info!(
                                "Player {:?}: Broadcasting pending choice {} to opponent",
                                conn.player_id, pending.choice_seq
                            );
                            if let Err(e) = conn.opponent_choice_tx.send(opponent_info).await {
                                log::error!("Failed to broadcast pending choice: {:?}", e);
                            }
                        } else {
                            // Normal case: send ChoiceRequest to client
                            conn.send(&ServerMessage::ChoiceRequest {
                                choice_seq: choice_request.choice_seq,
                                for_player: conn.player_id,
                                choice_type: choice_request.choice_type,
                                options: choice_request.options,
                                state_hash: choice_request.state_hash,
                                action_count: choice_request.action_count,
                                timestamp_ms: now_ms(),
                                context: None,
                                debug_info: choice_request.debug_info,
                            }).await?;
                        }
                    }
                    None => {
                        // Channel closed - game ended but we should wait for game_end_rx
                        log::debug!("Player {:?}: Request channel closed", conn.player_id);
                    }
                }
            }

            // Check for WebSocket messages from client
            msg = ws_rx.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        match serde_json::from_str::<ClientMessage>(&text) {
                            Ok(ClientMessage::SubmitChoice { choice_seq, choice_indices, action_count: client_action_count, client_state_hash, debug_info: client_debug_info, .. }) => {
                                // Check if we've sent a ChoiceRequest yet (tracked by expected_action_count)
                                // If not, the client is ahead of us (synchronized GameLoop timing)
                                // and we need to queue this choice for later processing.
                                if let Some(expected) = conn.expected_action_count.take() {
                                    // Normal case: we sent ChoiceRequest and client is responding
                                    log::trace!(
                                        "Player {:?}: received choice {} (client_action_count={}, expected={})",
                                        conn.player_id, choice_seq, client_action_count, expected
                                    );

                                    // Validate action_count - FATAL if mismatch
                                    if client_action_count != expected {
                                        log::error!(
                                            "FATAL SYNC ERROR: Player {:?} action_count mismatch! client={} expected={}",
                                            conn.player_id, client_action_count, expected
                                        );
                                        conn.send(&ServerMessage::Error {
                                            message: format!(
                                                "FATAL: action_count mismatch! client={} expected={}",
                                                client_action_count, expected
                                            ),
                                            fatal: true,
                                        }).await?;
                                        break;
                                    }

                                    // Validate state hash in network debug mode
                                    if conn.network_debug {
                                        if let (Some(client_hash), Some(server_hash)) = (client_state_hash, conn.expected_state_hash.take()) {
                                            if client_hash != server_hash {
                                                // FATAL: State hash mismatch - game state has diverged
                                                log::error!(
                                                    "FATAL SYNC ERROR: Player {:?} state hash mismatch at action_count={}!",
                                                    conn.player_id, expected
                                                );
                                                log::error!(
                                                    "  Server hash: 0x{:016x}",
                                                    server_hash
                                                );
                                                log::error!(
                                                    "  Client hash: 0x{:016x}",
                                                    client_hash
                                                );

                                                // Log detailed diff if debug_info is available
                                                if let Some(server_info) = conn.expected_debug_info.take() {
                                                    log::error!("  Server state: turn={} phase={} active={:?}",
                                                        server_info.turn, server_info.phase, server_info.active_player);
                                                    log::error!("  Server life: P1={} P2={}",
                                                        server_info.life_totals[0], server_info.life_totals[1]);
                                                    log::error!("  Server zones: P1 hand={} lib={} grave={}, P2 hand={} lib={} grave={}",
                                                        server_info.hand_sizes[0], server_info.library_sizes[0], server_info.graveyard_sizes[0],
                                                        server_info.hand_sizes[1], server_info.library_sizes[1], server_info.graveyard_sizes[1]);
                                                    log::error!("  Server battlefield={} stack={}",
                                                        server_info.battlefield_count, server_info.stack_size);
                                                    if !server_info.last_actions.is_empty() {
                                                        log::error!("  Server last actions:");
                                                        for action in &server_info.last_actions {
                                                            log::error!("    {}", action);
                                                        }
                                                    }
                                                }

                                                if let Some(client_info) = client_debug_info {
                                                    log::error!("  Client state: turn={} phase={} active={:?}",
                                                        client_info.turn, client_info.phase, client_info.active_player);
                                                    log::error!("  Client life: P1={} P2={}",
                                                        client_info.life_totals[0], client_info.life_totals[1]);
                                                    log::error!("  Client zones: P1 hand={} lib={} grave={}, P2 hand={} lib={} grave={}",
                                                        client_info.hand_sizes[0], client_info.library_sizes[0], client_info.graveyard_sizes[0],
                                                        client_info.hand_sizes[1], client_info.library_sizes[1], client_info.graveyard_sizes[1]);
                                                    log::error!("  Client battlefield={} stack={}",
                                                        client_info.battlefield_count, client_info.stack_size);
                                                    if !client_info.last_actions.is_empty() {
                                                        log::error!("  Client last actions:");
                                                        for action in &client_info.last_actions {
                                                            log::error!("    {}", action);
                                                        }
                                                    }
                                                }

                                                // Send fatal error to client and terminate game
                                                conn.send(&ServerMessage::Error {
                                                    message: format!(
                                                        "FATAL: State hash mismatch! Server=0x{:016x} Client=0x{:016x} at action_count={}",
                                                        server_hash, client_hash, expected
                                                    ),
                                                    fatal: true,
                                                }).await?;
                                                break;
                                            } else {
                                                log::trace!(
                                                    "Player {:?}: state hash validated 0x{:016x}",
                                                    conn.player_id, client_hash
                                                );
                                            }
                                        }
                                    }

                                    // Send response to NetworkController
                                    let response = ChoiceResponse { choice_seq, choice_indices: choice_indices.clone() };
                                    if conn.response_tx.send(response).is_err() {
                                        log::error!("Failed to send choice response");
                                        break;
                                    }

                                    // Send acknowledgment back to client with SERVER's action_count
                                    conn.send(&ServerMessage::ChoiceAccepted {
                                        choice_seq,
                                        action_count: expected,
                                        timestamp_ms: now_ms(),
                                    }).await?;

                                    // Broadcast to opponent with proper choice_type
                                    // current_choice_type should always be set since we processed ChoiceRequest
                                    let choice_type = conn.current_choice_type.take()
                                        .expect("current_choice_type should be set when expected_action_count is set");

                                    // For Priority choices, wait for the actual ability from NetworkController
                                    // This is needed so the client can execute the opponent's action correctly
                                    let spell_ability = if matches!(choice_type, ChoiceType::Priority { .. }) {
                                        // NetworkController sends ability info immediately after processing the choice
                                        // Use a short timeout to avoid blocking forever on edge cases
                                        match tokio::time::timeout(
                                            std::time::Duration::from_millis(100),
                                            conn.ability_rx.recv()
                                        ).await {
                                            Ok(Some(ability_info)) => {
                                                log::debug!(
                                                    "Player {:?}: Received ability info for choice {}: {:?}",
                                                    conn.player_id, ability_info.choice_seq, ability_info.ability
                                                );
                                                ability_info.ability
                                            }
                                            Ok(None) => {
                                                log::warn!("Player {:?}: ability channel closed", conn.player_id);
                                                None
                                            }
                                            Err(_) => {
                                                log::warn!(
                                                    "Player {:?}: timeout waiting for ability info for choice {}",
                                                    conn.player_id, choice_seq
                                                );
                                                None
                                            }
                                        }
                                    } else {
                                        None
                                    };

                                    let opponent_info = OpponentChoiceInfo {
                                        choice_seq,
                                        player: conn.player_id,
                                        choice_type,
                                        choice_indices: choice_indices.clone(),
                                        description: format!("Choice #{}", choice_seq),
                                        action_count: expected,
                                        timestamp_ms: now_ms(),
                                        spell_ability,
                                    };
                                    log::info!("Player {:?}: Broadcasting choice {} to opponent", conn.player_id, choice_seq);
                                    if let Err(e) = conn.opponent_choice_tx.send(opponent_info).await {
                                        log::error!("Failed to broadcast choice to opponent: {:?}", e);
                                    }
                                } else {
                                    // Client is ahead: queue this choice for processing when ChoiceRequest arrives
                                    log::debug!(
                                        "Player {:?}: Queueing early choice {} (action_count={}) - waiting for ChoiceRequest",
                                        conn.player_id, choice_seq, client_action_count
                                    );
                                    conn.pending_choice = Some(PendingChoice {
                                        choice_seq,
                                        choice_indices,
                                        action_count: client_action_count,
                                    });
                                }
                            }
                            Ok(ClientMessage::Ping { timestamp_ms }) => {
                                conn.send(&ServerMessage::Pong { timestamp_ms }).await?;
                            }
                            Ok(ClientMessage::Disconnect) => {
                                log::info!("Player {:?} disconnected gracefully", conn.player_id);
                                break;
                            }
                            Ok(ClientMessage::Authenticate { .. }) => {
                                conn.send(&ServerMessage::Error {
                                    message: "Already authenticated".to_string(),
                                    fatal: false,
                                }).await?;
                            }
                            Err(e) => {
                                log::error!("Failed to parse client message: {}", e);
                                conn.send(&ServerMessage::Error {
                                    message: format!("Invalid message: {}", e),
                                    fatal: false,
                                }).await?;
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) => {
                        log::info!("Player {:?} closed connection", conn.player_id);
                        break;
                    }
                    Some(Ok(_)) => {
                        // Ignore binary/ping/pong messages
                    }
                    Some(Err(e)) => {
                        log::error!("WebSocket error: {}", e);
                        break;
                    }
                    None => {
                        // Stream ended
                        break;
                    }
                }
            }

            // Check for reveal broadcasts from the other player's ChoiceRequest
            // These are reveals that the opponent received and is broadcasting to us
            // so our client has them before its game loop needs them
            reveals = conn.reveal_rx.recv() => {
                if let Some(reveal_list) = reveals {
                    log::debug!(
                        "Player {:?}: Received {} broadcast reveals from opponent",
                        conn.player_id, reveal_list.len()
                    );
                    let game_guard = game.lock().await;
                    for reveal in reveal_list {
                        // Build CardReveal from the broadcast info
                        if let Some(card) = game_guard.cards.try_get(reveal.card_id) {
                            let types_str: Vec<_> = card.types.iter().map(|t| format!("{:?}", t)).collect();
                            let subtypes_str: Vec<_> = card.subtypes.iter().map(|s| format!("{:?}", s)).collect();
                            let type_line = if subtypes_str.is_empty() {
                                types_str.join(" ")
                            } else {
                                format!("{} - {}", types_str.join(" "), subtypes_str.join(" "))
                            };

                            let card_reveal = CardReveal {
                                card_id: reveal.card_id,
                                name: card.name.to_string(),
                                mana_cost: card.mana_cost.to_string(),
                                type_line,
                                text: card.text.clone(),
                                pt: if card.is_creature() {
                                    match (card.base_power(), card.base_toughness()) {
                                        (Some(p), Some(t)) => Some((p as i32, t as i32)),
                                        _ => None,
                                    }
                                } else {
                                    None
                                },
                            };

                            let reason = zone_to_reveal_reason(reveal.to_zone);
                            conn.send(&ServerMessage::CardRevealed {
                                owner: reveal.owner,
                                card: card_reveal,
                                reason,
                            }).await?;
                        }
                    }
                }
            }

            // Check for immediate reveals from the game thread (after automatic actions like draws)
            // These are pushed immediately by the reveal_pusher hook in the GameLoop
            immed_reveals = conn.immediate_reveal_rx.recv() => {
                if let Some(reveal_list) = immed_reveals {
                    log::debug!(
                        "Player {:?}: Received {} immediate reveals from game thread",
                        conn.player_id, reveal_list.len()
                    );
                    let game_guard = game.lock().await;
                    for reveal in reveal_list {
                        // Build CardReveal from the broadcast info
                        if let Some(card) = game_guard.cards.try_get(reveal.card_id) {
                            let types_str: Vec<_> = card.types.iter().map(|t| format!("{:?}", t)).collect();
                            let subtypes_str: Vec<_> = card.subtypes.iter().map(|s| format!("{:?}", s)).collect();
                            let type_line = if subtypes_str.is_empty() {
                                types_str.join(" ")
                            } else {
                                format!("{} - {}", types_str.join(" "), subtypes_str.join(" "))
                            };

                            let card_reveal = CardReveal {
                                card_id: reveal.card_id,
                                name: card.name.to_string(),
                                mana_cost: card.mana_cost.to_string(),
                                type_line,
                                text: card.text.clone(),
                                pt: if card.is_creature() {
                                    match (card.base_power(), card.base_toughness()) {
                                        (Some(p), Some(t)) => Some((p as i32, t as i32)),
                                        _ => None,
                                    }
                                } else {
                                    None
                                },
                            };

                            let reason = zone_to_reveal_reason(reveal.to_zone);
                            conn.send(&ServerMessage::CardRevealed {
                                owner: reveal.owner,
                                card: card_reveal,
                                reason,
                            }).await?;
                        }
                    }
                    // Update last_reveal_index since we've now sent these
                    conn.last_reveal_index = game_guard.undo_log.len();
                }
            }

            // Check for opponent's choice to forward (for run_game mode)
            opponent_choice = conn.opponent_choice_rx.recv() => {
                if let Some(info) = opponent_choice {
                    log::debug!("Player {:?}: Forwarding opponent choice {}", conn.player_id, info.choice_seq);

                    // If opponent played a card, send CardRevealed so client knows what card it is
                    // This is essential because the client's shadow library doesn't have card identities
                    // for the opponent's hand
                    if let Some(ref ability) = info.spell_ability {
                        // Extract card_id from the ability
                        let card_id = ability.card_id();
                        {
                            let game_guard = game.lock().await;
                            if let Some(card) = game_guard.cards.try_get(card_id) {
                                // Build type line from types and subtypes
                                let types_str: Vec<_> = card.types.iter().map(|t| format!("{:?}", t)).collect();
                                let subtypes_str: Vec<_> = card.subtypes.iter().map(|s| format!("{:?}", s)).collect();
                                let type_line = if subtypes_str.is_empty() {
                                    types_str.join(" ")
                                } else {
                                    format!("{} - {}", types_str.join(" "), subtypes_str.join(" "))
                                };

                                let card_reveal = CardReveal {
                                    card_id,
                                    name: card.name.to_string(),
                                    mana_cost: card.mana_cost.to_string(),
                                    type_line,
                                    text: card.text.clone(),
                                    pt: if card.is_creature() {
                                        match (card.base_power(), card.base_toughness()) {
                                            (Some(p), Some(t)) => Some((p as i32, t as i32)),
                                            _ => None,
                                        }
                                    } else {
                                        None
                                    },
                                };

                                log::debug!(
                                    "Player {:?}: Sending CardRevealed for opponent's played card: {} (id={:?})",
                                    conn.player_id, card.name, card_id
                                );
                                conn.send(&ServerMessage::CardRevealed {
                                    owner: info.player,
                                    card: card_reveal,
                                    reason: RevealReason::Played,
                                }).await?;
                            }
                            // Update last_reveal_index
                            conn.last_reveal_index = game_guard.undo_log.len();
                        }
                    }

                    conn.send(&ServerMessage::OpponentChoice {
                        choice_seq: info.choice_seq,
                        player: info.player,
                        choice_type: info.choice_type,
                        choice_indices: info.choice_indices.clone(),
                        description: info.description,
                        action_count: info.action_count,
                        timestamp_ms: info.timestamp_ms,
                        spell_ability: info.spell_ability,
                        // TODO(mtg-037fw): Populate in debug mode
                        state_hash_after: None,
                        debug_info: None,
                    }).await?;
                }
            }
        }
    }

    Ok(())
}

/// Run the game loop with NetworkControllers
fn run_game_loop(
    game: Arc<Mutex<GameState>>,
    mut p1_controller: NetworkController,
    mut p2_controller: NetworkController,
    tag_gamelogs: bool,
    verbosity: crate::game::VerbosityLevel,
    p1_immed_reveal_tx: std::sync::mpsc::Sender<Vec<RevealBroadcast>>,
    p2_immed_reveal_tx: std::sync::mpsc::Sender<Vec<RevealBroadcast>>,
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

    log::debug!(
        "Server GameLoop: undo_log.len() = {} (should be 0 for synchronized mode)",
        game.undo_log.len()
    );

    // Track where we last collected reveals (to avoid re-sending)
    // Initialize to 14 to skip the opening hand draws that will be done by setup_game.
    // Both players draw 7 cards = 14 MoveCard entries that we don't need to re-reveal
    // (they were already revealed during the GameStarted handshake).
    let opening_hand_draws = 14;
    let last_reveal_index = std::sync::atomic::AtomicUsize::new(opening_hand_draws);

    // Create reveal pusher that sends reveals immediately after automatic actions
    let reveal_pusher = move |game_state: &GameState, _acting_player: PlayerId| {
        let current_len = game_state.undo_log.len();
        let last_idx = last_reveal_index.load(std::sync::atomic::Ordering::Relaxed);

        if current_len <= last_idx {
            return; // No new actions
        }

        // Collect reveals from new undo log entries
        let mut p1_reveals = Vec::new();
        let mut p2_reveals = Vec::new();

        let actions = game_state.undo_log.actions();
        for action in actions.iter().skip(last_idx) {
            if let GameAction::MoveCard {
                card_id,
                from_zone: Zone::Library,
                to_zone,
                owner,
            } = action
            {
                let reveal = RevealBroadcast {
                    owner: *owner,
                    card_id: *card_id,
                    to_zone: *to_zone,
                };

                // BOTH players need ALL reveals to keep their shadow states in sync.
                // Even if P2 draws a card, P1 needs to know about it to update their
                // shadow game state's view of P2's library/hand counts.
                // Cards going to:
                // - Hand: Player needs to know what they drew, opponent needs to track zone count
                // - Public zones (battlefield, graveyard, stack, exile): All players see
                //
                // We send ALL library-to-other-zone moves to BOTH players for synchronization.
                p1_reveals.push(reveal.clone());
                p2_reveals.push(reveal);
            }
        }

        // Update last index
        last_reveal_index.store(current_len, std::sync::atomic::Ordering::Relaxed);

        // Send reveals to handlers
        if !p1_reveals.is_empty() {
            log::debug!("Immediate reveal pusher: {} reveals for P1", p1_reveals.len());
            let _ = p1_immed_reveal_tx.send(p1_reveals);
        }
        if !p2_reveals.is_empty() {
            log::debug!("Immediate reveal pusher: {} reveals for P2", p2_reveals.len());
            let _ = p2_immed_reveal_tx.send(p2_reveals);
        }
    };

    // Create game loop with skip_opening_hands() to match client behavior.
    // Both server and client will draw opening hands during GameLoop::setup_game(),
    // ensuring identical undo_log entries and synchronized action_counts.
    let mut game_loop = GameLoop::new(&mut game)
        .skip_opening_hands()
        .with_reveal_pusher(reveal_pusher);

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
            // Build type line from types and subtypes
            let types_str: Vec<_> = card.types.iter().map(|t| format!("{:?}", t)).collect();
            let subtypes_str: Vec<_> = card.subtypes.iter().map(|s| format!("{:?}", s)).collect();
            let type_line = if subtypes_str.is_empty() {
                types_str.join(" ")
            } else {
                format!("{} - {}", types_str.join(" "), subtypes_str.join(" "))
            };

            hand.push(CardReveal {
                card_id,
                name: card.name.to_string(),
                mana_cost: card.mana_cost.to_string(),
                type_line,
                text: card.text.clone(),
                pt: if card.is_creature() {
                    match (card.base_power(), card.base_toughness()) {
                        (Some(p), Some(t)) => Some((p as i32, t as i32)),
                        _ => None,
                    }
                } else {
                    None
                },
            });
        }
    }

    Ok(hand)
}

/// Compute network-safe state hash
fn compute_network_hash(game: &GameState) -> u64 {
    // FIXME-UNFINISHED: Use proper network hash from state_hash::compute_hash with HashMode::Network
    // Currently only hashes turn number and life totals, missing battlefield state etc.
    let mut hash: u64 = game.turn.turn_number as u64;
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

    // Build type line from types and subtypes
    let types_str: Vec<_> = card.types.iter().map(|t| format!("{:?}", t)).collect();
    let subtypes_str: Vec<_> = card.subtypes.iter().map(|s| format!("{:?}", s)).collect();
    let type_line = if subtypes_str.is_empty() {
        types_str.join(" ")
    } else {
        format!("{} - {}", types_str.join(" "), subtypes_str.join(" "))
    };

    Some(CardReveal {
        card_id: info.card_id,
        name: card.name.to_string(),
        mana_cost: card.mana_cost.to_string(),
        type_line,
        text: card.text.clone(),
        pt: if card.is_creature() {
            match (card.base_power(), card.base_toughness()) {
                (Some(p), Some(t)) => Some((p as i32, t as i32)),
                _ => None,
            }
        } else {
            None
        },
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

/// Collect card reveals that a player should see since their last choice
///
/// Scans the undo log backwards until we find a `ChoicePoint` for this player
/// or until we reach `last_reveal_index` (actions before that were already sent).
/// Returns all `MoveCard` actions from Library that this player should see.
///
/// For the synchronized GameLoop mode, we need to send ALL library movements
/// so the client's shadow state stays synchronized. This includes:
/// - Own cards (e.g., own draws)
/// - Public zone movements (battlefield, graveyard, stack, exile)
/// - Opponent's draws (so client can track opponent's library/hand sizes)
///
/// `last_reveal_index` is the index into the undo_log where we last sent reveals.
/// This is used to skip opening hand reveals that were already sent during handshake.
///
/// NOTE: Currently unused - reveals are now handled by the immediate reveal system.
/// Kept for potential future use with non-draw reveals.
#[allow(dead_code)]
fn collect_reveals_for_player(game: &GameState, player_id: PlayerId, last_reveal_index: usize) -> Vec<CardRevealInfo> {
    use crate::undo::GameAction;

    let actions = game.undo_log.actions();
    let mut reveals = Vec::new();

    // Scan backwards from the end of the log, but stop at last_reveal_index
    // Using enumerate to track the index
    let total_actions = actions.len();
    for (rev_idx, action) in actions.iter().rev().enumerate() {
        // Convert reverse index to forward index
        let forward_idx = total_actions.saturating_sub(rev_idx + 1);

        // Stop if we've reached actions that were already handled
        if forward_idx < last_reveal_index {
            break;
        }

        match action {
            // Stop when we hit this player's last choice
            GameAction::ChoicePoint {
                player_id: choice_player,
                ..
            } if *choice_player == player_id => {
                break;
            }
            // Collect ALL card moves from library (needed for synchronized GameLoop mode)
            GameAction::MoveCard {
                card_id,
                from_zone: Zone::Library,
                to_zone,
                owner,
            } => {
                // For synchronized mode, include ALL library movements
                // - Own cards: player needs to know what they drew
                // - Opponent's cards: client needs card ID for shadow state tracking
                reveals.push(CardRevealInfo {
                    card_id: *card_id,
                    owner: *owner,
                    from_zone: Zone::Library,
                    to_zone: *to_zone,
                });
            }
            _ => {}
        }
    }

    // Reverse to get chronological order
    reveals.reverse();
    reveals
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
