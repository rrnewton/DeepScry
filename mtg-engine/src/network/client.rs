//! WebSocket client for multiplayer MTG
//!
//! Implements a client that:
//! - Connects to game server over WebSocket
//! - Maintains shadow game state with remote libraries
//! - Processes server messages and syncs game state
//! - Proxies choices to local PlayerController

use crate::core::{CardId, PlayerId};
use crate::game::{GameState, PlayerController, Step, VerbosityLevel};
use crate::loader::{AsyncCardDatabase, CardDefinition, DeckList};
use crate::network::protocol::{CardReveal, ChoiceType, ClientMessage, DeckSubmission, RevealReason, ServerMessage};
use anyhow::{anyhow, Result};
use futures_util::{SinkExt, StreamExt};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};

/// Reveal message sent from WebSocket handler to game thread
/// Includes full CardReveal info so game thread can instantiate cards
type RevealMsg = (PlayerId, CardReveal, RevealReason);

// ═══════════════════════════════════════════════════════════════════════════
// GAME LOOP MESSAGE (SINGLE-CHANNEL ARCHITECTURE)
// ═══════════════════════════════════════════════════════════════════════════

/// All messages from WebSocket handler to the game loop.
///
/// The single-channel architecture routes ALL server messages through one channel.
/// This eliminates race conditions between reveal processing and controller messages
/// by ensuring messages are processed in the exact order they were received from
/// the server.
///
/// The game loop's `drain_messages` callback processes these sequentially:
/// 1. CardRevealed - instantiates card in game state
/// 2. ChoiceRequest - signals NetworkLocalController to proceed
/// 3. OpponentChoice - signals RemoteController to proceed
/// 4. GameEnded - signals both controllers to exit
#[derive(Debug)]
pub enum GameLoopMessage {
    /// A card was revealed by the server - instantiate it in game state
    CardRevealed {
        owner: PlayerId,
        card_id: CardId,
        card_name: String,
        reason: RevealReason,
    },
    /// Server is requesting a choice from us (signals NetworkLocalController)
    ChoiceRequest { action_count: u64, choice_seq: u32 },
    /// Server acknowledged our previous choice (signals NetworkLocalController)
    ChoiceAcknowledged,
    /// Opponent made a choice - apply it to shadow state (signals RemoteController)
    OpponentChoice {
        choice_indices: Vec<usize>,
        description: String,
        spell_ability: Option<crate::core::SpellAbility>,
    },
    /// Game has ended
    GameEnded {
        winner: Option<PlayerId>,
        action_count: u64,
    },
    /// Server reported a fatal error
    Error(String),
}

// ═══════════════════════════════════════════════════════════════════════════
// CARD REVEAL UTILITIES
// ═══════════════════════════════════════════════════════════════════════════

/// Get CardDefinition from CardReveal
///
/// Prefers using the embedded CardDefinition sent by the server (enables
/// clients to run without a local card database). Falls back to database
/// lookup if the server didn't include the definition.
fn get_card_def_from_reveal(reveal: &CardReveal, card_db: &AsyncCardDatabase) -> CardDefinition {
    // Prefer the CardDefinition sent by the server - this enables DB-free clients
    if let Some(ref card_def) = reveal.card_def {
        let mut def = card_def.clone();
        // Rebuild parsed_svars which is skipped during serialization (AbilityParams doesn't impl Serialize).
        // Without this, trigger effects that reference SVars (like earthbend) won't be parsed.
        def.rebuild_parsed_svars();
        return def;
    }

    // Fallback to local database lookup
    match futures_executor::block_on(card_db.get_card(&reveal.name)) {
        Ok(Some(def)) => (*def).clone(),
        Ok(None) => panic!(
            "Card '{}' not in database and server didn't provide definition",
            reveal.name
        ),
        Err(e) => panic!("Failed to load card '{}' from database: {}", reveal.name, e),
    }
}

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
    /// Player name (None = let server assign default with suffix)
    pub player_name: Option<String>,
    /// Deck file path
    pub deck_path: PathBuf,
    /// Path to cardsfolder for loading cards
    pub cardsfolder: PathBuf,
}

