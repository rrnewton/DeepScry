//! WebSocket client for multiplayer MTG
//!
//! Implements a client that:
//! - Connects to game server over WebSocket
//! - Maintains shadow game state with remote libraries
//! - Processes server messages and syncs game state
//! - Proxies choices to local PlayerController
//!
//! ## Module Organization
//!
//! The main `run_game` function is factored into helper functions:
//! - `run_ws_handler` - WebSocket message loop (runs in tokio task)
//! - `handle_server_message` - Processes individual server messages
//! - `handle_choice_submission` - Sends client choices to server

// Network code has specific patterns that trigger these lints
#![allow(clippy::large_enum_variant)] // NetworkMessage has intentionally large variants
#![allow(clippy::clone_on_ref_ptr)] // Arc::clone() pattern is verbose, .clone() is fine
#![allow(clippy::new_without_default)] // SharedNetworkState::new() has specific semantics
#![allow(clippy::missing_panics_doc)] // Internal network code, panics are for poisoned mutexes

use crate::core::{CardId, PlayerId};
use crate::game::{GameState, PlayerController, VerbosityLevel};
use crate::loader::{AsyncCardDatabase, CardDefinition, DeckList};
use crate::network::protocol::{CardReveal, ChoiceType, ClientMessage, DeckSubmission, RevealReason, ServerMessage};
use anyhow::{anyhow, Result};
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};

// ═══════════════════════════════════════════════════════════════════════════
// SINGLE-CHANNEL NETWORK MESSAGE
// ═══════════════════════════════════════════════════════════════════════════

/// All messages from WebSocket to the game loop via a single channel.
///
/// The single-channel architecture routes ALL server messages through one channel.
/// This eliminates race conditions by ensuring messages are processed in the exact
/// order they were received from the server.
///
/// Both NetworkLocalController and RemoteController share access to the same channel
/// receiver. When a controller needs input, it reads from the channel:
/// - CardRevealed: Process immediately (update game state), continue waiting
/// - ChoiceRequest: Return to NetworkLocalController
/// - ChoiceAccepted: Return to NetworkLocalController
/// - OpponentChoice: Return to RemoteController
/// - GameEnded/Error: Return to either controller (triggers game exit)
///
/// This design follows docs/NETWORK_ARCHITECTURE.md:
/// - No select! over multiple channels
/// - No try_recv() polling
/// - Sequential processing in arrival order
#[derive(Debug, Clone)]
pub enum NetworkMessage {
    /// A card was revealed by the server - instantiate it in game state
    CardRevealed {
        owner: PlayerId,
        card: CardReveal,
        reason: RevealReason,
    },
    /// Server is requesting a choice from us
    ChoiceRequest {
        action_count: u64,
        choice_seq: u32,
        /// Server's authoritative abilities for Priority choices (eliminates race conditions)
        abilities: Option<Vec<Option<crate::core::SpellAbility>>>,
        /// Server's unique card names for LibrarySearchByName choices
        /// Used when client can't compute valid_cards due to hidden card identities
        library_search_names: Option<Vec<String>>,
        /// Count of cards for each unique name (enables random instance selection)
        library_search_counts: Option<Vec<usize>>,
    },
    /// Server acknowledged our previous choice
    ChoiceAccepted {
        choice_seq: u32,
        action_count: u64,
        /// The CardId chosen for library search operations (for local player)
        library_search_result: Option<CardId>,
    },
    /// Opponent made a choice
    OpponentChoice {
        action_count: u64,
        choice_indices: Vec<usize>,
        description: String,
        spell_ability: Option<crate::core::SpellAbility>,
        /// The CardId chosen for library search operations
        library_search_result: Option<CardId>,
    },
    /// Game has ended
    GameEnded {
        winner: Option<PlayerId>,
        action_count: u64,
    },
    /// Server reported an error (fatal = should exit)
    Error { message: String, fatal: bool },
    /// Library was reordered (informational)
    LibraryReordered { player: PlayerId, new_order: Vec<CardId> },
}

