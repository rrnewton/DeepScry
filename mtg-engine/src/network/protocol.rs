//! Network protocol message types
//!
//! Defines all messages exchanged between client and server.
//!
//! ## Global Ordering via action_count
//!
//! All messages include timing information for debugging synchronization issues:
//! - `action_count`: Length of the undo_log at a specific point in time.
//! - `timestamp_ms`: Wall-clock milliseconds since Unix epoch for debugging.
//!
//! ### What is action_count?
//!
//! The `action_count` is the length of the `GameState.undo_log`, which records
//! every `GameAction` that has been applied to the game state. Each action
//! (including ChoicePoint actions that record player decisions) increments the log.
//!
//! ### action_count semantics by message type
//!
//! **ChoiceRequest** (server → client):
//! - `action_count = D`: The server's undo_log has D entries BEFORE this choice is made.
//! - This is the server's authoritative sync point - client should validate their
//!   shadow state matches before making the choice.
//!
//! **SubmitChoice** (client → server):
//! - `action_count`: Client ECHOES the server's action_count from ChoiceRequest.
//! - This confirms the client was at the expected state when making the choice.
//! - Note: By the time client sends this, their GameLoop may have run ahead
//!   internally to `D + N` (where N ≥ 1), but they report the sync point D.
//!
//! **ChoiceAccepted** (server → client):
//! - `action_count`: Server's undo_log length AFTER applying the choice.
//! - Typically `D + 1` if only a ChoicePoint action was logged.
//! - Could be `D + N` if the choice triggered additional actions (e.g., casting
//!   a spell logs the spell resolution actions too).
//!
//! **OpponentChoice** (server → client):
//! - `action_count`: The server's undo_log length when this choice was made.
//! - Client uses this to validate their shadow state is synchronized.
//!
//! ### Example flow
//!
//! ```text
//! Server undo_log: [a1, a2, a3]  (length = 3)
//! Server sends: ChoiceRequest { action_count: 3, ... }
//!
//! Client shadow log: [a1, a2, a3]  (should match!)
//! Client makes choice, logs ChoicePoint to shadow: [a1, a2, a3, choice]
//! Client sends: SubmitChoice { action_count: 3, ... }  (echoes server's count)
//!
//! Server applies choice, logs ChoicePoint: [a1, a2, a3, choice]
//! Server sends: ChoiceAccepted { action_count: 4, ... }
//! ```
//!
//! ## Player Identification
//!
//! Messages explicitly identify which player they concern using `PlayerId`:
//! - P1 = PlayerId(0), P2 = PlayerId(1)
//! - ChoiceRequest includes `for_player` to identify who must respond
//! - OpponentChoice, CardRevealed include owner/player for context

use crate::core::{CardId, ManaCost, PlayerId, SpellAbility};
use crate::game::GameEndReason;
use serde::{Deserialize, Serialize};

// ═══════════════════════════════════════════════════════════════════════════
// GLOBAL TIMESTAMP UTILITIES
// ═══════════════════════════════════════════════════════════════════════════

/// Get current wall-clock timestamp in milliseconds since Unix epoch
#[cfg(not(target_arch = "wasm32"))]
pub fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Get current wall-clock timestamp in milliseconds since Unix epoch (WASM version)
#[cfg(target_arch = "wasm32")]
pub fn now_ms() -> u64 {
    js_sys::Date::now() as u64
}

// ═══════════════════════════════════════════════════════════════════════════
// CLIENT → SERVER MESSAGES
// ═══════════════════════════════════════════════════════════════════════════

/// Messages sent from client to server
///
/// Note: SubmitChoice variant is intentionally large (contains multiple Option<Vec<_>> fields
/// for various choice data). Boxing would complicate the protocol code for marginal benefit.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum ClientMessage {
    /// Initial authentication and deck submission
    Authenticate {
        /// Server password
        password: String,
        /// Player's display name (None = let server assign a default name with suffix)
        player_name: Option<String>,
        /// Deck to use for the game
        deck: DeckSubmission,
    },

    /// Response to a choice request from server
    SubmitChoice {
        /// Sequence number matching the ChoiceRequest
        choice_seq: u32,
        /// The chosen option indices (into the options array)
        ///
        /// For single-select choices (priority, targets), this is a 1-element vec.
        /// For multi-select choices (attackers, blockers), contains all selected indices.
        /// Index 0 typically means "done" or "pass" for multi-select choices.
        choice_indices: Vec<usize>,
        /// ECHOES the server's action_count from the ChoiceRequest
        ///
        /// Client sends back the same action_count it received in ChoiceRequest to
        /// confirm they were at the expected sync point when making this choice.
        /// Server validates this matches to detect sync drift early.
        #[serde(default)]
        action_count: u64,
        /// Wall-clock timestamp for debugging (ms since Unix epoch)
        #[serde(default)]
        timestamp_ms: u64,
        /// Client's computed state hash (for server validation in debug mode)
        /// When present, server compares against its expected hash
        client_state_hash: Option<u64>,
        /// Debug synchronization info (only in network debug mode)
        debug_info: Option<DebugSyncInfo>,
        /// The actual spell ability chosen (for Priority choices)
        ///
        /// When present, server uses this directly instead of looking up by index.
        /// This is more robust against client/server state divergence since it
        /// identifies the ability by CardId rather than position in a list.
        spell_ability: Option<SpellAbility>,
        /// Actual target CardIds for target choices
        ///
        /// When present, server forwards these to opponent in OpponentChoice.
        /// This ensures the opponent's shadow game uses the correct CardIds
        /// even if its valid_targets list differs.
        #[serde(default)]
        target_card_ids: Option<Vec<CardId>>,
    },

    /// Request to disconnect gracefully
    Disconnect,

    /// Keepalive ping
    Ping {
        /// Timestamp in milliseconds
        timestamp_ms: u64,
    },
}

