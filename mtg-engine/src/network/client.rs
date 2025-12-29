//! WebSocket client for multiplayer MTG
//!
//! Implements a client that:
//! - Connects to game server over WebSocket
//! - Maintains shadow game state with remote libraries
//! - Processes server messages and syncs game state
//! - Proxies choices to local PlayerController

use crate::core::{CardId, PlayerId};
use crate::game::{GameState, PlayerController, VerbosityLevel};
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

        // Process opening hand - register cards as known but DON'T add to hand yet
        // For synchronized GameLoop mode, the cards will be drawn from library via
        // the GameLoop after CardRevealed messages queue them. This avoids double-adding.
        let mut known_cards = HashMap::new();
        for reveal in info.opening_hand {
            if let Some(card_def) = Self::card_from_reveal(&reveal, card_db) {
                known_cards.insert(reveal.card_id, card_def.clone());
                // Just register the card definition - don't add to game or hand
                // The CardRevealed messages will handle zone placement
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
    /// Our deck (loaded during connect, used for synchronized GameLoop mode)
    our_deck: Option<DeckList>,
    /// Opponent's deck (received from server if deck_visibility enabled)
    opponent_deck: Option<DeckList>,
    /// Verbosity level for output
    verbosity: VerbosityLevel,
    /// Visual stacks mode for display
    visual_stacks: bool,
    /// Network debug mode: populate debug fields in protocol messages and validate state hashes
    network_debug: bool,
    /// Enable gamelog tagging ([GAMELOG ...] prefix)
    tag_gamelogs: bool,
    /// Output file for gamelogs (None = stdout)
    gamelog_output: Option<PathBuf>,
}

impl NetworkClient {
    /// Create a new network client
    pub fn new(config: ClientConfig) -> Self {
        Self {
            config,
            ws: None,
            state: None,
            card_db: None,
            our_deck: None,
            opponent_deck: None,
            verbosity: VerbosityLevel::Normal,
            visual_stacks: false,
            network_debug: false,
            tag_gamelogs: false,
            gamelog_output: None,
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

    /// Enable network debug mode for synchronization validation
    ///
    /// When enabled, the client includes state hashes in SubmitChoice messages
    /// and validates server's state hash after each choice. Helps detect desync.
    pub fn set_network_debug(&mut self, enabled: bool) {
        self.network_debug = enabled;
        if enabled {
            log::info!("Network debug mode ENABLED: state hash validation active");
        }
    }

    /// Enable gamelog tagging for equivalence testing
    ///
    /// When enabled, the client's shadow GameLoop logs [GAMELOG] entries.
    /// This enables 4-way equivalence testing (local, server, client1, client2).
    pub fn set_tag_gamelogs(&mut self, enabled: bool) {
        self.tag_gamelogs = enabled;
        if enabled {
            log::info!("Tag gamelogs ENABLED: client shadow state will emit [GAMELOG] entries");
        }
    }

    /// Set output file for gamelogs
    ///
    /// If set, gamelogs will be written to this file instead of stdout.
    /// This allows capturing client-side gamelogs for comparison in equivalence tests.
    pub fn set_gamelog_output(&mut self, path: PathBuf) {
        self.gamelog_output = Some(path);
    }

    /// Get our player ID (after game start)
    pub fn our_player_id(&self) -> Option<PlayerId> {
        self.state.as_ref().map(|s| s.our_player_id)
    }

    /// Connect to the server and authenticate
    pub async fn connect(&mut self) -> Result<()> {
        // Load card database
        log::info!("Loading card database...");
        let card_db = AsyncCardDatabase::new(self.config.cardsfolder.clone());
        card_db.eager_load().await?;
        self.card_db = Some(Arc::new(card_db));

        // Load deck and store for synchronized GameLoop mode
        log::info!("Loading deck from {:?}...", self.config.deck_path);
        let deck = crate::loader::DeckLoader::load_from_file(&self.config.deck_path)?;
        self.our_deck = Some(deck.clone());

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
    ///
    /// For synchronized GameLoop mode:
    /// 1. Receives GameStarted with opponent_decklist
    /// 2. Uses GameInitializer to create game with matching card IDs
    /// 3. Converts libraries to Remote mode
    /// 4. Receives CardRevealed messages for opening hands (14 cards)
    /// 5. Queues revealed card IDs for the shadow GameLoop to draw
    pub async fn wait_for_game_start(&mut self) -> Result<()> {
        use crate::loader::GameInitializer;

        log::info!("Waiting for game to start...");

        // First phase: wait for GameStarted
        let (
            our_hand_count,
            opponent_hand_count,
            our_player_id,
            opponent_name,
            starting_life,
            initial_state_hash,
            opponent_decklist,
        ) = loop {
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
                    opponent_library_size: _,
                    starting_life,
                    initial_state_hash,
                    opponent_decklist,
                } => {
                    log::info!("Game started! Playing against {}", opponent_name);
                    log::info!(
                        "Opening hand: {} cards, Library: {} cards",
                        opening_hand.len(),
                        library_size
                    );

                    let our_hand_count = opening_hand.len();
                    break (
                        our_hand_count,
                        opponent_hand_count,
                        your_player_id,
                        opponent_name,
                        starting_life,
                        initial_state_hash,
                        opponent_decklist,
                    );
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
        };

        // Store opponent deck if provided
        if let Some(ref deck_info) = opponent_decklist {
            self.opponent_deck = Some(deck_info.to_deck_list());
        }

        // Get decks for initialization
        let our_deck = self.our_deck.as_ref().ok_or_else(|| anyhow!("Our deck not loaded"))?;
        // For opponent deck: use provided decklist, or fall back to our deck (mirror match)
        let opponent_deck = self.opponent_deck.as_ref().unwrap_or(our_deck);

        // Determine player order - GameInitializer expects P1's deck first, then P2's
        let we_are_p1 = our_player_id.as_u32() == 0;
        let (p1_deck, p2_deck, p1_name, p2_name) = if we_are_p1 {
            (our_deck, opponent_deck, "You".to_string(), opponent_name.clone())
        } else {
            (opponent_deck, our_deck, opponent_name.clone(), "You".to_string())
        };

        // Debug: log deck order for entity ID verification
        log::debug!(
            "Client init: we_are_p1={}, p1_deck entries={}, p2_deck entries={}",
            we_are_p1,
            p1_deck.main_deck.len(),
            p2_deck.main_deck.len()
        );
        if log::log_enabled!(log::Level::Trace) {
            for (i, entry) in p1_deck.main_deck.iter().enumerate() {
                log::trace!("P1 deck[{}]: {}x {}", i, entry.count, entry.card_name);
            }
            for (i, entry) in p2_deck.main_deck.iter().enumerate() {
                log::trace!("P2 deck[{}]: {}x {}", i, entry.count, entry.card_name);
            }
        }

        // Initialize game using GameInitializer for deterministic card IDs
        let card_db = self.card_db.as_ref().expect("Card DB not loaded");
        let initializer = GameInitializer::new(card_db);
        let mut game = initializer
            .init_game(p1_name, p1_deck, p2_name, p2_deck, starting_life)
            .await?;

        // Get player IDs
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;
        let opponent_id = if we_are_p1 { p2_id } else { p1_id };

        // Convert libraries to Remote mode - we don't know the shuffle order
        // The server has shuffled and drawn, we need to receive reveals to know card order
        let our_lib_size = game
            .get_player_zones(our_player_id)
            .map(|z| z.library.len())
            .unwrap_or(0);
        let opp_lib_size = game.get_player_zones(opponent_id).map(|z| z.library.len()).unwrap_or(0);

        // Clear the local libraries and set them to Remote mode
        if let Some(zones) = game.get_player_zones_mut(our_player_id) {
            zones.library = crate::zones::CardZone::new_remote_library(our_player_id, our_lib_size);
        }
        if let Some(zones) = game.get_player_zones_mut(opponent_id) {
            zones.library = crate::zones::CardZone::new_remote_library(opponent_id, opp_lib_size);
        }

        // Create ClientGameState with the initialized game
        self.state = Some(ClientGameState {
            game,
            our_player_id,
            opponent_id,
            known_cards: HashMap::new(),
            expected_hash: initial_state_hash,
            opponent_name: opponent_name.clone(),
            choice_seq: 0,
        });

        // Second phase: receive CardRevealed messages for opening hands
        // Server sends our_hand_count + opponent_hand_count CardRevealed messages
        let expected_reveals = our_hand_count + opponent_hand_count;
        let mut reveals_received = 0;

        while reveals_received < expected_reveals {
            let msg = self.receive_message().await?;
            match msg {
                ServerMessage::CardRevealed { owner, card, reason } => {
                    log::debug!(
                        "Opening hand reveal {}/{}: {} (id={:?}) for {:?}",
                        reveals_received + 1,
                        expected_reveals,
                        card.name,
                        card.card_id,
                        owner
                    );
                    // Queue the card ID in the appropriate library for drawing
                    if let Some(ref mut state) = self.state {
                        if let Some(zones) = state.game.get_player_zones_mut(owner) {
                            zones.library.queue_reveal(card.card_id);
                        }
                    }
                    reveals_received += 1;
                    let _ = reason; // Reason is always Draw for opening hand
                }
                ServerMessage::Error { message, fatal } => {
                    if fatal {
                        return Err(anyhow!("Server error: {}", message));
                    }
                    log::warn!("Server warning: {}", message);
                }
                _ => {
                    log::debug!("Unexpected message while waiting for opening reveals: {:?}", msg);
                }
            }
        }

        log::info!("Received {} opening hand reveals, shadow state ready", reveals_received);
        Ok(())
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
            LocalChoice, LocalControllerMessage, NetworkLocalController, RemoteController, RemoteMessage,
        };
        use std::sync::mpsc;
        use tokio::sync::mpsc as tokio_mpsc;

        // Take ownership of WebSocket and state
        let ws = self.ws.take().ok_or_else(|| anyhow!("Not connected"))?;
        let client_state = self.state.take().ok_or_else(|| anyhow!("Game not started"))?;
        let our_player_id = client_state.our_player_id;
        let opponent_id = client_state.opponent_id;

        // Determine if we're P1 (PlayerId(0)) or P2 (PlayerId(1))
        // GameLoop.run_game expects (controller1, controller2) where controller1 is for P1
        let we_are_p1 = our_player_id.as_u32() == 0;

        // Split WebSocket
        let (mut ws_sink, mut ws_stream) = ws.split();

        // Create channels for communication
        // Local controller -> WebSocket (our choices)
        let (local_choice_tx, mut local_choice_rx) = tokio_mpsc::channel::<LocalChoice>(16);
        // WebSocket -> Local controller (acknowledgments)
        let (local_msg_tx, local_msg_rx) = mpsc::channel::<LocalControllerMessage>();
        // WebSocket -> Remote controller (opponent choices)
        let (remote_choice_tx, remote_choice_rx) = mpsc::channel::<RemoteMessage>();
        // Game end signal: (winner, server_action_count)
        let (game_end_tx, mut game_end_rx) = tokio_mpsc::channel::<(Option<PlayerId>, u64)>(1);

        // Shared queue for card reveals (WebSocket handler -> Game thread)
        let reveal_queue: SharedRevealQueue = Arc::new(Mutex::new(VecDeque::new()));
        let reveal_queue_ws = reveal_queue.clone();

        // Debug mode flag for sync validation
        let network_debug = self.network_debug;

        // Create controllers
        let local_controller = NetworkLocalController::new(
            controller,
            // Convert tokio channel to std channel for blocking thread
            {
                let (std_tx, std_rx) = mpsc::channel::<LocalChoice>();
                // Spawn a blocking task to forward from std channel to tokio channel
                // IMPORTANT: Must use spawn_blocking because std_rx.recv() blocks
                let local_choice_tx_clone = local_choice_tx.clone();
                tokio::task::spawn_blocking(move || {
                    let runtime = tokio::runtime::Handle::current();
                    while let Ok(choice) = std_rx.recv() {
                        log::trace!("Bridge: forwarding choice {} to tokio channel", choice.choice_index);
                        let result = runtime.block_on(local_choice_tx_clone.send(choice));
                        if let Err(e) = result {
                            log::debug!("Bridge: failed to forward choice: {:?}", e);
                            break;
                        }
                    }
                    log::debug!("Bridge: task exiting (channel closed)");
                });
                std_tx
            },
            local_msg_rx,
        )
        .with_network_debug(network_debug);
        let remote_controller = RemoteController::new(opponent_id, remote_choice_rx);

        // Get game state for the game thread
        let mut game = client_state.game;

        // Configure logger for gamelog tagging
        let tag_gamelogs = self.tag_gamelogs;
        let _gamelog_output = self.gamelog_output.clone(); // TODO(mtg-037fw): Use for file output
        if tag_gamelogs {
            game.logger.set_tag_gamelogs(true);
            log::debug!("Client GameLoop: tag_gamelogs enabled");
        }

        // Spawn WebSocket handler task
        // Track last sent action_count and action log for validation
        let ws_handler = tokio::spawn(async move {
            let mut last_sent_action_count: Option<u64> = None;
            let mut last_sent_actions: Option<String> = None;
            // Store the server's authoritative action_count from the last ChoiceRequest
            // This MUST be used instead of the client's shadow state action_count
            let mut server_action_count: Option<u64> = None;
            // Store the server's choice_seq from the last ChoiceRequest
            // We MUST echo this back in SubmitChoice
            let mut server_choice_seq: Option<u32> = None;

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
                                        log::debug!("WebSocket: opponent chose {} (idx={}), sending to RemoteController channel", description, choice_index);
                                        match remote_choice_tx.send(RemoteMessage::Choice {
                                            choice_index,
                                            description: description.clone(),
                                        }) {
                                            Ok(()) => log::trace!("WebSocket: sent opponent choice to RemoteController channel successfully"),
                                            Err(e) => log::error!("WebSocket: FAILED to send opponent choice to RemoteController channel: {:?}", e),
                                        }
                                    }
                                    Ok(ServerMessage::ChoiceRequest { action_count, choice_seq, .. }) => {
                                        // Server is asking for a choice - forward to NetworkLocalController
                                        // This is the synchronization point: the controller waits for this
                                        // before making a choice, ensuring client doesn't run ahead of server
                                        server_action_count = Some(action_count);
                                        server_choice_seq = Some(choice_seq);
                                        log::debug!("Player {:?}: Received ChoiceRequest #{}, server action_count={}", our_player_id, choice_seq, action_count);
                                        // Notify the local controller that server is ready for a choice
                                        let _ = local_msg_tx.send(LocalControllerMessage::ChoiceRequest {
                                            action_count,
                                            choice_seq,
                                        });
                                    }
                                    Ok(ServerMessage::ChoiceAccepted { choice_seq, action_count: server_action_count, .. }) => {
                                        // Server accepted our choice - validate action_count in network debug mode
                                        if network_debug {
                                            if let Some(client_action_count) = last_sent_action_count {
                                                if client_action_count != server_action_count {
                                                    log::error!(
                                                        "SYNC ERROR: action_count mismatch! client={} server={}",
                                                        client_action_count, server_action_count
                                                    );
                                                    // Log the client's action log for debugging
                                                    if let Some(ref actions) = last_sent_actions {
                                                        log::error!("Client's last 20 actions:\n{}", actions);
                                                    }
                                                    let _ = local_msg_tx.send(LocalControllerMessage::Error(
                                                        format!(
                                                            "Action count sync failure: client={} server={}",
                                                            client_action_count, server_action_count
                                                        )
                                                    ));
                                                    return;
                                                }
                                                log::debug!(
                                                    "SYNC OK: action_count={} (choice {})",
                                                    server_action_count, choice_seq
                                                );
                                            }
                                        }
                                        log::trace!("WebSocket: choice {} accepted (action_count={}), sending ack", choice_seq, server_action_count);
                                        let _ = local_msg_tx.send(LocalControllerMessage::ChoiceAcknowledged);
                                    }
                                    Ok(ServerMessage::GameEnded { winner, action_count, .. }) => {
                                        log::info!("Game ended, winner: {:?}, action_count: {}", winner, action_count);
                                        // Signal RemoteController to exit gracefully before we drop the channel
                                        let _ = remote_choice_tx.send(RemoteMessage::GameEnded);
                                        let _ = game_end_tx.send((winner, action_count)).await;
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
                                log::debug!("WebSocket: connection closed by server");
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
                            // CRITICAL: Use server's action_count, not the client's shadow state count
                            // The client's shadow state can drift from server due to timing/reveal issues
                            // The server's action_count from ChoiceRequest is the authoritative source
                            let action_count_to_send = server_action_count.unwrap_or_else(|| {
                                log::warn!(
                                    "WebSocket: No server action_count received yet, using client's {} (may cause sync error)",
                                    choice.action_count
                                );
                                choice.action_count
                            });

                            // Log comparison for debugging (only when they differ)
                            if network_debug && choice.action_count != action_count_to_send {
                                log::debug!(
                                    "WebSocket: action_count differs - client shadow={} server={}",
                                    choice.action_count, action_count_to_send
                                );
                            }

                            log::trace!(
                                "WebSocket: sending choice {} (server_action_count={}) to server",
                                choice.choice_index,
                                action_count_to_send
                            );
                            // Track for debug validation
                            last_sent_action_count = Some(action_count_to_send);
                            last_sent_actions = choice.last_actions;

                            // Get server's choice_seq, falling back to 0 if not received
                            let choice_seq_to_send = server_choice_seq.unwrap_or_else(|| {
                                log::warn!(
                                    "WebSocket: No server choice_seq received yet, using 0 (may cause sync error)"
                                );
                                0
                            });

                            // Clear server state after use (only valid for one choice)
                            server_action_count = None;
                            server_choice_seq = None;

                            let msg = ClientMessage::SubmitChoice {
                                choice_seq: choice_seq_to_send,
                                choice_index: choice.choice_index,
                                action_count: action_count_to_send,
                                timestamp_ms: crate::network::protocol::now_ms(),
                                // TODO(mtg-037fw): Populate these in debug mode
                                client_state_hash: None,
                                debug_info: None,
                            };
                            let text = serde_json::to_string(&msg).unwrap();
                            if ws_sink.send(Message::Text(text.into())).await.is_err() {
                                log::error!("WebSocket: failed to send choice to server");
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
            // Skip opening hand setup since server already drew hands and sent reveals
            let mut game_loop = GameLoop::new(&mut game)
                .with_reveal_drainer(drain_reveals)
                .skip_opening_hands();

            // Pass controllers in the correct order based on which player we are
            // GameLoop.run_game expects (controller_for_p1, controller_for_p2)
            log::debug!("Client GameLoop: we_are_p1={}", we_are_p1);
            if we_are_p1 {
                // We're P1: local controller is for P1, remote is for P2
                game_loop.run_game(&mut local_controller, &mut remote_controller)
            } else {
                // We're P2: remote controller is for P1, local is for P2
                game_loop.run_game(&mut remote_controller, &mut local_controller)
            }
        });

        // Wait for game to end
        // network_debug is used for action_count verification logging
        tokio::select! {
            result = game_result => {
                ws_handler.abort();
                match result {
                    Ok(Ok(game_result)) => {
                        log::info!("Client GameLoop finished: winner={:?}, action_count={}", game_result.winner, game_result.action_count);
                        Ok(game_result.winner)
                    }
                    Ok(Err(e)) => {
                        // Check if this was a legitimate game-end signal
                        // The GameLoop returns error when RemoteController gets GameEnded and returns ExitGame
                        // In that case, we should try to get the winner from game_end_rx
                        let error_msg = e.to_string();
                        if error_msg.contains("Game exit requested") {
                            // Try to receive game end signal with short timeout
                            match tokio::time::timeout(
                                std::time::Duration::from_millis(100),
                                game_end_rx.recv()
                            ).await {
                                Ok(Some((winner, server_action_count))) => {
                                    log::info!(
                                        "Game ended gracefully via ExitGame signal: winner={:?}, server_action_count={}",
                                        winner, server_action_count
                                    );
                                    Ok(winner)
                                }
                                Ok(None) => {
                                    // Channel closed without sending winner - treat as graceful exit
                                    log::info!("Game ended gracefully (channel closed)");
                                    Ok(None)
                                }
                                Err(_) => {
                                    // Timeout - no game end signal, but still treat as graceful exit
                                    // since we received ExitGame from controller
                                    log::info!("Game ended gracefully (no server signal within timeout)");
                                    Ok(None)
                                }
                            }
                        } else {
                            Err(anyhow!("Game error: {}", e))
                        }
                    }
                    Err(e) => Err(anyhow!("Game thread panic: {}", e)),
                }
            }
            server_end = game_end_rx.recv() => {
                ws_handler.abort();
                match server_end {
                    Some((winner, server_action_count)) => {
                        log::info!("Server signaled game end: winner={:?}, server_action_count={}", winner, server_action_count);
                        // Note: Client GameLoop was aborted, so we can't compare action_counts here
                        // The per-choice verification during the game should catch sync issues
                        Ok(winner)
                    }
                    None => Ok(None),
                }
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