impl NetworkMessage {
    /// Convert a ServerMessage to NetworkMessage, returning None for irrelevant messages
    pub fn from_server_message(msg: ServerMessage) -> Option<Self> {
        match msg {
            ServerMessage::CardRevealed { owner, card, reason } => {
                Some(NetworkMessage::CardRevealed { owner, card, reason })
            }
            ServerMessage::ChoiceRequest {
                action_count,
                choice_seq,
                abilities,
                choice_type,
                ..
            } => {
                // Extract library_search_names and counts from LibrarySearchByName choice type
                #[allow(clippy::wildcard_enum_match_arm)] // Intentionally match only one variant
                let (library_search_names, library_search_counts) = match choice_type {
                    crate::network::protocol::ChoiceType::LibrarySearchByName {
                        unique_names,
                        name_counts,
                        ..
                    } => (Some(unique_names), Some(name_counts)),
                    _ => (None, None),
                };
                Some(NetworkMessage::ChoiceRequest {
                    action_count,
                    choice_seq,
                    abilities,
                    library_search_names,
                    library_search_counts,
                })
            }
            ServerMessage::ChoiceAccepted {
                choice_seq,
                action_count,
                library_search_result,
                ..
            } => Some(NetworkMessage::ChoiceAccepted {
                choice_seq,
                action_count,
                library_search_result,
            }),
            ServerMessage::OpponentChoice {
                choice_indices,
                description,
                spell_ability,
                library_search_result,
                action_count,
                ..
            } => Some(NetworkMessage::OpponentChoice {
                action_count,
                choice_indices,
                description,
                spell_ability,
                library_search_result,
            }),
            ServerMessage::GameEnded {
                winner, action_count, ..
            } => Some(NetworkMessage::GameEnded { winner, action_count }),
            ServerMessage::Error { message, fatal } => Some(NetworkMessage::Error { message, fatal }),
            ServerMessage::LibraryReordered { player, new_order } => {
                Some(NetworkMessage::LibraryReordered { player, new_order })
            }
            // Ignore connection/setup messages - handled during connection setup, not gameplay
            ServerMessage::AuthResult { .. }
            | ServerMessage::WaitingForOpponent
            | ServerMessage::GameStarted { .. }
            | ServerMessage::SyncError { .. }
            | ServerMessage::Pong { .. } => None,
            // DebugStateDump only exists in debug builds
            #[cfg(debug_assertions)]
            ServerMessage::DebugStateDump { .. } => None,
        }
    }
}

/// Sender for client messages to the WebSocket writer task
pub type ClientMessageSender = mpsc::Sender<ClientMessage>;

// ═══════════════════════════════════════════════════════════════════════════
// SHARED NETWORK STATE (MVar Pattern for Choice Synchronization)
// ═══════════════════════════════════════════════════════════════════════════

/// Choice information from the network for the LOCAL player
///
/// Represents a ChoiceRequest (server asking us to make a choice).
/// Used by NetworkLocalController via the local_choice_mvar.
#[derive(Debug, Clone)]
pub enum LocalChoiceInfo {
    /// Server is requesting a choice from us
    Request {
        action_count: u64,
        choice_seq: u32,
        /// Server's authoritative abilities for Priority choices (eliminates race conditions)
        abilities: Option<Vec<Option<crate::core::SpellAbility>>>,
        /// Server's unique card names for LibrarySearchByName choices
        /// Used when client can't compute valid_cards due to hidden card identities
        library_search_names: Option<Vec<String>>,
        /// Count of cards for each unique name (enables random instance selection)
        library_search_counts: Option<Vec<usize>>,
    },
    /// Game ended - exit the game loop
    Exit { winner: Option<PlayerId> },
    /// Fatal error - exit with error
    Error { message: String },
}

/// Choice information from the network for the REMOTE player (opponent)
///
/// Represents an OpponentChoice (server telling us what opponent chose).
/// Used by RemoteController via the remote_choice_mvar.
#[derive(Debug, Clone)]
pub enum RemoteChoiceInfo {
    /// Opponent made a choice - use these indices
    Opponent {
        action_count: u64,
        indices: Vec<usize>,
        spell_ability: Option<crate::core::SpellAbility>,
        /// The CardId chosen for library search operations
        library_search_result: Option<CardId>,
    },
    /// Game ended - exit the game loop
    Exit { winner: Option<PlayerId> },
    /// Fatal error - exit with error
    Error { message: String },
}

/// ChoiceAccepted information for library search result synchronization
///
/// When the local player makes a LibrarySearchByName choice, the server responds
/// with ChoiceAccepted containing the actual CardId chosen. This allows the
/// client's shadow game to know which specific card was moved.
#[derive(Debug, Clone)]
pub enum ChoiceAcceptedInfo {
    /// Server accepted our choice, optionally with library search result
    Accepted {
        choice_seq: u32,
        /// The CardId chosen for library search operations (only set for LibrarySearchByName)
        library_search_result: Option<CardId>,
    },
    /// Game ended while waiting for ChoiceAccepted
    Exit { winner: Option<PlayerId> },
    /// Fatal error while waiting for ChoiceAccepted
    Error { message: String },
}

/// Legacy ChoiceInfo for backward compatibility
///
/// DEPRECATED: Use LocalChoiceInfo and RemoteChoiceInfo instead.
/// This enum is kept for the transition period.
#[derive(Debug, Clone)]
pub enum ChoiceInfo {
    /// Server is requesting a choice from us
    Request { action_count: u64, choice_seq: u32 },
    /// Opponent made a choice - use these indices
    Opponent {
        action_count: u64,
        indices: Vec<usize>,
        spell_ability: Option<crate::core::SpellAbility>,
        library_search_result: Option<CardId>,
    },
    /// Game ended - exit the game loop
    Exit { winner: Option<PlayerId> },
    /// Fatal error - exit with error
    Error { message: String },
}

/// Pending reveal for sync callback processing
///
/// Contains a CardRevealed message along with its associated action count
/// for deterministic processing.
#[derive(Debug, Clone)]
pub struct PendingReveal {
    pub action_count: u64,
    pub owner: PlayerId,
    pub card: CardReveal,
    pub reason: RevealReason,
}