/// Deck submission format for authentication
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeckSubmission {
    /// Main deck entries: (card_name, count)
    pub main_deck: Vec<(String, u8)>,
    /// Sideboard entries: (card_name, count)
    pub sideboard: Vec<(String, u8)>,
}

impl DeckSubmission {
    /// Create a new deck submission
    pub fn new(main_deck: Vec<(String, u8)>, sideboard: Vec<(String, u8)>) -> Self {
        Self { main_deck, sideboard }
    }

    /// Total cards in main deck
    pub fn main_deck_size(&self) -> usize {
        self.main_deck.iter().map(|(_, count)| *count as usize).sum()
    }

    /// Total cards in sideboard
    pub fn sideboard_size(&self) -> usize {
        self.sideboard.iter().map(|(_, count)| *count as usize).sum()
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SERVER → CLIENT MESSAGES
// ═══════════════════════════════════════════════════════════════════════════

/// Messages sent from server to client
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)] // ChoiceRequest is the hot path; boxing adds overhead
pub enum ServerMessage {
    /// Authentication result
    AuthResult {
        /// Whether authentication succeeded
        success: bool,
        /// Error message if failed
        error: Option<String>,
        /// Assigned player ID if successful
        your_player_id: Option<PlayerId>,
        /// Assigned player name (includes suffix if server-generated)
        your_name: Option<String>,
    },

    /// Waiting for opponent to connect
    WaitingForOpponent,

    /// Game is starting - includes initial setup info
    GameStarted {
        /// Your assigned player ID
        your_player_id: PlayerId,
        /// Opponent's display name
        opponent_name: String,
        /// Your opening hand (card IDs and definitions)
        opening_hand: Vec<CardReveal>,
        /// Number of cards in opponent's hand
        opponent_hand_count: usize,
        /// Your library size after drawing
        library_size: usize,
        /// Opponent library size after drawing (always visible per MTG rules)
        opponent_library_size: usize,
        /// Opponent's initial deck list (if deck_visibility enabled)
        /// This is the INITIAL list before sideboarding.
        opponent_decklist: Option<DeckListInfo>,
        /// Starting life total
        starting_life: i32,
        /// Initial game state hash for verification
        initial_state_hash: u64,
        /// Network debug mode - if true, clients should include state hashes
        /// in SubmitChoice and validate server hashes
        #[serde(default)]
        network_debug: bool,
        /// Deterministic CardID ranges for late-binding architecture (Phase 3)
        ///
        /// Contains the CardID ranges for both players' decks:
        /// - P1's deck gets CardIDs [0, p1_deck_size)
        /// - P2's deck gets CardIDs [p1_deck_size, p1_deck_size + p2_deck_size)
        ///
        /// This is PUBLIC information - everyone knows which CardIDs belong
        /// to which deck. Only the CardID ⟺ CardName binding is hidden until
        /// a RevealCard action makes it known.
        deck_card_ids: Option<DeckCardIdRanges>,
        /// Token definitions that may be created during this game.
        /// Sent upfront so clients can create tokens without a local card database.
        /// Key is the script name (e.g., "c_a_food_sac"), value is the CardDefinition.
        #[serde(default)]
        token_definitions: std::collections::HashMap<String, crate::loader::CardDefinition>,
        /// Serialized RNG state for deterministic game execution.
        ///
        /// The server sends its post-initial-shuffle RNG state so clients can
        /// initialize their RNG to match. This ensures subsequent shuffles
        /// (from tutors, etc.) produce identical results on both server and client.
        ///
        /// Uses bincode serialization of ChaCha12Rng (56 bytes).
        #[serde(default)]
        rng_state: Vec<u8>,
    },

    /// Card reveal event (draws, tutors, plays, etc.)
    CardRevealed {
        /// Who the card belongs to
        owner: PlayerId,
        /// The revealed card info
        card: CardReveal,
        /// Why this card is being revealed
        reason: RevealReason,
    },

    /// Library has been reordered (shuffled after search)
    ///
    /// Sent after a library search + shuffle to update the client's shadow
    /// state with the new CardId order. Card identities remain hidden;
    /// only the chosen card is revealed via CardRevealed.
    ///
    /// ## Late-binding architecture
    /// The client's library zone contains CardIds without known identities.
    /// This message updates the order of those CardIds (minus the one that
    /// was found and moved to another zone).
    LibraryReordered {
        /// Which player's library was reordered
        player: PlayerId,
        /// New order of CardIds in the library (top to bottom)
        /// Identities remain unknown until individually revealed
        new_order: Vec<CardId>,
    },

    /// Request a choice from this client
    ChoiceRequest {
        /// Sequence number for response correlation
        choice_seq: u32,
        /// Which player must make this choice (P1=0, P2=1)
        /// This is always the local player for the receiving client
        for_player: PlayerId,
        /// Type of choice being requested
        choice_type: ChoiceType,
        /// Human-readable options (for verification against client's local computation)
        options: Vec<String>,
        /// Game state hash at this decision point (excludes hidden info)
        state_hash: u64,
        /// Server's undo_log length BEFORE this choice is made
        ///
        /// This is the authoritative sync point. Client should validate their
        /// shadow state's action_count matches before proceeding with the choice.
        /// Client echoes this value back in SubmitChoice.action_count.
        action_count: u64,
        /// Wall-clock timestamp for debugging (ms since Unix epoch)
        timestamp_ms: u64,
        /// Optional context for the choice
        context: Option<ChoiceContext>,
        /// Debug synchronization info (only in network debug mode)
        debug_info: Option<DebugSyncInfo>,
        /// For Priority choices, the server's authoritative list of available abilities.
        ///
        /// Index 0 is "Pass priority" (None), indices 1+ are the actual abilities.
        /// This eliminates race conditions where the client computes abilities before
        /// receiving all CardRevealed messages. The client should use these abilities
        /// instead of locally-computed ones for NetworkLocalController.
        #[serde(default)]
        abilities: Option<Vec<Option<SpellAbility>>>,
    },

