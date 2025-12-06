//! WebSocket client for multiplayer MTG
//!
//! Implements a client that:
//! - Connects to game server over WebSocket
//! - Maintains shadow game state with remote libraries
//! - Processes server messages and syncs game state
//! - Proxies choices to local PlayerController

use crate::core::{CardId, PlayerId};
use crate::game::{print_battlefield_state, print_separator, GameState, PlayerController, VerbosityLevel};
use crate::loader::{AsyncCardDatabase, CardDefinition, DeckList};
use crate::network::protocol::{CardReveal, ChoiceType, ClientMessage, DeckSubmission, RevealReason, ServerMessage};
use anyhow::{anyhow, Result};
use futures_util::{SinkExt, StreamExt};
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::net::TcpStream;
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};

/// Shared queue for card reveals from the server
///
/// Used in `run_game()` to communicate card reveals from the WebSocket handler
/// to the game thread, which can then queue them into the appropriate library.
pub type SharedRevealQueue = Arc<Mutex<VecDeque<(PlayerId, CardId, RevealReason)>>>;

// ═══════════════════════════════════════════════════════════════════════════
// CLIENT CONFIGURATION
// ═══════════════════════════════════════════════════════════════════════════

/// Client configuration
#[derive(Debug, Clone)]
pub struct ClientConfig {
    /// Server address (host:port)
    pub server: String,
    /// Server password (empty if no password)
    pub password: String,
    /// Player name
    pub player_name: String,
    /// Deck file path
    pub deck_path: PathBuf,
    /// Path to cardsfolder for loading cards
    pub cardsfolder: PathBuf,
}