/// Shared network state for synchronization between network loop and game loop
///
/// This structure implements queued choice synchronization using the MVar pattern:
/// - `pending_reveals`: Queue of CardRevealed messages keyed by action count
/// - `local_choice_mvar`: MVar for ChoiceRequest messages (local player)
/// - `remote_choice_mvar`: MVar for OpponentChoice messages (remote player)
/// - `server_action_count`: Latest action count from server (for sync targeting)
///
/// The network event loop populates these, and the game loop/controllers consume them.
/// Choices are queued because the network can receive multiple before the game loop
/// consumes them (e.g., opponent passes, then our turn starts).
///
/// ## Two-MVar Architecture
///
/// We use SEPARATE MVars for local and remote choices because:
/// - Controllers alternate based on who has priority
/// - If both use the same MVar, the wrong controller can take the wrong message
/// - E.g., LocalController takes OpponentChoice meant for RemoteController
///
/// With separate MVars:
/// - `local_choice_mvar` ← ChoiceRequest (server asking us to choose)
/// - `remote_choice_mvar` ← OpponentChoice (server telling us opponent's choice)
/// - GameEnded/Error go to BOTH MVars (either controller might be waiting)
///
/// ## Sync Target
///
/// `server_action_count` is updated whenever we receive a ChoiceRequest or OpponentChoice.
/// The sync_callback should sync reveals up to this value (not the client's current count)
/// to ensure the client's game state matches the server's before making choices.
pub struct SharedNetworkState {
    /// Pending reveals for sync callback processing
    /// Keyed by action count for deterministic processing
    pending_reveals: std::sync::Mutex<std::collections::VecDeque<PendingReveal>>,

    /// MVar for local player choice requests (ChoiceRequest messages)
    local_choice_mvar: super::mvar::MVar<LocalChoiceInfo>,

    /// MVar for remote player choices (OpponentChoice messages)
    remote_choice_mvar: super::mvar::MVar<RemoteChoiceInfo>,

    /// MVar for ChoiceAccepted responses (for library search result synchronization)
    /// Used by NetworkLocalController to receive library_search_result after LibrarySearchByName
    choice_accepted_mvar: super::mvar::MVar<ChoiceAcceptedInfo>,

    /// Latest action count from server (updated on ChoiceRequest/OpponentChoice)
    /// Used as sync target to ensure client processes all reveals before choices
    server_action_count: std::sync::atomic::AtomicU64,
}