    /// Notify client of opponent's choice (for sync)
    OpponentChoice {
        /// Choice sequence number
        choice_seq: u32,
        /// Which player made this choice (P1=0, P2=1)
        player: PlayerId,
        /// What type of choice was made
        choice_type: ChoiceType,
        /// The choice indices selected
        ///
        /// For single-select choices (priority, targets), this is a 1-element vec.
        /// For multi-select choices (attackers, blockers), contains all selected indices.
        choice_indices: Vec<usize>,
        /// Human-readable description of what was chosen
        description: String,
        /// Server's undo_log length when this opponent choice was recorded
        ///
        /// Client uses this to verify their shadow state is synchronized.
        /// Should match the action_count from the ChoiceRequest that prompted
        /// this opponent's decision.
        action_count: u64,
        /// Wall-clock timestamp for debugging (ms since Unix epoch)
        timestamp_ms: u64,
        /// The actual spell ability chosen (for Priority choices)
        ///
        /// When the opponent plays a spell/land/ability, this contains the
        /// actual ability so the client can execute it directly without
        /// needing to compute available abilities from hidden hand contents.
        spell_ability: Option<SpellAbility>,
        /// The CardId chosen for library search operations
        ///
        /// When the opponent searches their library (e.g., typecycling), this contains
        /// the specific CardId that was chosen. Client needs this because it cannot
        /// determine which CardId was chosen from the name alone (libraries are hidden).
        #[serde(default)]
        library_search_result: Option<CardId>,
        /// Actual target CardIds for target choices
        ///
        /// When the opponent chooses targets, this contains the actual CardIds chosen.
        /// Client uses these directly instead of mapping indices, which can fail if
        /// the client's valid_targets list differs from the server's.
        #[serde(default)]
        target_card_ids: Option<Vec<CardId>>,
        /// State hash AFTER applying this choice (for client validation)
        state_hash_after: Option<u64>,
        /// Debug synchronization info (only in network debug mode)
        debug_info: Option<DebugSyncInfo>,
    },

    /// Acknowledge receipt of a submitted choice
    ///
    /// Sent by server after receiving a valid SubmitChoice, allowing the client's
    /// NetworkLocalController to unblock and continue processing.
    ChoiceAccepted {
        /// Echo of the choice sequence for correlation
        choice_seq: u32,
        /// Server's undo_log length AFTER processing the choice
        ///
        /// This is typically `D + 1` where D was the action_count in ChoiceRequest,
        /// because a ChoicePoint action was logged. May be higher if the choice
        /// triggered additional automatic actions (e.g., spell resolution).
        #[serde(default)]
        action_count: u64,
        /// Wall-clock timestamp for debugging (ms since Unix epoch)
        #[serde(default)]
        timestamp_ms: u64,
        /// The CardId chosen for library search operations (for local player's choices)
        ///
        /// When the local player searches their library, the server picks the specific
        /// CardId and sends it back here so the client's shadow game can stay in sync.
        #[serde(default)]
        library_search_result: Option<CardId>,
    },

    /// Game has ended
    GameEnded {
        /// Winner (None for draw)
        winner: Option<PlayerId>,
        /// Why the game ended
        reason: GameEndReason,
        /// Final state hash for verification
        final_state_hash: u64,
        /// Final action count (undo log length) - used for sync verification
        /// In debug mode, clients should compare this against their own action_count
        action_count: u64,
    },

    /// Synchronization error detected
    ///
    /// Sent when server detects that client's state has diverged from server's.
    /// Contains diagnostic information to help identify where the drift occurred.
    SyncError {
        /// Detailed error information
        details: SyncErrorDetails,
        /// Whether this is fatal (connection will close)
        fatal: bool,
    },

    /// Error message
    Error {
        /// Error description
        message: String,
        /// Whether this is a fatal error (connection will close)
        fatal: bool,
    },

    /// Keepalive pong response
    Pong {
        /// Echoed timestamp from ping
        timestamp_ms: u64,
    },

    /// Debug state dump (only sent when hash verification fails)
    ///
    /// This is NOT part of normal game flow - only for diagnostics.
    #[cfg(debug_assertions)]
    DebugStateDump {
        /// Full game state as JSON for diffing
        state_json: String,
        /// What triggered the dump
        reason: String,
        /// Expected hash
        expected_hash: u64,
        /// Client's reported hash (if applicable)
        client_hash: Option<u64>,
    },
}

// ═══════════════════════════════════════════════════════════════════════════
// SUPPORTING TYPES
// ═══════════════════════════════════════════════════════════════════════════

/// Deterministic CardID ranges for late-binding architecture
///
/// In the late-binding model, CardIDs are assigned positionally at game start:
/// - P1's deck gets CardIDs [0, p1_deck_size)
/// - P2's deck gets CardIDs [p1_deck_size, p1_deck_size + p2_deck_size)
///
/// This is PUBLIC information - all players know which CardIDs belong to which
/// deck. Only the CardID ⟺ CardName binding remains hidden until revealed.
///
/// # Example
///
/// With P1's 40-card deck and P2's 40-card deck:
/// - P1's deck: CardIDs 0..39 (inclusive range [0, 40))
/// - P2's deck: CardIDs 40..79 (inclusive range [40, 80))
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeckCardIdRanges {
    /// First CardID in P1's deck (inclusive)
    pub p1_start: u32,
    /// One past the last CardID in P1's deck (exclusive)
    pub p1_end: u32,
    /// First CardID in P2's deck (inclusive)
    pub p2_start: u32,
    /// One past the last CardID in P2's deck (exclusive)
    pub p2_end: u32,
}

