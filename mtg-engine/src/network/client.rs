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
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};

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
    /// FIXME-UNFINISHED: This duplicates GameLoop::print_battlefield_state from logging.rs.
    /// Should refactor to share code, ideally by using GameLoop for client-side game
    /// execution rather than this separate shadow state management.
    fn print_game_state(&self) {
        if self.verbosity < VerbosityLevel::Normal {
            return;
        }

        let state = match &self.state {
            Some(s) => s,
            None => return,
        };

        println!("\n════════════════════════════════════════════════════════════════");
        println!("                      GAME STATE");
        println!("════════════════════════════════════════════════════════════════");

        // Print each player's state
        for player in state.game.players.iter() {
            let is_us = player.id == state.our_player_id;
            let marker = if is_us { " (You)" } else { "" };

            println!("\n{}{}: Life {}", player.name, marker, player.life);

            if let Some(zones) = state.game.get_player_zones(player.id) {
                println!(
                    "  Hand: {} | Library: {} | Graveyard: {} | Exile: {}",
                    zones.hand.len(),
                    zones.library.len(),
                    zones.graveyard.len(),
                    zones.exile.len()
                );

                // Show our hand contents
                if is_us && !zones.hand.is_empty() {
                    println!("  Hand contents:");
                    for &card_id in &zones.hand.cards {
                        if let Ok(card) = state.game.cards.get(card_id) {
                            println!("    - {} ({})", card.name, card.mana_cost);
                        }
                    }
                }
            }

            // Battlefield permanents controlled by this player
            let player_permanents: Vec<_> = state
                .game
                .battlefield
                .cards
                .iter()
                .filter_map(|&card_id| {
                    state.game.cards.get(card_id).ok().and_then(|card| {
                        if card.controller == player.id {
                            Some((card_id, card))
                        } else {
                            None
                        }
                    })
                })
                .collect();

            if player_permanents.is_empty() {
                println!("  Battlefield: (empty)");
            } else {
                println!("  Battlefield:");
                for (card_id, card) in player_permanents {
                    let tap_status = if card.tapped { " (tapped)" } else { "" };
                    if card.is_creature() {
                        let power = card.current_power();
                        let toughness = card.current_toughness();
                        println!("    {} ({}) - {}/{}{}", card.name, card_id, power, toughness, tap_status);
                    } else {
                        println!("    {} ({}){}", card.name, card_id, tap_status);
                    }
                }
            }
        }

        println!("════════════════════════════════════════════════════════════════\n");
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

    /// Run the game loop with a local controller
    pub async fn run_game<C: PlayerController>(&mut self, mut controller: C) -> Result<Option<PlayerId>> {
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