impl SharedNetworkState {
    /// Create a new shared network state
    pub fn new() -> Self {
        Self {
            pending_reveals: std::sync::Mutex::new(std::collections::VecDeque::new()),
            local_choice_mvar: super::mvar::MVar::new(),
            remote_choice_mvar: super::mvar::MVar::new(),
            choice_accepted_mvar: super::mvar::MVar::new(),
            server_action_count: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// Get the latest server action count (sync target)
    ///
    /// This is the action count from the most recent ChoiceRequest or OpponentChoice.
    /// Use this as the target for sync_callback to ensure all reveals are processed.
    pub fn server_action_count(&self) -> u64 {
        self.server_action_count.load(std::sync::atomic::Ordering::Acquire)
    }

    /// Update the server action count
    ///
    /// Called by WS reader when receiving ChoiceRequest or OpponentChoice.
    fn update_server_action_count(&self, action_count: u64) {
        self.server_action_count
            .store(action_count, std::sync::atomic::Ordering::Release);
    }

    /// Push a pending reveal (called by network loop)
    pub fn push_reveal(&self, action_count: u64, owner: PlayerId, card: CardReveal, reason: RevealReason) {
        let mut queue = self.pending_reveals.lock().unwrap();
        queue.push_back(PendingReveal {
            action_count,
            owner,
            card,
            reason,
        });
    }

    /// Process pending reveals up to target action count (called by sync callback)
    ///
    /// Returns reveals that should be processed, removing them from the queue.
    /// Only returns reveals with action_count <= target.
    #[allow(dead_code)] // May be used for stricter sync validation in the future
    pub fn drain_reveals_up_to(&self, target: u64) -> Vec<PendingReveal> {
        let mut queue = self.pending_reveals.lock().unwrap();
        let mut result = Vec::new();

        // Drain all reveals up to target action count
        while queue.front().map(|r| r.action_count <= target).unwrap_or(false) {
            result.push(queue.pop_front().unwrap());
        }

        result
    }

    /// Drain ALL pending reveals (greedy mode)
    ///
    /// Used when we need to process all reveals regardless of action_count.
    /// This avoids race conditions where the WS reader hasn't yet updated
    /// server_action_count when sync_callback is called.
    ///
    /// Safe because:
    /// - Server sends CardRevealed BEFORE ChoiceRequest
    /// - Client shadow game runs deterministically, reaching the same game states
    /// - Reveals only arrive when they're actually needed
    pub fn drain_all_reveals(&self) -> Vec<PendingReveal> {
        let mut queue = self.pending_reveals.lock().unwrap();
        queue.drain(..).collect()
    }

    /// Push a local choice request (ChoiceRequest from server)
    pub fn push_local_choice(&self, choice: LocalChoiceInfo) {
        self.local_choice_mvar.put(choice);
    }

    /// Push a remote choice (OpponentChoice from server)
    pub fn push_remote_choice(&self, choice: RemoteChoiceInfo) {
        self.remote_choice_mvar.put(choice);
    }

    /// Take the next local choice from MVar (called by NetworkLocalController)
    ///
    /// Blocks until a choice is available, then consumes it.
    /// Returns None only if exit has been signaled and MVar is empty.
    pub fn take_local_choice(&self) -> Option<LocalChoiceInfo> {
        self.local_choice_mvar.take()
    }

    /// Take the next remote choice from MVar (called by RemoteController)
    ///
    /// Blocks until a choice is available, then consumes it.
    /// Returns None only if exit has been signaled and MVar is empty.
    pub fn take_remote_choice(&self) -> Option<RemoteChoiceInfo> {
        self.remote_choice_mvar.take()
    }

    /// Push a ChoiceAccepted response (called by WS reader)
    ///
    /// Used to communicate library_search_result back to NetworkLocalController
    /// after a LibrarySearchByName choice.
    pub fn push_choice_accepted(&self, info: ChoiceAcceptedInfo) {
        self.choice_accepted_mvar.put(info);
    }

    /// Take the next ChoiceAccepted response (called by NetworkLocalController)
    ///
    /// Blocks until ChoiceAccepted is available, then consumes it.
    /// Returns None only if exit has been signaled and MVar is empty.
    pub fn take_choice_accepted(&self) -> Option<ChoiceAcceptedInfo> {
        self.choice_accepted_mvar.take()
    }

    /// Take ChoiceAccepted for a specific choice_seq, discarding stale messages
    ///
    /// The MVar may contain ChoiceAccepted messages from previous choices that weren't
    /// consumed (because only library searches need to wait for ChoiceAccepted).
    /// This method loops until it finds the matching choice_seq or encounters Exit/Error.
    pub fn take_choice_accepted_for_seq(&self, expected_seq: u32) -> Option<ChoiceAcceptedInfo> {
        loop {
            match self.choice_accepted_mvar.take() {
                Some(ChoiceAcceptedInfo::Accepted {
                    choice_seq,
                    library_search_result,
                }) => {
                    if choice_seq == expected_seq {
                        // Found the matching ChoiceAccepted
                        return Some(ChoiceAcceptedInfo::Accepted {
                            choice_seq,
                            library_search_result,
                        });
                    }
                    // Stale message from previous choice - discard and continue
                    log::debug!(
                        "[ClientSharedState] Discarding stale ChoiceAccepted: got seq={}, expected seq={}",
                        choice_seq,
                        expected_seq
                    );
                }
                Some(exit_or_error) => {
                    // Exit or Error - return immediately
                    return Some(exit_or_error);
                }
                None => {
                    // MVar returned None (exit signaled and empty)
                    return None;
                }
            }
        }
    }

    /// Signal that the game should exit (notifies ALL MVars)
    pub fn signal_exit(&self) {
        self.local_choice_mvar.signal_exit();
        self.remote_choice_mvar.signal_exit();
        self.choice_accepted_mvar.signal_exit();
    }

    /// Check if exit has been signaled
    pub fn should_exit(&self) -> bool {
        // Check either MVar - they should be in sync
        self.local_choice_mvar.is_exit_signaled()
    }

    // Legacy methods for backward compatibility (DEPRECATED)

    /// Push a choice to the MVar (called by network loop)
    ///
    /// DEPRECATED: Use push_local_choice or push_remote_choice instead.
    #[allow(dead_code)]
    pub fn push_choice(&self, choice: ChoiceInfo) {
        match choice {
            ChoiceInfo::Request {
                action_count,
                choice_seq,
            } => {
                self.push_local_choice(LocalChoiceInfo::Request {
                    action_count,
                    choice_seq,
                    abilities: None,             // Legacy path doesn't have abilities
                    library_search_names: None,  // Legacy path doesn't have library search names
                    library_search_counts: None, // Legacy path doesn't have counts
                });
            }
            ChoiceInfo::Opponent {
                action_count,
                indices,
                spell_ability,
                library_search_result,
            } => {
                self.push_remote_choice(RemoteChoiceInfo::Opponent {
                    action_count,
                    indices,
                    spell_ability,
                    library_search_result,
                });
            }
            ChoiceInfo::Exit { winner } => {
                self.push_local_choice(LocalChoiceInfo::Exit { winner });
                self.push_remote_choice(RemoteChoiceInfo::Exit { winner });
            }
            ChoiceInfo::Error { message } => {
                self.push_local_choice(LocalChoiceInfo::Error {
                    message: message.clone(),
                });
                self.push_remote_choice(RemoteChoiceInfo::Error { message });
            }
        }
    }

    /// Take the next choice from MVar (called by controller)
    ///
    /// DEPRECATED: Use take_local_choice or take_remote_choice instead.
    /// This method only returns local choices for backward compatibility.
    #[allow(dead_code)]
    pub fn take_choice(&self) -> Option<ChoiceInfo> {
        self.take_local_choice().map(|local| match local {
            LocalChoiceInfo::Request {
                action_count,
                choice_seq,
                ..  // Ignore abilities and library_search_names for legacy ChoiceInfo
            } => ChoiceInfo::Request {
                action_count,
                choice_seq,
            },
            LocalChoiceInfo::Exit { winner } => ChoiceInfo::Exit { winner },
            LocalChoiceInfo::Error { message } => ChoiceInfo::Error { message },
        })
    }
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
    /// Serialized RNG state from server for deterministic shuffles
    pub rng_state: Vec<u8>,
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
                    // Opponent plays a card FROM hand - CardID slot was pre-reserved.
                    // We only instantiate the card so it can be recognized when the GameLoop
                    // executes the action. We do NOT add it to hand - the card is being
                    // played FROM hand, and the GameLoop will move it to stack/battlefield.
                    if !self.game.cards.is_revealed(card.card_id) {
                        self.game.cards.insert(card.card_id, card_instance);
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
            rng_state,         // Serialized RNG state for deterministic shuffles
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
                    rng_state,
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
                    if !rng_state.is_empty() {
                        log::debug!("Received RNG state from server ({} bytes)", rng_state.len());
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
                        rng_state,
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

        // Mark as shadow game - tolerant of incomplete zone tracking for opponent's hidden zones.
        // The server is authoritative; this client state is approximate.
        game.set_shadow_game(true);

        // Initialize RNG from server's state for deterministic shuffles
        // This ensures subsequent shuffles (tutors, etc.) produce identical results
        if !rng_state.is_empty() {
            use rand_chacha::ChaCha12Rng;
            match bincode::deserialize::<ChaCha12Rng>(&rng_state) {
                Ok(rng) => {
                    *game.rng.borrow_mut() = rng;
                    log::info!("Initialized RNG from server state ({} bytes)", rng_state.len());
                }
                Err(e) => {
                    log::error!("Failed to deserialize RNG state: {} - shuffles may diverge!", e);
                }
            }
        } else {
            log::warn!("No RNG state received from server - shuffles may diverge!");
        }

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
    /// ## MVar Architecture
    ///
    /// This implementation uses SharedNetworkState for synchronization:
    ///
    /// ```text
    /// WebSocket ──► Reader Task ──► SharedNetworkState ◄── sync_callback (reveals)
    ///                                      │
    ///                                      ├── choice_mvar ◄── Controllers
    ///                                      │
    /// WebSocket ◄── Writer Task ◄── send_rx ◄── send_tx ◄── Controllers
    /// ```
    ///
    /// - **Reader task**: Routes messages to SharedNetworkState (reveals → queue, choices → MVar)
    /// - **Writer task**: Forwards client messages to WebSocket
    /// - **sync_callback**: Drains pending reveals from SharedNetworkState
    /// - **Controllers**: Read choices from MVar via SharedNetworkState
    ///
    /// # Errors
    ///
    /// Returns an error if not connected, game not started, or communication fails.
    pub async fn run_game<C: PlayerController + Send + 'static>(&mut self, controller: C) -> Result<Option<PlayerId>> {
        use crate::game::GameLoop;
        use crate::network::{NetworkLocalController, RemoteController};

        // Take ownership of WebSocket and state
        let ws = self.ws.take().ok_or_else(|| anyhow!("Not connected"))?;
        let client_state = self.state.take().ok_or_else(|| anyhow!("Game not started"))?;
        let our_player_id = client_state.our_player_id;
        let opponent_id = client_state.opponent_id;
        let we_are_p1 = our_player_id.as_u32() == 0;

        // Split WebSocket for concurrent read/write
        let (ws_sink, ws_stream) = ws.split();

        // Shared network state for synchronization (MVar pattern)
        let shared_state = Arc::new(SharedNetworkState::new());

        // Channel for outbound messages (Controllers → WebSocket)
        let (send_tx, send_rx) = mpsc::channel::<ClientMessage>();

        let network_debug = self.network_debug;
        let card_db = self.card_db.clone().expect("Card DB not loaded");

        // Configure game state
        let mut game = client_state.game;
        if self.tag_gamelogs {
            game.logger.set_tag_gamelogs(true);
            log::debug!("Client GameLoop: tag_gamelogs enabled");
        }

        // Spawn reader task: WebSocket → SharedNetworkState
        let reader_state = shared_state.clone();
        let reader_card_db = card_db.clone();
        let reader_handle = tokio::spawn(run_ws_reader_shared(ws_stream, reader_state, reader_card_db));

        // Spawn writer task: send_rx → WebSocket
        let writer_handle = tokio::spawn(run_ws_writer(ws_sink, send_rx));

        // Clone for sync callback and controllers
        let sync_state = shared_state.clone();
        let controller_state = shared_state.clone();
        let card_db_for_sync = card_db.clone();

        // Run game loop in spawn_blocking (works with both single and multi-threaded runtimes)
        let game_result = tokio::task::spawn_blocking(move || {
            // Create controllers with shared state
            let mut local_controller = NetworkLocalController::new_with_shared_state(
                controller,
                send_tx.clone(),
                controller_state.clone(),
                our_player_id,
            )
            .with_network_debug(network_debug);

            let mut remote_controller = RemoteController::new_with_shared_state(opponent_id, controller_state.clone());

            // SYNC CALLBACK: Drains pending reveals from SharedNetworkState
            //
            // Called at synchronization points (before validation, after draws, etc.)
            // to ensure cards are instantiated before they're needed.
            //
            // GREEDY DRAINING: We drain ALL pending reveals, not just up to server_action_count.
            // This avoids a race condition where:
            //   1. Shadow game calls sync_to_action() BEFORE WS reader processes ChoiceRequest
            //   2. server_action_count is still stale from the previous ChoiceRequest
            //   3. Reveals for this turn's draw are not drained (tagged with stale action_count)
            //   4. Available abilities are computed without the drawn card's identity
            //   5. Desync occurs
            //
            // Greedy draining is safe because:
            // - Server sends CardRevealed BEFORE ChoiceRequest
            // - Client shadow game runs deterministically, reaching the same game states
            // - Reveals only arrive when they're actually needed
            let sync_callback = move |game: &mut GameState, _target_action: u64| {
                let game_action = game.undo_log.len() as u64;
                // Greedy: drain ALL pending reveals
                let reveals = sync_state.drain_all_reveals();

                if !reveals.is_empty() {
                    log::debug!(
                        "sync_callback: processing {} reveals (greedy mode, game_action={})",
                        reveals.len(),
                        game_action
                    );
                }

                for reveal in reveals {
                    log::trace!(
                        "sync_callback: processing reveal {} at tagged_action={} (game={})",
                        reveal.card.name,
                        reveal.action_count,
                        game_action
                    );
                    process_card_reveal(
                        game,
                        &card_db_for_sync,
                        reveal.owner,
                        reveal.card,
                        reveal.reason,
                        our_player_id,
                    );
                }
            };

            // Create game loop with sync callback (no pre-choice hook)
            // defer_game_end_check: Server is authoritative about game end - client waits for GameEnded
            // reveal_validation: Disabled for clients - validation timing is tricky with async reveals
            // and the server is authoritative anyway
            let result = {
                let mut game_loop = GameLoop::new(&mut game)
                    .with_sync_callback(sync_callback)
                    .with_reveal_validation(our_player_id, false) // Server is authoritative
                    .skip_opening_hands()
                    .with_deferred_game_end();

                // Pass controllers in the correct order based on which player we are
                log::debug!("Client GameLoop: we_are_p1={}", we_are_p1);
                if we_are_p1 {
                    game_loop.run_game(&mut local_controller, &mut remote_controller)
                } else {
                    game_loop.run_game(&mut remote_controller, &mut local_controller)
                }
            };

            result
        })
        .await;

        // Clean up tasks
        reader_handle.abort();
        writer_handle.abort();

        // Return result - handle both JoinError and game error
        match game_result {
            Ok(Ok(result)) => {
                log::info!(
                    "Client GameLoop finished: winner={:?}, action_count={}",
                    result.winner,
                    result.action_count
                );
                Ok(result.winner)
            }
            Ok(Err(e)) => {
                let error_msg = e.to_string();
                if error_msg.contains("Game exit requested") {
                    // Game ended normally via controller ExitGame
                    log::info!("Game ended via ExitGame");
                    Ok(None)
                } else {
                    Err(anyhow!("Game error: {}", e))
                }
            }
            Err(e) => {
                // JoinError - task panicked or was cancelled
                Err(anyhow!("Game thread panic: {}", e))
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SINGLE-CHANNEL WEBSOCKET TASKS
// ═══════════════════════════════════════════════════════════════════════════

/// WebSocket reader task using SharedNetworkState (MVar architecture)
///
/// This reader routes messages to the appropriate destination:
/// - CardRevealed → pending_reveals queue (for sync callback)
/// - ChoiceRequest/OpponentChoice → choice_mvar (for controllers)
/// - GameEnded/Error → signal exit and set choice_mvar
async fn run_ws_reader_shared(
    mut ws_stream: SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>,
    shared_state: Arc<SharedNetworkState>,
    _card_db: Arc<AsyncCardDatabase>,
) {
    // Track action_count from ChoiceAccepted/ChoiceRequest for reveal tagging
    // For now we use 0 (greedy draining) - can refine later
    let current_action_count = std::sync::atomic::AtomicU64::new(0);

    while let Some(msg_result) = ws_stream.next().await {
        match msg_result {
            Ok(Message::Text(text)) => match serde_json::from_str::<ServerMessage>(&text) {
                Ok(server_msg) => {
                    if let Some(network_msg) = NetworkMessage::from_server_message(server_msg) {
                        match network_msg {
                            NetworkMessage::CardRevealed { owner, card, reason } => {
                                // Route to pending reveals queue
                                let action = current_action_count.load(std::sync::atomic::Ordering::Relaxed);
                                log::debug!(
                                    "WsReaderShared: buffering reveal {} (id={}) for {:?} at action {}",
                                    card.name,
                                    card.card_id.as_u32(),
                                    owner,
                                    action
                                );
                                shared_state.push_reveal(action, owner, card, reason);
                            }
                            NetworkMessage::ChoiceRequest {
                                action_count,
                                choice_seq,
                                abilities,
                                library_search_names,
                                library_search_counts,
                            } => {
                                // Update tracked action count (for sync targeting)
                                current_action_count.store(action_count, std::sync::atomic::Ordering::Relaxed);
                                shared_state.update_server_action_count(action_count);
                                // Push to LOCAL choice MVar (for NetworkLocalController)
                                log::debug!(
                                    "WsReaderShared: ChoiceRequest seq={} action={} abilities={} lib_search={} -> local_mvar",
                                    choice_seq,
                                    action_count,
                                    abilities.as_ref().map(|a| a.len()).unwrap_or(0),
                                    library_search_names.as_ref().map(|n| n.len()).unwrap_or(0)
                                );
                                shared_state.push_local_choice(LocalChoiceInfo::Request {
                                    action_count,
                                    choice_seq,
                                    abilities,
                                    library_search_names,
                                    library_search_counts,
                                });
                            }
                            NetworkMessage::ChoiceAccepted {
                                choice_seq,
                                action_count,
                                library_search_result,
                            } => {
                                // Update tracked action count
                                current_action_count.store(action_count, std::sync::atomic::Ordering::Relaxed);
                                log::debug!(
                                    "WsReaderShared: ChoiceAccepted seq={} action={} lib_search_result={:?}",
                                    choice_seq,
                                    action_count,
                                    library_search_result
                                );
                                // Push to choice_accepted_mvar for library search synchronization
                                shared_state.push_choice_accepted(ChoiceAcceptedInfo::Accepted {
                                    choice_seq,
                                    library_search_result,
                                });
                            }
                            NetworkMessage::OpponentChoice {
                                action_count,
                                choice_indices,
                                description: _,
                                spell_ability,
                                library_search_result,
                            } => {
                                // Update tracked action count (for sync targeting)
                                shared_state.update_server_action_count(action_count);
                                // Push to REMOTE choice MVar (for RemoteController)
                                log::debug!(
                                    "WsReaderShared: OpponentChoice indices={:?} action={} lib_search={:?} -> remote_mvar",
                                    choice_indices,
                                    action_count,
                                    library_search_result
                                );
                                shared_state.push_remote_choice(RemoteChoiceInfo::Opponent {
                                    action_count,
                                    indices: choice_indices,
                                    spell_ability,
                                    library_search_result,
                                });
                            }
                            NetworkMessage::GameEnded { winner, action_count } => {
                                log::info!(
                                    "WsReaderShared: Game ended, winner={:?}, action={}",
                                    winner,
                                    action_count
                                );
                                // Push to ALL MVars (any controller might be waiting)
                                shared_state.signal_exit();
                                shared_state.push_local_choice(LocalChoiceInfo::Exit { winner });
                                shared_state.push_remote_choice(RemoteChoiceInfo::Exit { winner });
                                shared_state.push_choice_accepted(ChoiceAcceptedInfo::Exit { winner });
                                return;
                            }
                            NetworkMessage::Error { message, fatal } => {
                                if fatal {
                                    log::error!("WsReaderShared: Fatal error: {}", message);
                                    // Push to ALL MVars (any controller might be waiting)
                                    shared_state.signal_exit();
                                    shared_state.push_local_choice(LocalChoiceInfo::Error {
                                        message: message.clone(),
                                    });
                                    shared_state.push_remote_choice(RemoteChoiceInfo::Error {
                                        message: message.clone(),
                                    });
                                    shared_state.push_choice_accepted(ChoiceAcceptedInfo::Error { message });
                                    return;
                                }
                                log::warn!("WsReaderShared: Non-fatal error: {}", message);
                            }
                            NetworkMessage::LibraryReordered { player, new_order } => {
                                log::debug!(
                                    "WsReaderShared: Library reordered for {:?}, {} cards",
                                    player,
                                    new_order.len()
                                );
                                // Informational only
                            }
                        }
                    }
                }
                Err(e) => {
                    log::error!("WsReaderShared: failed to parse server message: {}", e);
                }
            },
            Ok(Message::Close(_)) => {
                log::debug!("WsReaderShared: WebSocket closed by server");
                shared_state.signal_exit();
                shared_state.push_local_choice(LocalChoiceInfo::Exit { winner: None });
                shared_state.push_remote_choice(RemoteChoiceInfo::Exit { winner: None });
                return;
            }
            Ok(_) => {
                // Ignore binary/ping/pong
            }
            Err(e) => {
                log::error!("WsReaderShared: WebSocket error: {}", e);
                shared_state.signal_exit();
                shared_state.push_local_choice(LocalChoiceInfo::Error { message: e.to_string() });
                shared_state.push_remote_choice(RemoteChoiceInfo::Error { message: e.to_string() });
                return;
            }
        }
    }
    log::debug!("WsReaderShared: WebSocket stream ended");
    shared_state.signal_exit();
}

/// Legacy WebSocket reader task (with pending_reveals buffer)
///
/// DEPRECATED: Use run_ws_reader_shared instead with MVar architecture.
#[allow(dead_code)]
async fn run_ws_reader(
    mut ws_stream: SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>,
    recv_tx: mpsc::Sender<NetworkMessage>,
    pending_reveals: crate::network::local_controller::PendingReveals,
) {
    use crate::network::local_controller::BufferedReveal;

    while let Some(msg_result) = ws_stream.next().await {
        match msg_result {
            Ok(Message::Text(text)) => match serde_json::from_str::<ServerMessage>(&text) {
                Ok(server_msg) => {
                    if let Some(network_msg) = NetworkMessage::from_server_message(server_msg) {
                        // CardRevealed goes DIRECTLY to pending_reveals buffer
                        // This ensures reveals are processed before GameLoop validation
                        if let NetworkMessage::CardRevealed { owner, card, reason } = network_msg {
                            if let Ok(mut reveals) = pending_reveals.lock() {
                                log::debug!(
                                    "WsReader: buffering reveal {} (id={}) for {:?}",
                                    card.name,
                                    card.card_id.as_u32(),
                                    owner
                                );
                                reveals.push(BufferedReveal { owner, card, reason });
                            } else {
                                log::error!("WsReader: failed to lock pending_reveals");
                            }
                        } else {
                            // All other messages go to controller channel
                            if recv_tx.send(network_msg).is_err() {
                                log::debug!("WsReader: recv channel closed, exiting");
                                return;
                            }
                        }
                    }
                }
                Err(e) => {
                    log::error!("WsReader: failed to parse server message: {}", e);
                }
            },
            Ok(Message::Close(_)) => {
                log::debug!("WsReader: WebSocket closed by server");
                // Send a GameEnded message to unblock controllers
                let _ = recv_tx.send(NetworkMessage::GameEnded {
                    winner: None,
                    action_count: 0,
                });
                return;
            }
            Ok(_) => {
                // Ignore binary/ping/pong
            }
            Err(e) => {
                log::error!("WsReader: WebSocket error: {}", e);
                let _ = recv_tx.send(NetworkMessage::Error {
                    message: e.to_string(),
                    fatal: true,
                });
                return;
            }
        }
    }
    log::debug!("WsReader: WebSocket stream ended");
}

/// WebSocket writer task: reads from send_rx and forwards to WebSocket
///
/// This is a simple linear loop with no select! - just recv and forward.
/// Uses tokio mpsc for async receive to avoid blocking issues.
async fn run_ws_writer(
    mut ws_sink: SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>,
    send_rx: mpsc::Receiver<ClientMessage>,
) {
    // Wrap the std::sync::mpsc::Receiver in a tokio task to make it async
    // We use a tokio mpsc channel as a bridge
    let (bridge_tx, mut bridge_rx) = tokio::sync::mpsc::channel::<ClientMessage>(16);

    // Spawn a blocking task that reads from std channel and forwards to tokio channel
    let _bridge_task = tokio::task::spawn_blocking(move || {
        while let Ok(msg) = send_rx.recv() {
            // Use blocking_send since we're in a blocking context
            if bridge_tx.blocking_send(msg).is_err() {
                log::debug!("WsWriter bridge: tokio channel closed");
                return;
            }
        }
        log::debug!("WsWriter bridge: std channel closed");
    });

    // Now we can use async receive
    while let Some(client_msg) = bridge_rx.recv().await {
        let text = match serde_json::to_string(&client_msg) {
            Ok(t) => t,
            Err(e) => {
                log::error!("WsWriter: failed to serialize message: {}", e);
                continue;
            }
        };
        if let Err(e) = ws_sink.send(Message::Text(text.into())).await {
            log::error!("WsWriter: failed to send to WebSocket: {}", e);
            return;
        }
    }
    log::debug!("WsWriter: bridge channel closed, exiting");
}

// ═══════════════════════════════════════════════════════════════════════════
// GAME LOOP HELPERS
// ═══════════════════════════════════════════════════════════════════════════

/// Process a card reveal in the client's shadow game state
///
/// Wrapper around the shared `reveal_processor::process_card_reveal` that uses
/// the native card definition provider (with database fallback).
fn process_card_reveal(
    game: &mut GameState,
    card_db: &AsyncCardDatabase,
    owner: PlayerId,
    card_reveal: CardReveal,
    reason: RevealReason,
    local_player: PlayerId,
) {
    use super::reveal_processor::{process_card_reveal as shared_process, NativeCardDefProvider};

    let provider = NativeCardDefProvider::new(card_db);
    shared_process(
        game,
        &provider,
        owner,
        card_reveal,
        reason,
        "Native",
        Some(local_player),
    );
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