impl DeckCardIdRanges {
    /// Create new ranges from deck sizes
    ///
    /// P1's deck gets CardIDs [0, p1_size)
    /// P2's deck gets CardIDs [p1_size, p1_size + p2_size)
    pub fn from_deck_sizes(p1_size: usize, p2_size: usize) -> Self {
        let p1_start = 0u32;
        let p1_end = p1_size as u32;
        let p2_start = p1_end;
        let p2_end = p2_start + p2_size as u32;
        Self {
            p1_start,
            p1_end,
            p2_start,
            p2_end,
        }
    }

    /// Get the CardID range for P1's deck as [start, end)
    pub fn p1_range(&self) -> std::ops::Range<u32> {
        self.p1_start..self.p1_end
    }

    /// Get the CardID range for P2's deck as [start, end)
    pub fn p2_range(&self) -> std::ops::Range<u32> {
        self.p2_start..self.p2_end
    }

    /// Get total number of CardIDs (both decks combined)
    pub fn total_cards(&self) -> u32 {
        self.p2_end
    }

    /// Check if a CardID belongs to P1's deck
    pub fn is_p1_card(&self, card_id: CardId) -> bool {
        let id = card_id.as_u32();
        id >= self.p1_start && id < self.p1_end
    }

    /// Check if a CardID belongs to P2's deck
    pub fn is_p2_card(&self, card_id: CardId) -> bool {
        let id = card_id.as_u32();
        id >= self.p2_start && id < self.p2_end
    }
}

/// Information about a revealed card
///
/// Contains the card's server-assigned ID, name, and optionally the full card definition.
/// When `card_def` is provided, the client can instantiate the card directly without
/// needing a local card database. This enables lightweight/headless clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardReveal {
    /// The card's entity ID (must match server's ID for sync)
    pub card_id: CardId,
    /// Card name (for logging and validation)
    pub name: String,
    /// Full card definition (enables client to run without local card DB)
    /// Server should always provide this for real reveals; omitted for dummy reveals (hidden opponent cards)
    pub card_def: Option<crate::loader::CardDefinition>,
}

/// Reason a card was revealed to a player
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RevealReason {
    /// Card drawn from library
    Draw,
    /// Card revealed for targeting
    Targeting,
    /// Card played or cast (moved to public zone)
    Played,
    /// Card searched from library (tutor effect)
    Searched,
    /// Card revealed by a game effect
    Effect,
    /// Part of opening hand
    OpeningHand,
    /// Token created
    TokenCreated,
}

/// Type of choice being requested from the player
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChoiceType {
    /// Choose spell/ability to play (or pass priority)
    Priority {
        /// Number of available actions (excluding pass)
        available_count: usize,
    },
    /// Choose targets for spell/ability
    Targets {
        /// The spell/ability being targeted for
        spell_id: CardId,
        /// Number of targets to choose
        target_count: usize,
    },
    /// Choose mana sources to tap for payment
    ManaSources {
        /// The cost being paid
        cost: ManaCost,
    },
    /// Choose which creatures to attack with
    Attackers {
        /// Number of creatures that can attack
        available_count: usize,
    },
    /// Choose blockers and what they block
    Blockers {
        /// Number of attacking creatures
        attacker_count: usize,
        /// Number of potential blockers
        blocker_count: usize,
    },
    /// Choose damage assignment order for multiple blockers
    DamageOrder {
        /// The attacking creature
        attacker: CardId,
        /// Number of blockers to order
        blocker_count: usize,
    },
    /// Choose cards to discard (to hand size or effect)
    Discard {
        /// Number of cards to discard
        count: usize,
    },
    /// Choose a card from library by NAME (tutor/search effect)
    ///
    /// This variant sends unique card names instead of CardIds, allowing
    /// the client to choose without knowing which specific CardId will be used.
    /// The server picks the actual CardId after receiving the name choice.
    ///
    /// ## Protocol
    /// 1. Server filters library for matching cards, extracts unique names
    /// 2. Server sends names in `options` field (e.g., ["Decline", "Island", "Swamp"])
    /// 3. Client picks a name index
    /// 4. Server picks a CardId with that name from valid_cards
    /// 5. Server sends CardRevealed for the chosen card only
    LibrarySearchByName {
        /// Unique card names matching the search filter
        /// (derived from valid_cards, deduplicated by name)
        unique_names: Vec<String>,
        /// Number of copies of each unique name in the library.
        /// Same length as unique_names. Allows client to pick a specific
        /// instance when multiple cards have the same name (for LOCAL/NETWORK equivalence).
        name_counts: Vec<usize>,
        /// Description of what's being searched for (e.g., "a basic land")
        filter_description: String,
    },
    /// Choose permanents to sacrifice (Balance, Cataclysm, etc.)
    Sacrifice {
        /// Number of valid permanents that can be sacrificed
        valid_count: usize,
        /// Number of permanents to sacrifice
        count: usize,
        /// Description of the permanent type (e.g., "creatures", "lands")
        card_type_description: String,
    },
    /// Choose modes for a modal spell (e.g., "Choose one —")
    ///
    /// Modal spells like Heartless Act, Cryptic Command, or charms require
    /// the player to select one or more modes when casting.
    Modes {
        /// The spell being cast (for context)
        spell_id: CardId,
        /// Number of modes to choose (usually 1, but can be more for "choose two")
        mode_count: usize,
        /// Minimum number of modes required (may be less than mode_count for optional modes)
        min_modes: usize,
        /// Whether the same mode can be chosen multiple times (for Entwine-like effects)
        can_repeat: bool,
        /// Total number of available modes
        available_modes: usize,
    },
}

/// Additional context for a choice request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChoiceContext {
    /// Spell/ability that triggered this choice (if applicable)
    pub spell: Option<CardReveal>,
    /// Human-readable description of the choice
    pub description: String,
}