impl ClientConfig {
    /// Create a new client config
    pub fn new(server: String, password: String, player_name: Option<String>, deck_path: PathBuf) -> Self {
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
    pub your_name: String,
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
/// - Library card identities are unknown until revealed via RevealCard
/// - Only sees own hand contents and public information
/// - Syncs via choice messages, not full state transfer
///
/// TODO(mtg-qtqcr Phase 3): Complete late-binding architecture migration
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
    ///
    /// # Errors
    ///
    /// Returns an error if the card database cannot resolve opening hand cards.
    pub fn new(info: GameStartInfo, card_db: &AsyncCardDatabase) -> Result<Self> {
        let our_player_id = info.your_player_id;

        // Determine opponent ID
        let opponent_id = if our_player_id.as_u32() == 0 {
            PlayerId::new(1)
        } else {
            PlayerId::new(0)
        };

        // Use player names from game start info
        let our_name = info.your_name.clone();

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
                our_name
            },
            info.starting_life,
            100, // Estimated card count
        );

        // Set up our library as remote (we don't know the order)
        // TODO(mtg-qtqcr Phase 3): Use CardZone::new_library_with_cards() once server sends DeckCardIdRanges
        // For now, just create empty libraries since we don't have CardIDs yet
        if let Some(zones) = game.get_player_zones_mut(our_player_id) {
            zones.library = crate::zones::CardZone::new(crate::zones::Zone::Library, our_player_id);
        }

        // Set up opponent's library as remote
        if let Some(zones) = game.get_player_zones_mut(opponent_id) {
            zones.library = crate::zones::CardZone::new(crate::zones::Zone::Library, opponent_id);
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

    /// Create a CardDefinition from a CardReveal (delegates to free function)
    fn card_from_reveal(reveal: &CardReveal, card_db: &AsyncCardDatabase) -> Option<CardDefinition> {
        Some(get_card_def_from_reveal(reveal, card_db))
    }

    /// Process a CardRevealed message
    ///
    /// In the late-binding architecture:
    /// - For deck cards (Draw, OpeningHand, Played): CardID slot was pre-reserved, use insert()
    /// - For tokens (TokenCreated): New CardID, use insert_if_vacant() as fallback
    ///
    /// # Errors
    ///
    /// This function currently always succeeds, but returns Result for API consistency.
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

            // If this is a token definition with script_name, add to token_definitions
            // This allows the client to create tokens without local card database
            if let Some(script_name) = &card_def.script_name {
                if !self.game.token_definitions.contains_key(script_name) {
                    log::debug!("Adding token definition '{}' from server", script_name);
                    self.game
                        .token_definitions
                        .insert(script_name.clone(), std::sync::Arc::new(card_def.clone()));
                }
            }

            // Create card instance
            let card_instance = card_def.instantiate(card.card_id, owner);

            // Handle based on reason
            match reason {
                RevealReason::Draw | RevealReason::OpeningHand => {
                    // Late-binding: CardID slot was pre-reserved via init_game_reserve_only()
                    // Insert into the reserved slot (write-once semantics)
                    if !self.game.cards.is_revealed(card.card_id) {
                        self.game.cards.insert(card.card_id, card_instance);
                    }
                }
                RevealReason::Played => {
                    // Opponent plays a card - CardID slot was pre-reserved
                    if !self.game.cards.is_revealed(card.card_id) {
                        self.game.cards.insert(card.card_id, card_instance);
                    }
                    // Add to hand if not already there (card will be moved to stack/battlefield)
                    if let Some(zones) = self.game.get_player_zones_mut(owner) {
                        if !zones.hand.contains(card.card_id) {
                            zones.hand.add(card.card_id);
                        }
                    }
                }
                RevealReason::Targeting | RevealReason::Effect | RevealReason::Searched => {
                    // Card is now public knowledge - insert if not already revealed
                    if !self.game.cards.is_revealed(card.card_id) {
                        self.game.cards.insert(card.card_id, card_instance);
                    }
                }
                RevealReason::TokenCreated => {
                    // Token created - new CardID not from deck, use insert_if_vacant
                    if self.game.cards.insert_if_vacant(card.card_id, card_instance) {
                        self.game.battlefield.add(card.card_id);
                    }
                }
            }
        }

