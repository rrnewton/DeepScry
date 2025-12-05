//! WebSocket game server for multiplayer MTG
//!
//! Implements a server that:
//! - Accepts client connections over WebSocket
//! - Handles authentication and deck submission
//! - Matches players (first waits for second)
//! - Runs authoritative game state with NetworkControllers
//! - Broadcasts card reveals and opponent choices

use crate::core::PlayerId;
use crate::game::{GameLoop, GameState};
use crate::loader::{AsyncCardDatabase, DeckEntry, DeckList, GameInitializer};
use crate::network::protocol::{CardReveal, ClientMessage, DeckListInfo, DeckSubmission, ServerMessage};
use crate::network::{ChoiceRequest, ChoiceResponse, NetworkController, DEFAULT_PORT};
use anyhow::{anyhow, Result};
use futures_util::{SinkExt, StreamExt};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc as tokio_mpsc, Mutex};
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
struct PlayerConnection {
    /// Player ID in the game
    player_id: PlayerId,
    /// WebSocket sender
    ws_tx: futures_util::stream::SplitSink<WebSocketStream<TcpStream>, Message>,
    /// Channel to receive choice requests (bridged from sync NetworkController channel)
    request_rx: tokio_mpsc::Receiver<ChoiceRequest>,
    /// Channel to send choice responses to NetworkController
    response_tx: std::sync::mpsc::Sender<ChoiceResponse>,
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

    // Create PlayerConnections with tokio receivers
    let mut p1_conn = PlayerConnection {
        player_id: PlayerId::new(0),
        ws_tx: p1_ws_tx,
        request_rx: p1_async_request_rx,
        response_tx: p1_response_tx,
    };
    let mut p2_conn = PlayerConnection {
        player_id: PlayerId::new(1),
        ws_tx: p2_ws_tx,
        request_rx: p2_async_request_rx,
        response_tx: p2_response_tx,
    };

    // Convert deck submissions to DeckList format
    let p1_decklist = submission_to_decklist(&p1.deck);
    let p2_decklist = submission_to_decklist(&p2.deck);

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
    let seed = rand::random::<u64>();
    game.seed_rng(seed);
    log::info!("Game {}: Using seed {}", game_id, seed);

    let p1_id = game.players[0].id;
    let p2_id = game.players[1].id;
    game.shuffle_library(p1_id);
    game.shuffle_library(p2_id);

    // Draw opening hands
    let p1_hand = draw_opening_hand(&mut game, p1_id)?;
    let p2_hand = draw_opening_hand(&mut game, p2_id)?;

    // Compute initial state hash
    let initial_hash = compute_network_hash(&game);

    // Build deck list info if visibility is enabled
    let p1_deck_info = if config.deck_visibility {
        Some(DeckListInfo::from_submission(&p1.deck))
    } else {
        None
    };
    let p2_deck_info = if config.deck_visibility {
        Some(DeckListInfo::from_submission(&p2.deck))
    } else {
        None
    };

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
        })
        .await?;

    log::info!("Game {}: Sent GameStarted to both players", game_id);

    // Create NetworkControllers
    let p1_controller = NetworkController::new(p1_id, p1_request_tx, p1_response_rx);
    let p2_controller = NetworkController::new(p2_id, p2_request_tx, p2_response_rx);

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
    let game_loop_handle = tokio::task::spawn_blocking(move || run_game_loop(game_clone, p1_controller, p2_controller));

    // Wait for game to complete
    let result = game_loop_handle.await?;

    // Cancel all handlers
    p1_handler.abort();
    p2_handler.abort();
    p1_bridge.abort();
    p2_bridge.abort();

    match result {
        Ok(winner) => {
            log::info!("Game {}: Completed, winner = {:?}", game_id, winner);
        }
        Err(e) => {
            log::error!("Game {}: Error - {}", game_id, e);
        }
    }

    Ok(())
}

/// Handle WebSocket messages for a player
async fn handle_player_websocket(
    mut conn: PlayerConnection,
    mut ws_rx: futures_util::stream::SplitStream<WebSocketStream<TcpStream>>,
    _game: Arc<Mutex<GameState>>,
    _opponent_id: PlayerId,
) -> Result<()> {
    loop {
        tokio::select! {
            // Check for choice requests from NetworkController (via bridge)
            request = conn.request_rx.recv() => {
                match request {
                    Some(choice_request) => {
                        // Send ChoiceRequest to client
                        conn.send(&ServerMessage::ChoiceRequest {
                            choice_seq: choice_request.choice_seq,
                            choice_type: choice_request.choice_type,
                            options: choice_request.options,
                            state_hash: choice_request.state_hash,
                            context: None,
                        }).await?;
                    }
                    None => {
                        // Channel closed - game ended
                        break;
                    }
                }
            }

            // Check for WebSocket messages from client
            msg = ws_rx.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        match serde_json::from_str::<ClientMessage>(&text) {
                            Ok(ClientMessage::SubmitChoice { choice_seq, choice_index }) => {
                                // Send response to NetworkController
                                let response = ChoiceResponse { choice_seq, choice_index };
                                if conn.response_tx.send(response).is_err() {
                                    log::error!("Failed to send choice response");
                                    break;
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
        }
    }

    Ok(())
}

/// Run the game loop with NetworkControllers
fn run_game_loop(
    game: Arc<Mutex<GameState>>,
    mut p1_controller: NetworkController,
    mut p2_controller: NetworkController,
) -> Result<Option<PlayerId>> {
    // Take ownership of game for the game loop
    let mut game = {
        // We need to get the game out of the mutex for the game loop
        // This is safe because the WebSocket handlers only read game state
        let guard = game.blocking_lock();
        guard.clone()
    };

    // Create game loop
    let mut game_loop = GameLoop::new(&mut game);

    // Run until game ends
    let result = game_loop.run_game(&mut p1_controller, &mut p2_controller);

    match result {
        Ok(game_result) => Ok(game_result.winner),
        Err(e) => Err(anyhow!("Game loop error: {}", e)),
    }
}

/// Draw opening hand for a player and return CardReveals
fn draw_opening_hand(game: &mut GameState, player_id: PlayerId) -> Result<Vec<CardReveal>> {
    let mut hand = Vec::new();

    // Draw 7 cards
    for _ in 0..7 {
        if let Ok(Some(card_id)) = game.draw_card(player_id) {
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
    }

    Ok(hand)
}

/// Compute network-safe state hash
fn compute_network_hash(game: &GameState) -> u64 {
    // Use simplified hash for now - just turn number and life totals
    // TODO: Use proper network hash that excludes hidden info
    let mut hash: u64 = game.turn.turn_number as u64;
    for player in &game.players {
        hash = hash.wrapping_mul(31).wrapping_add(player.life as u64);
    }
    hash
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