/// Opponent's initial deck list (tournament-style visibility)
///
/// Contains the INITIAL deck list before any sideboarding. After game 1,
/// you know what cards they COULD have, but not which sideboard cards
/// they actually swapped in.
///
/// Note: Deck sizes vary by format (60+ for Standard/Modern, 100 for Commander)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeckListInfo {
    /// Main deck card names and counts
    pub main_deck: Vec<(String, u8)>,
    /// Sideboard card names and counts (empty for Commander)
    pub sideboard: Vec<(String, u8)>,
    /// Total main deck size
    pub main_deck_size: usize,
    /// Total sideboard size
    pub sideboard_size: usize,
}

impl DeckListInfo {
    /// Create from a deck submission
    pub fn from_submission(deck: &DeckSubmission) -> Self {
        Self {
            main_deck: deck.main_deck.clone(),
            sideboard: deck.sideboard.clone(),
            main_deck_size: deck.main_deck_size(),
            sideboard_size: deck.sideboard_size(),
        }
    }

    /// Convert to a DeckList for game initialization
    pub fn to_deck_list(&self) -> crate::loader::DeckList {
        use crate::loader::DeckEntry;

        crate::loader::DeckList {
            main_deck: self
                .main_deck
                .iter()
                .map(|(name, count)| DeckEntry {
                    card_name: name.clone(),
                    count: *count,
                })
                .collect(),
            sideboard: self
                .sideboard
                .iter()
                .map(|(name, count)| DeckEntry {
                    card_name: name.clone(),
                    count: *count,
                })
                .collect(),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// NETWORK DEBUG SYNC INFO
// ═══════════════════════════════════════════════════════════════════════════

/// Debug synchronization information for detecting and diagnosing state drift.
///
/// Only populated when network debug mode is enabled (`--network-debug`).
/// Contains enough information to identify where client/server states diverged.
///
/// Design principle: Include only PUBLIC information that both sides can compute,
/// so we can meaningfully compare what each side believes the state to be.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DebugSyncInfo {
    /// Current turn number
    pub turn: u32,
    /// Current phase name (e.g., "Main1", "Combat", "End")
    pub phase: String,
    /// Which player's turn it is
    pub active_player: PlayerId,
    /// Who currently has priority (if applicable)
    pub priority_player: Option<PlayerId>,
    /// Life totals: [P1_life, P2_life]
    pub life_totals: [i32; 2],
    /// Hand sizes: [P1_hand_size, P2_hand_size]
    pub hand_sizes: [usize; 2],
    /// Library sizes: [P1_lib_size, P2_lib_size]
    pub library_sizes: [usize; 2],
    /// Number of permanents on battlefield
    pub battlefield_count: usize,
    /// Number of items on stack
    pub stack_size: usize,
    /// Graveyard sizes: [P1_gy_size, P2_gy_size]
    pub graveyard_sizes: [usize; 2],
    /// Last N actions from undo log (human-readable strings)
    /// Typically the last 10-20 actions for debugging
    #[serde(default)]
    pub last_actions: Vec<String>,
    /// Hash of RNG state for detecting shuffle divergence.
    /// If server and client RNGs diverge, this will differ.
    #[serde(default)]
    pub rng_hash: Option<u64>,
    /// CardIds in the requesting player's hand (sorted for comparison).
    /// This allows detecting hand desync - when client/server disagree on
    /// which cards are in the player's hand.
    #[serde(default)]
    pub requesting_player_hand_ids: Vec<u32>,
}

impl DebugSyncInfo {
    /// Create a new DebugSyncInfo with default values
    pub fn new() -> Self {
        Self {
            turn: 0,
            phase: String::new(),
            active_player: PlayerId::new(0),
            priority_player: None,
            life_totals: [0, 0],
            hand_sizes: [0, 0],
            library_sizes: [0, 0],
            battlefield_count: 0,
            stack_size: 0,
            graveyard_sizes: [0, 0],
            last_actions: Vec::new(),
            rng_hash: None,
            requesting_player_hand_ids: Vec::new(),
        }
    }

    /// Format for error output - concise single-line summary
    pub fn summary(&self) -> String {
        format!(
            "T{}:{} active=P{} life=[{},{}] hands=[{},{}] libs=[{},{}] bf={} stack={}",
            self.turn,
            self.phase,
            self.active_player.as_u32(),
            self.life_totals[0],
            self.life_totals[1],
            self.hand_sizes[0],
            self.hand_sizes[1],
            self.library_sizes[0],
            self.library_sizes[1],
            self.battlefield_count,
            self.stack_size,
        )
    }

    /// Format detailed multi-line output for diagnostics
    pub fn detailed(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("  Turn: {}, Phase: {}\n", self.turn, self.phase));
        out.push_str(&format!("  Active player: P{}\n", self.active_player.as_u32()));
        if let Some(priority) = self.priority_player {
            out.push_str(&format!("  Priority: P{}\n", priority.as_u32()));
        }
        out.push_str(&format!(
            "  Life: P1={}, P2={}\n",
            self.life_totals[0], self.life_totals[1]
        ));
        out.push_str(&format!(
            "  Hands: P1={}, P2={}\n",
            self.hand_sizes[0], self.hand_sizes[1]
        ));
        out.push_str(&format!(
            "  Libraries: P1={}, P2={}\n",
            self.library_sizes[0], self.library_sizes[1]
        ));
        out.push_str(&format!(
            "  Graveyards: P1={}, P2={}\n",
            self.graveyard_sizes[0], self.graveyard_sizes[1]
        ));
        out.push_str(&format!(
            "  Battlefield: {} cards, Stack: {} items\n",
            self.battlefield_count, self.stack_size
        ));
        if !self.last_actions.is_empty() {
            out.push_str("  Last actions:\n");
            for (i, action) in self.last_actions.iter().enumerate() {
                out.push_str(&format!("    [{}] {}\n", i + 1, action));
            }
        }
        out
    }

    /// Compare two DebugSyncInfo and return differences
    pub fn diff(&self, other: &DebugSyncInfo) -> Vec<String> {
        let mut diffs = Vec::new();

        if self.turn != other.turn {
            diffs.push(format!("turn: {} vs {}", self.turn, other.turn));
        }
        if self.phase != other.phase {
            diffs.push(format!("phase: {} vs {}", self.phase, other.phase));
        }
        if self.active_player != other.active_player {
            diffs.push(format!(
                "active_player: P{} vs P{}",
                self.active_player.as_u32(),
                other.active_player.as_u32()
            ));
        }
        if self.life_totals != other.life_totals {
            diffs.push(format!(
                "life_totals: {:?} vs {:?}",
                self.life_totals, other.life_totals
            ));
        }
        if self.hand_sizes != other.hand_sizes {
            diffs.push(format!("hand_sizes: {:?} vs {:?}", self.hand_sizes, other.hand_sizes));
        }
        if self.library_sizes != other.library_sizes {
            diffs.push(format!(
                "library_sizes: {:?} vs {:?}",
                self.library_sizes, other.library_sizes
            ));
        }
        if self.battlefield_count != other.battlefield_count {
            diffs.push(format!(
                "battlefield_count: {} vs {}",
                self.battlefield_count, other.battlefield_count
            ));
        }
        if self.stack_size != other.stack_size {
            diffs.push(format!("stack_size: {} vs {}", self.stack_size, other.stack_size));
        }
        if self.graveyard_sizes != other.graveyard_sizes {
            diffs.push(format!(
                "graveyard_sizes: {:?} vs {:?}",
                self.graveyard_sizes, other.graveyard_sizes
            ));
        }
        if self.rng_hash != other.rng_hash {
            diffs.push(format!("rng_hash: {:?} vs {:?}", self.rng_hash, other.rng_hash));
        }
        // Only compare hand IDs if both have them populated
        if !self.requesting_player_hand_ids.is_empty()
            && !other.requesting_player_hand_ids.is_empty()
            && self.requesting_player_hand_ids != other.requesting_player_hand_ids
        {
            diffs.push(format!(
                "HAND DESYNC: server hand_ids={:?} vs client hand_ids={:?}",
                self.requesting_player_hand_ids, other.requesting_player_hand_ids
            ));
        }

        diffs
    }
}

impl Default for DebugSyncInfo {
    fn default() -> Self {
        Self::new()
    }
}

/// Network sync error details
///
/// Sent when server or client detects a state hash mismatch.
/// Contains enough information to diagnose where divergence occurred.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncErrorDetails {
    /// The choice sequence where mismatch was detected
    pub choice_seq: u32,
    /// Expected action count
    pub expected_action_count: u64,
    /// Actual action count reported
    pub actual_action_count: u64,
    /// Expected state hash
    pub expected_hash: u64,
    /// Actual state hash reported
    pub actual_hash: u64,
    /// Debug info from the side that detected the error
    pub local_debug_info: DebugSyncInfo,
    /// Debug info from the other side (if available)
    pub remote_debug_info: Option<DebugSyncInfo>,
    /// Human-readable description of the mismatch
    pub description: String,
}