        Ok(())
    }

    /// Process an OpponentChoice message (sync opponent's decision)
    ///
    /// # Errors
    ///
    /// This function currently always succeeds, but returns Result for API consistency.
    pub fn process_opponent_choice(
        &mut self,
        _choice_seq: u32,
        _choice_type: ChoiceType,
        _choice_indices: &[usize],
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
    ///
    /// Note: Wildcard is intentional - ServerMessage has 12+ variants;
    /// we only expect AuthResult during connect, others are errors.
    ///
    /// # Errors
    ///
    /// Returns an error if card database loading, deck loading, WebSocket connection,
    /// or authentication fails.
    #[allow(clippy::wildcard_enum_match_arm)]
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
                your_name,
            } => {
                if !success {
                    return Err(anyhow!("Authentication failed: {}", error.unwrap_or_default()));
                }
                // Update player_name with server-assigned name if we didn't provide one
                if let Some(assigned_name) = your_name {
                    self.config.player_name = Some(assigned_name.clone());
                    log::info!(
                        "Authenticated as '{}' (player {:?})",
                        assigned_name,
                        your_player_id.unwrap_or_else(|| PlayerId::new(0))
                    );
                } else {
                    log::info!(
                        "Authenticated as player {:?}",
                        your_player_id.unwrap_or_else(|| PlayerId::new(0))
                    );
                }
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
    ///
    /// Note: Wildcards are intentional - ServerMessage has 12+ variants;
    /// we handle specific variants and log/ignore unexpected ones.
    ///
    /// # Errors
    ///
    /// Returns an error if WebSocket communication fails or game initialization fails.
    ///
    /// # Panics
    ///
    /// Panics if WebSocket communication or card database operations fail unexpectedly.
    #[allow(clippy::wildcard_enum_match_arm)]
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
            server_network_debug,
            deck_card_ids,     // Phase 3: CardID ranges for late-binding architecture
            token_definitions, // Token definitions for network clients without local card DB
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
                    network_debug,
                    deck_card_ids,
                    token_definitions,
                } => {
                    log::info!("Game started! Playing against {}", opponent_name);
                    log::info!(
                        "Opening hand: {} cards, Library: {} cards",
                        opening_hand.len(),
                        library_size
                    );
                    if network_debug {
                        log::info!("Network debug mode ENABLED by server");
                    }
                    if let Some(ref ranges) = deck_card_ids {
                        log::debug!(
                            "Deck CardID ranges: P1=[{}..{}), P2=[{}..{})",
                            ranges.p1_start,
                            ranges.p1_end,
                            ranges.p2_start,
                            ranges.p2_end
                        );
                    }
                    if !token_definitions.is_empty() {
                        log::info!("Received {} token definitions from server", token_definitions.len());
                    }

                    let our_hand_count = opening_hand.len();
                    break (
                        our_hand_count,
                        opponent_hand_count,
                        your_player_id,
                        opponent_name,
                        starting_life,
                        initial_state_hash,
                        opponent_decklist,
                        network_debug,
                        deck_card_ids,
                        token_definitions,
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

        // Apply server's network_debug setting to client
        self.network_debug = server_network_debug;

        // Store opponent deck - REQUIRED for synchronized GameLoop mode
        // Without this, card IDs will not match between clients
        let opponent_deck = match opponent_decklist {
            Some(ref deck_info) => deck_info.to_deck_list(),
            None => {
                return Err(anyhow!(
                    "Server did not send opponent deck list - cannot synchronize card IDs"
                ));
            }
        };
        self.opponent_deck = Some(opponent_deck.clone());

        // Get decks for initialization
        let our_deck = self.our_deck.as_ref().ok_or_else(|| anyhow!("Our deck not loaded"))?;
        let opponent_deck = &opponent_deck;

        // Determine player order - GameInitializer expects P1's deck first, then P2's
        let we_are_p1 = our_player_id.as_u32() == 0;
        // Use server-assigned name (should be populated from AuthResult)
        let our_name = self.config.player_name.clone().unwrap_or_else(|| "Player".to_string());
        let (p1_deck, p2_deck, p1_name, p2_name) = if we_are_p1 {
            (our_deck, opponent_deck, our_name, opponent_name.clone())
        } else {
            (opponent_deck, our_deck, opponent_name.clone(), our_name)
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

        // Initialize game using GameInitializer
        let card_db = self.card_db.as_ref().expect("Card DB not loaded");
        let initializer = GameInitializer::new(card_db);

        // Late-binding architecture: use init_game_reserve_only when server provides DeckCardIdRanges
        // This creates CardID slots upfront, with identities revealed later via CardRevealed messages
        let mut game = if let Some(ref ranges) = deck_card_ids {
            log::debug!(
                "Using late-binding architecture: {} total CardIDs (P1: [{}..{}), P2: [{}..{}))",
                ranges.total_cards(),
                ranges.p1_start,
                ranges.p1_end,
                ranges.p2_start,
                ranges.p2_end
            );
            initializer.init_game_reserve_only(p1_name, p2_name, starting_life, ranges)
        } else {
            // Fallback: instantiate cards locally (legacy mode, may cause ID mismatches)
            log::warn!("Server did not provide DeckCardIdRanges - using legacy initialization");
            initializer
                .init_game(p1_name, p1_deck, p2_name, p2_deck, starting_life)
                .await?
        };

        // Enable reveal logging for network games.
        // Both server and client must have matching skip_reveals settings
        // so their undo_logs contain the same RevealCard actions.
        game.set_skip_reveals(false);

        // Get player IDs
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;
        let opponent_id = if we_are_p1 { p2_id } else { p1_id };

        // Populate token_definitions from server's GameStarted message
        // This allows the client to create tokens without a local card database
        if !token_definitions.is_empty() {
            log::debug!("Populating {} token definitions from server", token_definitions.len());
            for (script_name, card_def) in token_definitions {
                log::trace!("  Token: {} -> {}", script_name, card_def.name);
                game.token_definitions
                    .insert(script_name, std::sync::Arc::new(card_def));
            }
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
                    // HIDDEN INFO ARCHITECTURE (mtg-qtqcr):
                    // - Real reveal: name is non-empty, instantiate the card
                    // - Dummy reveal: name is empty, opponent's hidden card - don't instantiate
                    let is_dummy_reveal = card.name.is_empty();

                    if is_dummy_reveal {
                        log::debug!(
                            "Opening hand dummy reveal {}/{}: CardID {} for {:?} (hidden)",
                            reveals_received + 1,
                            expected_reveals,
                            card.card_id.as_u32(),
                            owner
                        );
                        // Dummy reveal: CardID exists but we don't know what card it is
                        // The slot is already reserved, leave it empty (None in EntityStore)
                    } else {
                        log::debug!(
                            "Opening hand reveal {}/{}: {} (id={:?}) for {:?}",
                            reveals_received + 1,
                            expected_reveals,
                            card.name,
                            card.card_id,
                            owner
                        );

                        // Late-binding: instantiate the card now that we know its identity
                        // The CardID slot was already reserved via init_game_reserve_only()
                        let card_db = self.card_db.as_ref().expect("Card DB not loaded");
                        let card_def = get_card_def_from_reveal(&card, card_db);
                        let card_instance = card_def.instantiate(card.card_id, owner);

                        // Insert into the reserved slot (write-once semantics)
                        if let Some(ref mut state) = self.state {
                            state.game.cards.insert(card.card_id, card_instance);
                            state.known_cards.insert(card.card_id, card_def);
                        }
                    }

                    reveals_received += 1;
                    let _ = reason; // Reason is Draw for opening hand reveals
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

        // Log at DEBUG level with truncation for long messages
        if log::log_enabled!(log::Level::Debug) {
            let truncated = if json.len() > 500 {
                format!("{}... ({} bytes total)", &json[..500], json.len())
            } else {
                json.clone()
            };
            log::debug!("[CLIENT->SERVER] {}", truncated);
        }

        ws.send(Message::Text(json.into())).await?;
        Ok(())
    }

    /// Receive a message from the server
    async fn receive_message(&mut self) -> Result<ServerMessage> {
        let ws = self.ws.as_mut().ok_or_else(|| anyhow!("Not connected"))?;

        loop {
            match ws.next().await {
                Some(Ok(Message::Text(text))) => {
                    // Log at DEBUG level with truncation for long messages
                    if log::log_enabled!(log::Level::Debug) {
                        let truncated = if text.len() > 500 {
                            format!("{}... ({} bytes total)", &text[..500], text.len())
                        } else {
                            text.to_string()
                        };
                        log::debug!("[SERVER->CLIENT] {}", truncated);
                    }

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
    ///
    /// # Errors
    ///
    /// Returns an error if the WebSocket message cannot be sent.
    ///
    /// # Panics
    ///
    /// Panics if the system time is before the Unix epoch (should never happen).
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
    ///
    /// # Errors
    ///
    /// Returns an error if the disconnect message cannot be sent or WebSocket close fails.
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
    ///
    /// Note: Wildcards are intentional - ServerMessage and RevealReason have 12+/7+
    /// variants; we handle specific variants and log/ignore unexpected ones.
    ///
    /// # Errors
    ///
    /// Returns an error if not connected, game not started, or communication fails.
    ///
    /// # Panics
    ///
    /// Panics if internal channel communication fails or required state is missing.
    #[allow(clippy::wildcard_enum_match_arm)]
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
        // Fatal error signal from WebSocket handler
        let (fatal_error_tx, mut fatal_error_rx) = tokio_mpsc::channel::<String>(1);

        // Channel for card reveals (WebSocket handler -> Game thread)
        // Using std::sync::mpsc so game thread can do blocking recv_timeout
        let (reveal_tx, reveal_rx) = mpsc::channel::<RevealMsg>();

        // Debug mode flag for sync validation
        let network_debug = self.network_debug;

        // Create controllers
        // SINGLE-CHANNEL FIX: Controllers no longer need reveal_tx - reveals go directly
        // to reveal_rx from the WebSocket handler. Controllers don't push reveals anymore.

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
                        log::trace!("Bridge: forwarding choice {:?} to tokio channel", choice.choice_indices);
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

            // SINGLE-CHANNEL ARCHITECTURE: Send reveals directly to reveal_tx immediately.
            // This eliminates the race condition where drain_reveals() was called before
            // controllers had a chance to push their bundled reveals.
            //
            // The fix: reveals go DIRECTLY to reveal_tx as they arrive, not via bundling.
            // Controllers no longer need push_pending_reveals() - reveals are already
            // in reveal_rx waiting for drain_reveals() to process them.

            loop {
                tokio::select! {
                    // Receive messages from server
                    msg = ws_stream.next() => {
                        match msg {
                            Some(Ok(Message::Text(text))) => {
                                match serde_json::from_str::<ServerMessage>(&text) {
                                    Ok(ServerMessage::CardRevealed { owner, card, reason }) => {
                                        // SINGLE-CHANNEL FIX: Send reveals directly to reveal_tx.
                                        // This ensures drain_reveals() can process them in order,
                                        // eliminating the race condition from the bundling approach.
                                        log::debug!(
                                            "WebSocket: Sending reveal to game thread: {} (id={}) for {:?} ({:?})",
                                            card.name, card.card_id.as_u32(), owner, reason
                                        );
                                        if let Err(e) = reveal_tx.send((owner, card, reason)) {
                                            log::error!("WebSocket: Failed to send reveal: {:?}", e);
                                        }
                                    }
                                    Ok(ServerMessage::LibraryReordered { player, new_order }) => {
                                        // Library was shuffled after a search - update shadow state
                                        // TODO(mtg-library-sync): Apply this to the shadow GameLoop's library zone
                                        // For now, just log it. The name-based search protocol doesn't
                                        // require the client to know card positions in library.
                                        log::debug!(
                                            "WebSocket: Library reordered for player {:?}, new size: {}",
                                            player, new_order.len()
                                        );
                                    }
                                    Ok(ServerMessage::OpponentChoice { choice_indices, description, spell_ability, .. }) => {
                                        // SINGLE-CHANNEL FIX: No bundled reveals - they're already in reveal_tx.
                                        // Just send the choice to the controller.
                                        log::debug!(
                                            "WebSocket: opponent chose {} (indices={:?}), spell_ability={:?}",
                                            description, choice_indices, spell_ability
                                        );

                                        match remote_choice_tx.send(RemoteMessage::Choice {
                                            choice_indices,
                                            description: description.clone(),
                                            spell_ability,
                                            card_reveal: None,
                                            reveals: Vec::new(), // Empty - reveals sent directly to reveal_tx
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

                                        // SINGLE-CHANNEL FIX: No bundled reveals - they're already in reveal_tx.
                                        // drain_reveals() will have processed them by the time the controller
                                        // gets this ChoiceRequest and evaluates available options.
                                        log::debug!(
                                            "Player {:?}: Received ChoiceRequest #{}, server action_count={}",
                                            our_player_id, choice_seq, action_count
                                        );

                                        // Notify the local controller that server is ready for a choice
                                        let _ = local_msg_tx.send(LocalControllerMessage::ChoiceRequest {
                                            action_count,
                                            choice_seq,
                                            reveals: Vec::new(), // Empty - reveals sent directly to reveal_tx
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
                                        // CRITICAL: Send winner to game_end_tx FIRST, before signaling controllers
                                        // Otherwise the game loop might exit before we send the winner
                                        let _ = game_end_tx.send((winner, action_count)).await;
                                        // Now signal RemoteController to exit gracefully
                                        let _ = remote_choice_tx.send(RemoteMessage::GameEnded);
                                        return;
                                    }
                                    Ok(ServerMessage::Error { message, fatal }) => {
                                        if fatal {
                                            log::error!("Fatal server error: {}", message);
                                            let _ = local_msg_tx.send(LocalControllerMessage::Error(message.clone()));
                                            // Signal the fatal error to the main task
                                            let _ = fatal_error_tx.send(message).await;
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
                                "WebSocket: sending choice {:?} (server_action_count={}) to server",
                                choice.choice_indices,
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
                                choice_indices: choice.choice_indices,
                                action_count: action_count_to_send,
                                timestamp_ms: crate::network::protocol::now_ms(),
                                // Include debug fields when network_debug is enabled
                                client_state_hash: choice.client_state_hash,
                                debug_info: choice.debug_info,
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

        // Clone card_db for use in the blocking thread
        let card_db_clone = self.card_db.clone().expect("Card DB not loaded");

        // Run game loop in blocking thread
        // The game loop has a reveal drainer that will drain the queue before each draw
        let mut local_controller = local_controller;
        let mut remote_controller = remote_controller;
        let game_result = tokio::task::spawn_blocking(move || {
            // Track the last turn we blocked waiting for a draw reveal.
            // This prevents blocking multiple times in the same turn (once in draw_step,
            // again in priority_round after the draw has consumed the reveal).
            // Use AtomicU32 because the closure needs to be Sync.
            let last_draw_wait_turn = std::sync::atomic::AtomicU32::new(0);

            // Capture our_player_id for use in the drain_reveals closure
            let our_player_for_drain = our_player_id;

            // Create drain function that processes reveals from the channel
            // This function WAITS for reveals when OUR player's library is drawing
            // We only wait for our own draws - opponent draws are hidden and shouldn't reveal to us
            let drain_reveals = move |game: &mut GameState| {
                // Helper to process a single reveal
                // Now handles all reveal types by instantiating cards when needed
                //
                // HIDDEN INFO ARCHITECTURE (mtg-qtqcr):
                // - Real reveal: name is non-empty, instantiate the card
                // - Dummy reveal: name is empty, opponent's hidden card - don't instantiate
                let process_reveal =
                    |game: &mut GameState, owner: PlayerId, card_reveal: CardReveal, reason: RevealReason| {
                        let card_id = card_reveal.card_id;

                        // Check for dummy reveal (empty name = hidden card)
                        if card_reveal.name.is_empty() {
                            log::debug!(
                                "Dummy reveal: CardID {} for {:?} ({:?}) - skipping instantiation",
                                card_id.as_u32(),
                                owner,
                                reason
                            );
                            return; // Nothing to instantiate for dummy reveals
                        }

                        match reason {
                            RevealReason::Draw => {
                                // Late-binding architecture: instantiate the card now that we know its identity
                                // The CardID slot was reserved via init_game_reserve_only(), we need to bind
                                // the card identity when revealed via draw.
                                let card_already_known = game.cards.get(card_id).is_ok();
                                log::debug!(
                                    "RevealReason::Draw: {} (id={}) for {:?} card_already_known={}",
                                    card_reveal.name,
                                    card_id.as_u32(),
                                    owner,
                                    card_already_known
                                );

                                if !card_already_known {
                                    let card_def = get_card_def_from_reveal(&card_reveal, &card_db_clone);
                                    let card_instance = card_def.instantiate(card_id, owner);
                                    game.cards.insert(card_id, card_instance);
                                    log::debug!(
                                        "Instantiated drawn card for {:?}: {} ({:?})",
                                        owner,
                                        card_reveal.name,
                                        card_id
                                    );
                                }
                            }
                            RevealReason::Played => {
                                // A card was played - this could be:
                                // 1. From a known hand (opening hand or drawn card) - card already in game.cards
                                // 2. From a hidden hand (opponent's late-drawn card) - need to instantiate
                                //
                                // IMPORTANT: The CardRevealed message may arrive AFTER the OpponentChoice
                                // has already been processed and the card has been played. So we should NOT
                                // try to add the card to the hand here - that would cause double-counting.
                                //
                                // The only case where we need to add to hand is when the card isn't in the game
                                // yet (hidden hand card) - but in that case, we need to handle it BEFORE the
                                // play happens, not after. So this case will be handled elsewhere.

                                let card_already_known = game.cards.get(card_id).is_ok();
                                log::debug!(
                                    "RevealReason::Played: {} (id={}) card_already_known={}",
                                    card_reveal.name,
                                    card_id.as_u32(),
                                    card_already_known
                                );

                                // Just instantiate the card if not already known
                                if !card_already_known {
                                    // Use fallback that creates minimal definition if DB lookup fails
                                    let card_def = get_card_def_from_reveal(&card_reveal, &card_db_clone);
                                    let card_instance = card_def.instantiate(card_id, owner);
                                    game.cards.insert(card_id, card_instance); // Safe: checked above
                                    log::debug!(
                                        "Instantiated played card for {:?}: {} ({:?})",
                                        owner,
                                        card_reveal.name,
                                        card_id
                                    );

                                    // If we just instantiated a card and it's not in hand or battlefield yet,
                                    // then we need to add it to hand for the play to work
                                    let card_in_hand =
                                        game.get_player_zones(owner).is_some_and(|z| z.hand.contains(card_id));
                                    let card_on_battlefield = game.battlefield.cards.contains(&card_id);

                                    if !card_in_hand && !card_on_battlefield {
                                        // Card not found anywhere - add it to hand
                                        // The hidden_card_count mechanism is removed in the new architecture
                                        if let Some(zones) = game.get_player_zones_mut(owner) {
                                            zones.hand.add(card_id);
                                            log::debug!(
                                                "Added revealed card to hand: {} (id={})",
                                                card_reveal.name,
                                                card_id.as_u32()
                                            );
                                        }
                                    }
                                }
                                // For cards already in game.cards, we don't need to do anything
                                // The card was already known and has already been played
                            }
                            RevealReason::TokenCreated => {
                                // Token created - instantiate and add to battlefield
                                // Use fallback that creates minimal definition if DB lookup fails
                                let card_def = get_card_def_from_reveal(&card_reveal, &card_db_clone);
                                let card_instance = card_def.instantiate(card_id, owner);
                                if game.cards.insert_if_vacant(card_id, card_instance) {
                                    game.battlefield.add(card_id);
                                    log::debug!("Created token for {:?}: {} ({:?})", owner, card_reveal.name, card_id);
                                }
                            }
                            _ => {
                                // Other reveal reasons (Effect, Searched, etc.) - just track the card
                                log::debug!(
                                    "Received {:?} reveal for {:?}: {} ({:?})",
                                    reason,
                                    owner,
                                    card_reveal.name,
                                    card_id
                                );
                            }
                        }
                    };

                // Check if we need to wait for a draw reveal
                // IMPORTANT: Only block-wait when we're SPECIFICALLY in the Draw step.
                // drain_reveals is called at every priority check (priority.rs:208) AND at
                // the draw step (steps.rs:197). We should only block at the draw step.
                //
                // The issue: After syncing to an action_count, the client's GameLoop runs
                // ahead through automatic phases. If we block at priority checks waiting
                // for a reveal, we deadlock because the server hasn't drawn yet.
                let active_player = game.turn.active_player;
                let turn_number = game.turn.turn_number;
                let current_step = game.turn.current_step;

                // Only block-wait if:
                // 1. Turn > 1 (drawing happens on subsequent turns)
                // 2. We're specifically in the Draw step (not at a priority check)
                // 3. Active player is US (we don't wait for opponent draws - hidden info!)
                // 4. Our library is remote (we need a reveal)
                // 5. No reveal is already pending (we need to wait for one)
                // 6. We haven't already waited for a draw reveal THIS turn
                //    (prevents blocking again in priority_round after draw_card consumed the reveal)
                let is_our_turn = active_player == our_player_for_drain;

                // TODO(mtg-qtqcr Phase 3): In late-binding architecture, we don't have
                // remote libraries or pending reveals. Cards are always in library.cards,
                // and their identities are revealed via RevealCard actions.
                // For now, disable the blocking logic for draws.
                let library_is_remote = false;
                let has_pending_reveal = true; // Always consider reveals pending to skip blocking

                let already_waited_this_turn =
                    last_draw_wait_turn.load(std::sync::atomic::Ordering::Relaxed) == turn_number;

                let should_block = turn_number > 1
                    && current_step == Step::Draw
                    && is_our_turn  // CRITICAL: Only wait for our own draws!
                    && library_is_remote
                    && !has_pending_reveal
                    && !already_waited_this_turn;

                if should_block {
                    // We're in the Draw step about to draw from a remote library with no
                    // pending reveal. We CANNOT block here because:
                    // 1. The game thread must be able to respond to ChoiceRequests
                    // 2. The server sends draw reveals AFTER the draw, not before
                    // 3. Blocking would cause a deadlock if the server is waiting for us
                    //
                    // TODO(mtg-qtqcr Phase 3): In late-binding architecture, reveals are
                    // handled via RevealCard GameAction, not the old HiddenDraw mechanism.
                    // The reveal will arrive eventually via the WebSocket handler.
                    log::debug!(
                        "Turn {}: Checking for draw reveal for {:?} (remote library, no pending reveal)",
                        turn_number,
                        active_player
                    );

                    // Mark that we've checked for a reveal this turn
                    last_draw_wait_turn.store(turn_number, std::sync::atomic::Ordering::Relaxed);

                    // Brief poll - don't block! The reveal may not be here yet.
                    // We'll try again before the actual draw in draw_card.
                }

                // Non-blocking drain: get all reveals currently in the channel
                while let Ok((owner, card_reveal, reason)) = reveal_rx.try_recv() {
                    process_reveal(game, owner, card_reveal, reason);
                }
            };

            // Process any reveals before starting (for opening hand)
            // Note: On turn 1, the game loop skips drawing, so we just non-blocking drain
            // whatever reveals are available (they won't be needed for turn 1)
            drain_reveals(&mut game);

            // Create game loop with reveal drainer hook
            // Skip opening hand setup since server already drew hands and sent reveals
            // Enable reveal validation only when network_debug is enabled (gated by flag)
            // Pass our_player_id so validation skips opponent's hidden cards
            let mut game_loop = GameLoop::new(&mut game)
                .with_reveal_drainer(drain_reveals)
                .with_reveal_validation(our_player_id, network_debug)
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
        //
        // BIASED SELECT: Prioritize server_end over game_result to ensure we get
        // the authoritative winner from the server. Without biased, when both are
        // ready at the same time, select! picks pseudo-randomly, and we might
        // take game_result's Err path and fail to receive the winner.
        tokio::select! {
            biased;

            server_end = game_end_rx.recv() => {
                ws_handler.abort();
                match server_end {
                    Some((winner, server_action_count)) => {
                        log::info!("Server signaled game end: winner={:?}, server_action_count={}", winner, server_action_count);
                        // Note: Client GameLoop was aborted, so we can't compare action_counts here
                        // The per-choice verification during the game should catch sync issues
                        Ok(winner)
                    }
                    None => {
                        // Channel closed without sending winner - check if fatal error
                        // Wait a short time for fatal error to arrive (race condition mitigation)
                        match tokio::time::timeout(
                            std::time::Duration::from_millis(100),
                            fatal_error_rx.recv()
                        ).await {
                            Ok(Some(msg)) => {
                                log::error!("Game terminated due to fatal error: {}", msg);
                                Err(anyhow!("Fatal server error: {}", msg))
                            }
                            _ => {
                                // No fatal error, treat as normal termination (draw)
                                Ok(None)
                            }
                        }
                    }
                }
            }
            // Game loop completed (either normally or with error)
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
                        let error_msg = e.to_string();
                        if error_msg.contains("Game exit requested") {
                            // With biased select, server_end should normally win.
                            // If we get here, the game_end message wasn't sent (edge case).
                            // Try once more to receive it.
                            match game_end_rx.try_recv() {
                                Ok((winner, server_action_count)) => {
                                    log::info!(
                                        "Game ended via ExitGame fallback: winner={:?}, server_action_count={}",
                                        winner, server_action_count
                                    );
                                    Ok(winner)
                                }
                                Err(_) => {
                                    log::warn!("Game exited but no winner received from server (treating as draw)");
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
            // Fatal error from WebSocket handler
            error_msg = fatal_error_rx.recv() => {
                ws_handler.abort();
                match error_msg {
                    Some(msg) => {
                        log::error!("Game terminated due to fatal error: {}", msg);
                        Err(anyhow!("Fatal server error: {}", msg))
                    }
                    None => {
                        // Error channel closed without message - shouldn't happen
                        Ok(None)
                    }
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
            Some("Alice".to_string()),
            PathBuf::from("deck.dck"),
        );

        assert_eq!(config.server, "localhost:17771");
        assert_eq!(config.password, "secret");
        assert_eq!(config.player_name, Some("Alice".to_string()));
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