impl ClientConfig {
    /// Create a new client config
    pub fn new(server: String, password: String, player_name: String, deck_path: PathBuf) -> Self {
        Self {
            server,
            password,
            player_name,
            deck_path,
            cardsfolder: PathBuf::from("cardsfolder"),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// CLIENT GAME STATE
// ═══════════════════════════════════════════════════════════════════════════

/// Information needed to initialize client game state from GameStarted message
pub struct GameStartInfo {
    pub your_player_id: PlayerId,
    pub opponent_name: String,
    pub opening_hand: Vec<CardReveal>,
    pub opponent_hand_count: usize,
    pub library_size: usize,
    pub opponent_library_size: usize,
    pub starting_life: i32,
    pub initial_state_hash: u64,
}

/// Shadow game state maintained by the client
///
/// This mirrors the server's game state but:
/// - Libraries use LibraryMode::Remote (contents unknown until revealed)
/// - Only sees own hand contents and public information
/// - Syncs via choice messages, not full state transfer
pub struct ClientGameState {
    /// Shadow game state
    pub game: GameState,
    /// Our player ID
    pub our_player_id: PlayerId,
    /// Opponent's player ID
    pub opponent_id: PlayerId,
    /// Known card definitions (cards revealed to us)
    pub known_cards: HashMap<CardId, CardDefinition>,
    /// Expected state hash from server
    pub expected_hash: u64,
    /// Opponent's name
    pub opponent_name: String,
    /// Current choice sequence number
    pub choice_seq: u32,
}

impl ClientGameState {
    /// Create a new client game state from GameStarted message
    pub fn new(info: GameStartInfo, card_db: &AsyncCardDatabase) -> Result<Self> {
        let our_player_id = info.your_player_id;

        // Determine opponent ID
        let opponent_id = if our_player_id.as_u32() == 0 {
            PlayerId::new(1)
        } else {
            PlayerId::new(0)
        };

        // Create player names
        let our_name = "You".to_string();

        // Create shadow game state with remote libraries
        let mut game = GameState::new_two_player_with_capacity(
            if our_player_id.as_u32() == 0 {
                our_name.clone()
            } else {
                info.opponent_name.clone()
            },
            if our_player_id.as_u32() == 0 {
                info.opponent_name.clone()
            } else {
                our_name.clone()
            },
            info.starting_life,
            100, // Estimated card count
        );

        // Set up our library as remote (we don't know the order)
        if let Some(zones) = game.get_player_zones_mut(our_player_id) {
            zones.library = crate::zones::CardZone::new_remote_library(our_player_id, info.library_size);
        }

        // Set up opponent's library as remote
        if let Some(zones) = game.get_player_zones_mut(opponent_id) {
            zones.library = crate::zones::CardZone::new_remote_library(opponent_id, info.opponent_library_size);
        }

        // Process opening hand - add cards to our hand
        let mut known_cards = HashMap::new();
        for reveal in info.opening_hand {
            // Create card from reveal (simplified - in full impl would use card_db)
            if let Some(card_def) = Self::card_from_reveal(&reveal, card_db) {
                known_cards.insert(reveal.card_id, card_def.clone());

                // Add card to game and to our hand
                let card = card_def.instantiate(reveal.card_id, our_player_id);
                game.cards.insert(reveal.card_id, card);

                if let Some(zones) = game.get_player_zones_mut(our_player_id) {
                    zones.hand.add(reveal.card_id);
                }
            }
        }

        // Add placeholder cards to opponent's hand (we don't know what they are)
        // Just track the count for now
        log::debug!(
            "Opponent has {} cards in hand (contents unknown)",
            info.opponent_hand_count
        );

        Ok(Self {
            game,
            our_player_id,
            opponent_id,
            known_cards,
            expected_hash: info.initial_state_hash,
            opponent_name: info.opponent_name,
            choice_seq: 0,
        })
    }

    /// Create a CardDefinition from a CardReveal
    fn card_from_reveal(reveal: &CardReveal, card_db: &AsyncCardDatabase) -> Option<CardDefinition> {
        // Try to get from card database first
        // If not found, create a minimal definition from the reveal
        // This is a blocking call - in production we'd want async
        if let Ok(Some(def)) = futures_executor::block_on(card_db.get_card(&reveal.name)) {
            // Clone the Arc contents to get an owned CardDefinition
            return Some((*def).clone());
        }

        // Fallback: Create minimal definition from reveal info
        // This is used when the client doesn't have the full card database
        log::warn!("Card '{}' not in database, using reveal info", reveal.name);
        None
    }

    /// Process a CardRevealed message
    pub fn process_card_revealed(
        &mut self,
        owner: PlayerId,
        card: CardReveal,
        reason: RevealReason,
        card_db: &AsyncCardDatabase,
    ) -> Result<()> {
        log::debug!("Card revealed: {} (owner={:?}, reason={:?})", card.name, owner, reason);

        // Get or create card definition
        if let Some(card_def) = Self::card_from_reveal(&card, card_db) {
            self.known_cards.insert(card.card_id, card_def.clone());

            // Create card instance if not already in game
            if self.game.cards.get(card.card_id).is_err() {
                let card_instance = card_def.instantiate(card.card_id, owner);
                self.game.cards.insert(card.card_id, card_instance);
            }

            // Handle based on reason
            match reason {
                RevealReason::Draw => {
                    // Queue card in library's pending_reveals buffer
                    if let Some(zones) = self.game.get_player_zones_mut(owner) {
                        zones.library.queue_reveal(card.card_id);
                    }
                }
                RevealReason::OpeningHand => {
                    // Already handled in new()
                }
                RevealReason::Played | RevealReason::Targeting | RevealReason::Effect => {
                    // Card is now public knowledge, already added above
                }
                RevealReason::Searched => {
                    // Tutor effect - card revealed but not yet in a known zone
                }
                RevealReason::TokenCreated => {
                    // Token created - add to battlefield (shared zone on GameState)
                    self.game.battlefield.add(card.card_id);
                }
            }
        }

        Ok(())
    }

    /// Process an OpponentChoice message (sync opponent's decision)
    pub fn process_opponent_choice(
        &mut self,
        _choice_seq: u32,
        _choice_type: ChoiceType,
        _choice_index: usize,
        description: &str,
    ) -> Result<()> {
        log::debug!("Opponent chose: {}", description);
        // FIXME-UNFINISHED: Should replay the choice on our shadow state to keep
        // it in sync with the server. Currently the client shadow state diverges
        // from server state after opponent choices. Need to integrate with GameLoop
        // to actually execute the choice locally.
        Ok(())
    }

    /// Verify state hash matches expected
    pub fn verify_hash(&mut self, expected: u64) -> bool {
        // FIXME-UNFINISHED: Should compute local network-safe hash and compare.
        // Currently just accepts server hash without verification.
        // Need to use HashMode::Network from state_hash module.
        self.expected_hash = expected;
        true
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// NETWORK CLIENT
// ═══════════════════════════════════════════════════════════════════════════

/// Network client for connecting to game server
pub struct NetworkClient {
    /// Client configuration
    config: ClientConfig,
    /// WebSocket connection
    ws: Option<WebSocketStream<MaybeTlsStream<TcpStream>>>,
    /// Client game state (after connection)
    state: Option<ClientGameState>,
    /// Card database
    card_db: Option<Arc<AsyncCardDatabase>>,
    /// Verbosity level for output
    verbosity: VerbosityLevel,
    /// Visual stacks mode for display
    visual_stacks: bool,
}

impl NetworkClient {
    /// Create a new network client
    pub fn new(config: ClientConfig) -> Self {
        Self {
            config,
            ws: None,
            state: None,
            card_db: None,
            verbosity: VerbosityLevel::Normal,
            visual_stacks: false,
        }
    }

    /// Set verbosity level
    pub fn set_verbosity(&mut self, verbosity: VerbosityLevel) {
        self.verbosity = verbosity;
    }

    /// Set visual stacks mode
    pub fn set_visual_stacks(&mut self, visual_stacks: bool) {
        self.visual_stacks = visual_stacks;
    }

    /// Get our player ID (after game start)
    pub fn our_player_id(&self) -> Option<PlayerId> {
        self.state.as_ref().map(|s| s.our_player_id)
    }

    /// Print the current game state (battlefield, life totals, etc.)
    ///
    /// Uses the shared display function, showing our player's hand contents.
    fn print_game_state(&self) {
        if self.verbosity < VerbosityLevel::Normal {
            return;
        }

        let state = match &self.state {
            Some(s) => s,
            None => return,
        };

        print_separator(Some("GAME STATE"));
        // Show our hand contents (not active player, since we're a network client)
        print_battlefield_state(&state.game, Some(state.our_player_id));
    }

    /// Connect to the server and authenticate
    pub async fn connect(&mut self) -> Result<()> {
        // Load card database
        log::info!("Loading card database...");
        let card_db = AsyncCardDatabase::new(self.config.cardsfolder.clone());
        card_db.eager_load().await?;
        self.card_db = Some(Arc::new(card_db));

        // Load deck
        log::info!("Loading deck from {:?}...", self.config.deck_path);
        let deck = crate::loader::DeckLoader::load_from_file(&self.config.deck_path)?;

        // Build WebSocket URL
        let url = format!("ws://{}", self.config.server);
        log::info!("Connecting to {}...", url);

        // Connect
        let (ws, _response) = connect_async(&url).await?;
        self.ws = Some(ws);

        // Send authentication
        let auth_msg = ClientMessage::Authenticate {
            password: self.config.password.clone(),
            player_name: self.config.player_name.clone(),
            deck: deck_to_submission(&deck),
        };
        self.send_message(&auth_msg).await?;

        // Wait for auth result
        let response = self.receive_message().await?;
        match response {
            ServerMessage::AuthResult {
                success,
                error,
                your_player_id,
            } => {
                if !success {
                    return Err(anyhow!("Authentication failed: {}", error.unwrap_or_default()));
                }
                log::info!(
                    "Authenticated as player {:?}",
                    your_player_id.unwrap_or(PlayerId::new(0))
                );
            }
            _ => {
                return Err(anyhow!("Unexpected response: expected AuthResult"));
            }
        }

        Ok(())
    }

    /// Wait for game to start and initialize shadow state
    pub async fn wait_for_game_start(&mut self) -> Result<()> {
        log::info!("Waiting for game to start...");

        loop {
            let msg = self.receive_message().await?;
            match msg {
                ServerMessage::WaitingForOpponent => {
                    log::info!("Waiting for opponent to connect...");
                }
                ServerMessage::GameStarted {
                    your_player_id,
                    opponent_name,
                    opening_hand,
                    opponent_hand_count,
                    library_size,
                    opponent_library_size,
                    starting_life,
                    initial_state_hash,
                    ..
                } => {
                    log::info!("Game started! Playing against {}", opponent_name);
                    log::info!(
                        "Opening hand: {} cards, Library: {} cards",
                        opening_hand.len(),
                        library_size
                    );

                    // Create shadow game state
                    let card_db = self.card_db.as_ref().expect("Card DB not loaded");
                    let info = GameStartInfo {
                        your_player_id,
                        opponent_name,
                        opening_hand,
                        opponent_hand_count,
                        library_size,
                        opponent_library_size,
                        starting_life,
                        initial_state_hash,
                    };
                    self.state = Some(ClientGameState::new(info, card_db)?);

                    return Ok(());
                }
                ServerMessage::Error { message, fatal } => {
                    if fatal {
                        return Err(anyhow!("Server error: {}", message));
                    }
                    log::warn!("Server warning: {}", message);
                }
                _ => {
                    log::debug!("Ignoring message while waiting for game start");
                }
            }
        }
    }

    /// Run the game using message-based protocol (no local GameLoop)
    ///
    /// This is the simpler approach where the client just responds to server messages.
    /// The server runs the authoritative GameLoop and sends choice requests.
    /// Use `run_game()` for the preferred GameLoop-based approach.
    pub async fn run_game_message_based<C: PlayerController>(&mut self, mut controller: C) -> Result<Option<PlayerId>> {
        let card_db = self.card_db.clone().expect("Card DB not loaded");

        // Print initial game state
        self.print_game_state();

        loop {
            let msg = self.receive_message().await?;

            match msg {
                ServerMessage::ChoiceRequest {
                    choice_seq,
                    choice_type,
                    options,
                    state_hash,
                    context,
                } => {
                    log::debug!(
                        "Choice request #{}: {:?} with {} options",
                        choice_seq,
                        choice_type,
                        options.len()
                    );

                    // Print current game state before choice
                    self.print_game_state();

                    // Verify state hash
                    if let Some(ref mut state) = self.state {
                        if !state.verify_hash(state_hash) {
                            log::warn!("State hash mismatch! Expected {}", state_hash);
                        }
                        state.choice_seq = choice_seq;
                    }

                    // Get choice from local controller
                    let choice_index =
                        self.get_choice_from_controller(&mut controller, &choice_type, &options, context.as_ref())?;

                    // Send response
                    let response = ClientMessage::SubmitChoice {
                        choice_seq,
                        choice_index,
                    };
                    self.send_message(&response).await?;

                    // Log the choice made
                    if self.verbosity >= VerbosityLevel::Normal && choice_index < options.len() {
                        println!("  → You chose: {}", options[choice_index]);
                    }
                }

                ServerMessage::CardRevealed { owner, card, reason } => {
                    if self.verbosity >= VerbosityLevel::Normal {
                        let owner_str = if self.state.as_ref().map(|s| s.our_player_id) == Some(owner) {
                            "You"
                        } else {
                            "Opponent"
                        };
                        println!("  Card revealed ({:?}): {} - {}", reason, owner_str, card.name);
                    }
                    if let Some(ref mut state) = self.state {
                        state.process_card_revealed(owner, card, reason, &card_db)?;
                    }
                }

                ServerMessage::OpponentChoice {
                    choice_seq,
                    choice_type,
                    choice_index,
                    description,
                } => {
                    if self.verbosity >= VerbosityLevel::Normal {
                        println!("  Opponent chose: {}", description);
                    }
                    if let Some(ref mut state) = self.state {
                        state.process_opponent_choice(choice_seq, choice_type, choice_index, &description)?;
                    }
                }

                ServerMessage::GameEnded {
                    winner,
                    reason,
                    final_state_hash,
                } => {
                    // Print final game state
                    self.print_game_state();
                    log::info!("Game ended: {:?} - Winner: {:?}", reason, winner);
                    if let Some(ref mut state) = self.state {
                        state.expected_hash = final_state_hash;
                    }
                    return Ok(winner);
                }

                ServerMessage::Error { message, fatal } => {
                    if fatal {
                        return Err(anyhow!("Server error: {}", message));
                    }
                    log::warn!("Server warning: {}", message);
                }

                ServerMessage::Pong { .. } => {
                    // Ignore pong responses
                }

                _ => {
                    log::debug!("Ignoring unexpected message");
                }
            }
        }
    }

    /// Get a choice from the local controller
    ///
    /// Note: The network protocol uses simplified string-based options, so we can't
    /// directly use the full PlayerController interface which expects CardIds and
    /// detailed game state. Instead, we provide a simple index-based choice.
    fn get_choice_from_controller<C: PlayerController>(
        &self,
        controller: &mut C,
        choice_type: &ChoiceType,
        options: &[String],
        _context: Option<&crate::network::protocol::ChoiceContext>,
    ) -> Result<usize> {
        // Display options to user
        if self.verbosity >= VerbosityLevel::Normal {
            println!("\n=== Your Turn ===");
            println!("Choice type: {:?}", choice_type);
            println!("Options:");
            for (i, opt) in options.iter().enumerate() {
                println!("  {}: {}", i, opt);
            }
        }

        // For AI controllers, use their simple choice from index
        // For human controllers, read from stdin
        let choice = controller.choose_from_options(options);

        if choice >= options.len() {
            log::warn!("Invalid choice {}, defaulting to 0", choice);
            Ok(0)
        } else {
            Ok(choice)
        }
    }

    /// Send a message to the server
    async fn send_message(&mut self, msg: &ClientMessage) -> Result<()> {
        let ws = self.ws.as_mut().ok_or_else(|| anyhow!("Not connected"))?;
        let json = serde_json::to_string(msg)?;
        ws.send(Message::Text(json.into())).await?;
        Ok(())
    }

    /// Receive a message from the server
    async fn receive_message(&mut self) -> Result<ServerMessage> {
        let ws = self.ws.as_mut().ok_or_else(|| anyhow!("Not connected"))?;

        loop {
            match ws.next().await {
                Some(Ok(Message::Text(text))) => {
                    let msg: ServerMessage = serde_json::from_str(&text)?;
                    return Ok(msg);
                }
                Some(Ok(Message::Close(_))) => {
                    return Err(anyhow!("Connection closed"));
                }
                Some(Ok(_)) => {
                    // Ignore binary/ping/pong
                    continue;
                }
                Some(Err(e)) => {
                    return Err(e.into());
                }
                None => {
                    return Err(anyhow!("Connection closed"));
                }
            }
        }
    }

    /// Send a ping to keep connection alive
    pub async fn send_ping(&mut self) -> Result<()> {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        self.send_message(&ClientMessage::Ping {
            timestamp_ms: timestamp,
        })
        .await
    }

    /// Disconnect gracefully
    pub async fn disconnect(&mut self) -> Result<()> {
        self.send_message(&ClientMessage::Disconnect).await?;
        if let Some(mut ws) = self.ws.take() {
            ws.close(None).await?;
        }
        Ok(())
    }

    /// Run the game with a synchronized local GameLoop
    ///
    /// This runs an actual GameLoop on the client side, keeping the game state
    /// in sync with the server through choice messages. Card reveals from the
    /// server are queued into the local game state's libraries.
    ///
    /// # Architecture
    ///
    /// ```text
    /// ┌─────────────────────────────────────────────────────────────────┐
    /// │                        Client                                   │
    /// │  ┌──────────────────┐     ┌────────────────────────────────┐   │
    /// │  │ WebSocket Task   │     │ Game Thread (spawn_blocking)   │   │
    /// │  │                  │     │                                │   │
    /// │  │ ◄─ CardRevealed ─┼─────┼─► queue to library             │   │
    /// │  │ ◄─ OpponentChoice┼─────┼─► RemoteController             │   │
    /// │  │ ─► SubmitChoice ◄┼─────┼── NetworkLocalController       │   │
    /// │  │ ◄─ GameEnded ────┼─────┼─► signal end                   │   │
    /// │  └──────────────────┘     └────────────────────────────────┘   │
    /// └─────────────────────────────────────────────────────────────────┘
    /// ```
    pub async fn run_game<C: PlayerController + Send + 'static>(&mut self, controller: C) -> Result<Option<PlayerId>> {
        use crate::game::GameLoop;
        use crate::network::{
            LocalChoice, LocalControllerMessage, NetworkLocalController, RemoteChoice, RemoteController,
        };
        use std::sync::mpsc;
        use tokio::sync::mpsc as tokio_mpsc;

        // Take ownership of WebSocket and state
        let ws = self.ws.take().ok_or_else(|| anyhow!("Not connected"))?;
        let client_state = self.state.take().ok_or_else(|| anyhow!("Game not started"))?;
        let _our_player_id = client_state.our_player_id;
        let opponent_id = client_state.opponent_id;

        // Split WebSocket
        let (mut ws_sink, mut ws_stream) = ws.split();

        // Create channels for communication
        // Local controller -> WebSocket (our choices)
        let (local_choice_tx, mut local_choice_rx) = tokio_mpsc::channel::<LocalChoice>(16);
        // WebSocket -> Local controller (acknowledgments)
        let (local_msg_tx, local_msg_rx) = mpsc::channel::<LocalControllerMessage>();
        // WebSocket -> Remote controller (opponent choices)
        let (remote_choice_tx, remote_choice_rx) = mpsc::channel::<RemoteChoice>();
        // Game end signal
        let (game_end_tx, mut game_end_rx) = tokio_mpsc::channel::<Option<PlayerId>>(1);

        // Shared queue for card reveals (WebSocket handler -> Game thread)
        let reveal_queue: SharedRevealQueue = Arc::new(Mutex::new(VecDeque::new()));
        let reveal_queue_ws = reveal_queue.clone();

        // Create controllers
        let local_controller = NetworkLocalController::new(
            controller,
            // Convert tokio channel to std channel for blocking thread
            {
                let (std_tx, std_rx) = mpsc::channel();
                // Spawn a task to forward from std channel to tokio channel
                let local_choice_tx_clone = local_choice_tx.clone();
                tokio::spawn(async move {
                    while let Ok(choice) = std_rx.recv() {
                        if local_choice_tx_clone.send(choice).await.is_err() {
                            break;
                        }
                    }
                });
                std_tx
            },
            local_msg_rx,
        );
        let remote_controller = RemoteController::new(opponent_id, remote_choice_rx);

        // Get game state for the game thread
        let mut game = client_state.game;

        // Spawn WebSocket handler task
        let ws_handler = tokio::spawn(async move {
            loop {
                tokio::select! {
                    // Receive messages from server
                    msg = ws_stream.next() => {
                        match msg {
                            Some(Ok(Message::Text(text))) => {
                                match serde_json::from_str::<ServerMessage>(&text) {
                                    Ok(ServerMessage::CardRevealed { owner, card, reason }) => {
                                        log::debug!("Card revealed: {:?} for {:?} ({:?})", card.name, owner, reason);
                                        // Queue card reveal for the game thread to process
                                        if let Ok(mut queue) = reveal_queue_ws.lock() {
                                            queue.push_back((owner, card.card_id, reason));
                                        }
                                    }
                                    Ok(ServerMessage::OpponentChoice { choice_index, description, .. }) => {
                                        log::debug!("Opponent chose: {} ({})", description, choice_index);
                                        let _ = remote_choice_tx.send(RemoteChoice {
                                            choice_index,
                                            description,
                                        });
                                    }
                                    Ok(ServerMessage::ChoiceRequest { .. }) => {
                                        // We're being asked for a choice - acknowledge to local controller
                                        let _ = local_msg_tx.send(LocalControllerMessage::ChoiceAcknowledged);
                                    }
                                    Ok(ServerMessage::GameEnded { winner, .. }) => {
                                        log::info!("Game ended, winner: {:?}", winner);
                                        let _ = game_end_tx.send(winner).await;
                                        return;
                                    }
                                    Ok(ServerMessage::Error { message, fatal }) => {
                                        if fatal {
                                            log::error!("Fatal server error: {}", message);
                                            let _ = local_msg_tx.send(LocalControllerMessage::Error(message));
                                            return;
                                        }
                                        log::warn!("Server warning: {}", message);
                                    }
                                    Ok(_) => {
                                        // Ignore other messages
                                    }
                                    Err(e) => {
                                        log::error!("Failed to parse server message: {}", e);
                                    }
                                }
                            }
                            Some(Ok(Message::Close(_))) | None => {
                                log::info!("Connection closed");
                                let _ = local_msg_tx.send(LocalControllerMessage::GameEnded);
                                return;
                            }
                            Some(Ok(_)) => {
                                // Ignore binary/ping/pong
                            }
                            Some(Err(e)) => {
                                log::error!("WebSocket error: {}", e);
                                let _ = local_msg_tx.send(LocalControllerMessage::Error(e.to_string()));
                                return;
                            }
                        }
                    }

                    // Send our choices to server
                    choice = local_choice_rx.recv() => {
                        if let Some(choice) = choice {
                            let msg = ClientMessage::SubmitChoice {
                                choice_seq: 0, // TODO: Track sequence numbers
                                choice_index: choice.choice_index,
                            };
                            let text = serde_json::to_string(&msg).unwrap();
                            if ws_sink.send(Message::Text(text.into())).await.is_err() {
                                log::error!("Failed to send choice to server");
                                return;
                            }
                        }
                    }
                }
            }
        });

        // Run game loop in blocking thread
        // The game loop has a reveal drainer that will drain the queue before each draw
        let mut local_controller = local_controller;
        let mut remote_controller = remote_controller;
        let game_result = tokio::task::spawn_blocking(move || {
            // Create drain function that will be called before each draw
            let reveal_queue_clone = reveal_queue.clone();
            let drain_reveals = move |game: &mut GameState| {
                if let Ok(mut queue) = reveal_queue_clone.lock() {
                    while let Some((owner, card_id, reason)) = queue.pop_front() {
                        if matches!(reason, RevealReason::Draw) {
                            if let Some(zones) = game.get_player_zones_mut(owner) {
                                zones.library.queue_reveal(card_id);
                                log::debug!("Queued reveal for {:?}: {:?}", owner, card_id);
                            }
                        }
                        // Other reveal reasons (TokenCreated, etc.) would need card instantiation
                        // which requires access to card_db - skip for now
                    }
                }
            };

            // Process any reveals before starting (for opening hand)
            drain_reveals(&mut game);

            // Create game loop with reveal drainer hook
            let mut game_loop = GameLoop::new(&mut game).with_reveal_drainer(drain_reveals);
            game_loop.run_game(&mut local_controller, &mut remote_controller)
        });

        // Wait for game to end
        tokio::select! {
            result = game_result => {
                ws_handler.abort();
                match result {
                    Ok(Ok(game_result)) => Ok(game_result.winner),
                    Ok(Err(e)) => Err(anyhow!("Game error: {}", e)),
                    Err(e) => Err(anyhow!("Game thread panic: {}", e)),
                }
            }
            winner = game_end_rx.recv() => {
                ws_handler.abort();
                Ok(winner.flatten())
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// HELPER FUNCTIONS
// ═══════════════════════════════════════════════════════════════════════════

/// Convert DeckList to DeckSubmission for network protocol
fn deck_to_submission(deck: &DeckList) -> DeckSubmission {
    DeckSubmission::new(
        deck.main_deck.iter().map(|e| (e.card_name.clone(), e.count)).collect(),
        deck.sideboard.iter().map(|e| (e.card_name.clone(), e.count)).collect(),
    )
}

// ═══════════════════════════════════════════════════════════════════════════
// TESTS
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loader::DeckEntry;

    #[test]
    fn test_client_config() {
        let config = ClientConfig::new(
            "localhost:17771".to_string(),
            "secret".to_string(),
            "Alice".to_string(),
            PathBuf::from("deck.dck"),
        );

        assert_eq!(config.server, "localhost:17771");
        assert_eq!(config.password, "secret");
        assert_eq!(config.player_name, "Alice");
    }

    #[test]
    fn test_deck_to_submission() {
        let deck = DeckList {
            main_deck: vec![
                DeckEntry {
                    card_name: "Lightning Bolt".to_string(),
                    count: 4,
                },
                DeckEntry {
                    card_name: "Mountain".to_string(),
                    count: 20,
                },
            ],
            sideboard: vec![DeckEntry {
                card_name: "Pyroclasm".to_string(),
                count: 2,
            }],
        };

        let submission = deck_to_submission(&deck);
        assert_eq!(submission.main_deck_size(), 24);
        assert_eq!(submission.sideboard_size(), 2);
    }
}