impl SyncErrorDetails {
    /// Format a detailed error report for logging
    pub fn format_report(&self) -> String {
        let mut report = String::new();
        report.push_str("=== NETWORK SYNC ERROR ===\n");
        report.push_str(&format!(
            "Detected at: choice_seq={}, action_count={} (expected {})\n",
            self.choice_seq, self.actual_action_count, self.expected_action_count
        ));
        report.push_str(&format!(
            "Hash mismatch: actual={:016x} expected={:016x}\n",
            self.actual_hash, self.expected_hash
        ));
        report.push_str(&format!("\n{}\n", self.description));

        report.push_str("\nLocal state:\n");
        report.push_str(&self.local_debug_info.detailed());

        if let Some(ref remote) = self.remote_debug_info {
            report.push_str("\nRemote state:\n");
            report.push_str(&remote.detailed());

            let diffs = self.local_debug_info.diff(remote);
            if !diffs.is_empty() {
                report.push_str("\nDifferences detected:\n");
                for diff in diffs {
                    report.push_str(&format!("  - {}\n", diff));
                }
            }
        }

        report.push_str("==========================\n");
        report
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// TESTS
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(clippy::wildcard_enum_match_arm)] // Test panic branch
    fn test_client_message_serialization() {
        let msg = ClientMessage::Authenticate {
            password: "secret".to_string(),
            player_name: Some("Alice".to_string()),
            deck: DeckSubmission::new(
                vec![("Lightning Bolt".to_string(), 4), ("Mountain".to_string(), 20)],
                vec![("Pyroclasm".to_string(), 2)],
            ),
        };

        let json = serde_json::to_string(&msg).expect("serialize");
        let roundtrip: ClientMessage = serde_json::from_str(&json).expect("deserialize");

        match roundtrip {
            ClientMessage::Authenticate {
                password,
                player_name,
                deck,
            } => {
                assert_eq!(password, "secret");
                assert_eq!(player_name, Some("Alice".to_string()));
                assert_eq!(deck.main_deck_size(), 24);
                assert_eq!(deck.sideboard_size(), 2);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    #[allow(clippy::wildcard_enum_match_arm)] // Test panic branch
    fn test_client_message_serialization_no_name() {
        // Test with None player_name (server should assign default)
        let msg = ClientMessage::Authenticate {
            password: "secret".to_string(),
            player_name: None,
            deck: DeckSubmission::new(
                vec![("Lightning Bolt".to_string(), 4), ("Mountain".to_string(), 20)],
                vec![("Pyroclasm".to_string(), 2)],
            ),
        };

        let json = serde_json::to_string(&msg).expect("serialize");
        // player_name is serialized as null (skip_serializing_if was removed for bincode compat)

        let roundtrip: ClientMessage = serde_json::from_str(&json).expect("deserialize");

        match roundtrip {
            ClientMessage::Authenticate {
                password,
                player_name,
                deck,
            } => {
                assert_eq!(password, "secret");
                assert_eq!(player_name, None);
                assert_eq!(deck.main_deck_size(), 24);
                assert_eq!(deck.sideboard_size(), 2);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    #[allow(clippy::wildcard_enum_match_arm)] // Test panic branch
    fn test_server_message_serialization() {
        let msg = ServerMessage::ChoiceRequest {
            choice_seq: 42,
            for_player: PlayerId::new(0),
            choice_type: ChoiceType::Priority { available_count: 3 },
            options: vec![
                "Pass priority".to_string(),
                "Play land: Mountain".to_string(),
                "Cast spell: Lightning Bolt".to_string(),
            ],
            state_hash: 0xDEADBEEF,
            action_count: 0,
            timestamp_ms: 1234567890,
            context: None,
            debug_info: None,
            abilities: None,
        };

        let json = serde_json::to_string(&msg).expect("serialize");
        let roundtrip: ServerMessage = serde_json::from_str(&json).expect("deserialize");

        match roundtrip {
            ServerMessage::ChoiceRequest {
                choice_seq,
                options,
                state_hash,
                ..
            } => {
                assert_eq!(choice_seq, 42);
                assert_eq!(options.len(), 3);
                assert_eq!(state_hash, 0xDEADBEEF);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_card_reveal_serialization() {
        let reveal = CardReveal {
            card_id: CardId::new(123),
            name: "Serra Angel".to_string(),
            card_def: None, // Test without card definition
        };

        let json = serde_json::to_string(&reveal).expect("serialize");
        assert!(json.contains("Serra Angel"));
        assert!(json.contains("123"));

        let roundtrip: CardReveal = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(roundtrip.name, "Serra Angel");
        assert_eq!(roundtrip.card_id, CardId::new(123));
    }

    #[test]
    #[allow(clippy::wildcard_enum_match_arm)] // Test panic branch
    fn test_choice_type_serialization() {
        let choice = ChoiceType::Targets {
            spell_id: CardId::new(42),
            target_count: 1,
        };

        let json = serde_json::to_string(&choice).expect("serialize");
        assert!(json.contains("targets"));

        let roundtrip: ChoiceType = serde_json::from_str(&json).expect("deserialize");
        match roundtrip {
            ChoiceType::Targets { spell_id, target_count } => {
                assert_eq!(spell_id, CardId::new(42));
                assert_eq!(target_count, 1);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_reveal_reason_serialization() {
        for reason in [
            RevealReason::Draw,
            RevealReason::Targeting,
            RevealReason::Played,
            RevealReason::Searched,
            RevealReason::Effect,
            RevealReason::OpeningHand,
            RevealReason::TokenCreated,
        ] {
            let json = serde_json::to_string(&reason).expect("serialize");
            let roundtrip: RevealReason = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(roundtrip, reason);
        }
    }

    #[test]
    fn test_all_server_message_variants() {
        // Test all ServerMessage variants for round-trip serialization
        let player_id = PlayerId::new(1);
        let card_id = CardId::new(42);

        let messages: Vec<ServerMessage> = vec![
            ServerMessage::AuthResult {
                success: true,
                error: None,
                your_player_id: Some(player_id),
                your_name: Some("Player1".to_string()),
            },
            ServerMessage::AuthResult {
                success: false,
                error: Some("Invalid password".to_string()),
                your_player_id: None,
                your_name: None,
            },
            ServerMessage::WaitingForOpponent,
            ServerMessage::GameStarted {
                your_player_id: player_id,
                opponent_name: "Bob".to_string(),
                opening_hand: vec![CardReveal {
                    card_id,
                    name: "Mountain".to_string(),
                    card_def: None,
                }],
                opponent_hand_count: 7,
                library_size: 53,
                opponent_library_size: 53,
                opponent_decklist: None,
                starting_life: 20,
                initial_state_hash: 0x12345678,
                network_debug: false,
                deck_card_ids: Some(DeckCardIdRanges::from_deck_sizes(60, 60)),
                token_definitions: std::collections::HashMap::new(),
                rng_state: vec![1, 2, 3, 4], // Dummy RNG state for testing
            },
            ServerMessage::ChoiceRequest {
                choice_seq: 1,
                for_player: player_id,
                choice_type: ChoiceType::Priority { available_count: 2 },
                options: vec!["Pass".to_string(), "Play Mountain".to_string()],
                state_hash: 0xABCDEF,
                action_count: 0,
                timestamp_ms: 1234567890,
                context: None,
                debug_info: None,
                abilities: None,
            },
            ServerMessage::CardRevealed {
                owner: player_id,
                card: CardReveal {
                    card_id,
                    name: "Lightning Bolt".to_string(),
                    card_def: None,
                },
                reason: RevealReason::Draw,
            },
            ServerMessage::OpponentChoice {
                choice_seq: 5,
                player: player_id,
                choice_type: ChoiceType::Priority { available_count: 0 },
                choice_indices: vec![0],
                description: "Pass priority".to_string(),
                action_count: 0,
                timestamp_ms: 1234567891,
                spell_ability: None,
                state_hash_after: None,
                debug_info: None,
                library_search_result: None,
                target_card_ids: None,
            },
            ServerMessage::GameEnded {
                winner: Some(player_id),
                reason: GameEndReason::PlayerDeath(PlayerId::new(0)),
                final_state_hash: 0xFEDCBA98,
                action_count: 123,
            },
            ServerMessage::GameEnded {
                winner: None,
                reason: GameEndReason::Draw,
                final_state_hash: 0,
                action_count: 456,
            },
            ServerMessage::Error {
                message: "Connection timeout".to_string(),
                fatal: true,
            },
            ServerMessage::Pong {
                timestamp_ms: 1234567890,
            },
        ];

        for msg in messages {
            let json = serde_json::to_string(&msg).expect("serialize");
            let roundtrip: ServerMessage = serde_json::from_str(&json).expect("deserialize");
            // Re-serialize to compare (since we can't derive PartialEq for all variants)
            let json2 = serde_json::to_string(&roundtrip).expect("re-serialize");
            assert_eq!(json, json2, "Round-trip failed for message variant");
        }
    }

    #[test]
    fn test_all_client_message_variants() {
        // Test all ClientMessage variants for round-trip serialization
        let messages: Vec<ClientMessage> = vec![
            ClientMessage::Authenticate {
                password: "secret123".to_string(),
                player_name: Some("Alice".to_string()),
                deck: DeckSubmission::new(
                    vec![("Forest".to_string(), 20), ("Grizzly Bears".to_string(), 4)],
                    vec![],
                ),
            },
            ClientMessage::SubmitChoice {
                choice_seq: 42,
                choice_indices: vec![1],
                action_count: 0,
                timestamp_ms: 1234567890,
                client_state_hash: None,
                debug_info: None,
                spell_ability: None,
                target_card_ids: None,
            },
            ClientMessage::Ping {
                timestamp_ms: 9876543210,
            },
            ClientMessage::Disconnect,
        ];

        for msg in messages {
            let json = serde_json::to_string(&msg).expect("serialize");
            let roundtrip: ClientMessage = serde_json::from_str(&json).expect("deserialize");
            let json2 = serde_json::to_string(&roundtrip).expect("re-serialize");
            assert_eq!(json, json2, "Round-trip failed for message variant");
        }
    }

    #[test]
    fn test_choice_type_all_variants() {
        let card_id = CardId::new(99);

        let choice_types = vec![
            ChoiceType::Priority { available_count: 5 },
            ChoiceType::Targets {
                spell_id: card_id,
                target_count: 2,
            },
            ChoiceType::ManaSources {
                cost: ManaCost::from_string("2R"),
            },
            ChoiceType::Attackers { available_count: 4 },
            ChoiceType::Blockers {
                attacker_count: 3,
                blocker_count: 5,
            },
            ChoiceType::DamageOrder {
                attacker: card_id,
                blocker_count: 2,
            },
            ChoiceType::Discard { count: 2 },
            ChoiceType::LibrarySearchByName {
                unique_names: vec!["Island".to_string(), "Swamp".to_string()],
                name_counts: vec![2, 3], // 2 Islands, 3 Swamps
                filter_description: "a basic land".to_string(),
            },
            ChoiceType::Sacrifice {
                valid_count: 5,
                count: 2,
                card_type_description: "creatures".to_string(),
            },
            ChoiceType::Modes {
                spell_id: card_id,
                mode_count: 1,
                min_modes: 1,
                can_repeat: false,
                available_modes: 2,
            },
        ];

        for ct in choice_types {
            let json = serde_json::to_string(&ct).expect("serialize");
            let roundtrip: ChoiceType = serde_json::from_str(&json).expect("deserialize");
            let json2 = serde_json::to_string(&roundtrip).expect("re-serialize");
            assert_eq!(json, json2, "Round-trip failed for ChoiceType variant");
        }
    }

    #[test]
    fn test_deck_card_id_ranges_from_deck_sizes() {
        let ranges = DeckCardIdRanges::from_deck_sizes(40, 40);

        // P1's deck: CardIDs 0..39
        assert_eq!(ranges.p1_start, 0);
        assert_eq!(ranges.p1_end, 40);

        // P2's deck: CardIDs 40..79
        assert_eq!(ranges.p2_start, 40);
        assert_eq!(ranges.p2_end, 80);

        assert_eq!(ranges.total_cards(), 80);
    }

    #[test]
    fn test_deck_card_id_ranges_ranges() {
        let ranges = DeckCardIdRanges::from_deck_sizes(60, 40);

        assert_eq!(ranges.p1_range(), 0..60);
        assert_eq!(ranges.p2_range(), 60..100);
    }

    #[test]
    fn test_deck_card_id_ranges_ownership() {
        let ranges = DeckCardIdRanges::from_deck_sizes(40, 40);

        // P1's cards: 0..39
        assert!(ranges.is_p1_card(CardId::new(0)));
        assert!(ranges.is_p1_card(CardId::new(39)));
        assert!(!ranges.is_p1_card(CardId::new(40)));

        // P2's cards: 40..79
        assert!(!ranges.is_p2_card(CardId::new(39)));
        assert!(ranges.is_p2_card(CardId::new(40)));
        assert!(ranges.is_p2_card(CardId::new(79)));
        assert!(!ranges.is_p2_card(CardId::new(80)));
    }

    #[test]
    fn test_deck_card_id_ranges_serialization() {
        let ranges = DeckCardIdRanges::from_deck_sizes(42, 38);

        let json = serde_json::to_string(&ranges).expect("serialize");
        let roundtrip: DeckCardIdRanges = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(roundtrip.p1_start, ranges.p1_start);
        assert_eq!(roundtrip.p1_end, ranges.p1_end);
        assert_eq!(roundtrip.p2_start, ranges.p2_start);
        assert_eq!(roundtrip.p2_end, ranges.p2_end);
    }

    #[test]
    fn test_deck_card_id_ranges_asymmetric_decks() {
        // Commander deck (100 cards) vs Limited deck (40 cards)
        let ranges = DeckCardIdRanges::from_deck_sizes(100, 40);

        assert_eq!(ranges.p1_start, 0);
        assert_eq!(ranges.p1_end, 100);
        assert_eq!(ranges.p2_start, 100);
        assert_eq!(ranges.p2_end, 140);

        // Boundary checks
        assert!(ranges.is_p1_card(CardId::new(99)));
        assert!(!ranges.is_p1_card(CardId::new(100)));
        assert!(ranges.is_p2_card(CardId::new(100)));
        assert!(ranges.is_p2_card(CardId::new(139)));
    }
}
