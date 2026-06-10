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
use crate::network::ChoicePayload;
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

/// Opaque token issued by the server when a player joins or creates a game.
///
/// A client that holds a valid `ReconnectToken` may re-join an in-progress
/// game after a connection drop by sending `ClientMessage::Reconnect`. The
/// token is bound to a specific `(game_name, player_id)` pair on the server;
/// presenting it to the wrong game or as the wrong player is rejected.
///
/// **Representation**: a random 128-bit value encoded as a lowercase hex
/// string (32 ASCII hex digits). This makes it copy-paste friendly and
/// trivially JSON-serializable as a plain string. 128 bits is sufficient to
/// make brute-force guessing impractical for the lifetime of a single game
/// (≤ 4 hours per the server cap).
///
/// **Lifecycle**: the server issues a fresh token on `CreateGame` / `JoinGame`
/// success; the same token is reused if `Reconnect` succeeds (i.e., the
/// server does not rotate the token on every reconnect). Tokens are
/// invalidated when the associated game ends.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ReconnectToken(pub String);

impl ReconnectToken {
    /// Generate a fresh cryptographically random 128-bit token.
    ///
    /// Uses `rand::random::<u128>()` for simplicity and safety — `rand`
    /// is already in the dependency tree and its `ThreadRng` source is
    /// `OsRng`-seeded.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn generate() -> Self {
        use rand::random;
        let bits: u128 = random();
        Self(format!("{bits:032x}"))
    }

    /// Check whether the token string is structurally valid (32 lowercase hex chars).
    pub fn is_valid_format(&self) -> bool {
        self.0.len() == 32 && self.0.chars().all(|c| c.is_ascii_hexdigit())
    }
}

impl std::fmt::Display for ReconnectToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Messages sent from client to server
///
/// Note: SubmitChoice variant is intentionally large (contains multiple Option<Vec<_>> fields
/// for various choice data). Boxing would complicate the protocol code for marginal benefit.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum ClientMessage {
    /// Register a unique display name on this lobby connection.
    ///
    /// **Must be the first message sent** by new lobby clients (those that do
    /// NOT use the legacy `Authenticate` flow). The server reserves the name
    /// for the lifetime of this WebSocket connection; when the connection
    /// drops, the reservation is released. A second client claiming the same
    /// name receives `ServerMessage::RegisterResult { success: false, .. }`.
    ///
    /// After a successful `Register` the client may call `ListGames`,
    /// `CreateGame`, or `JoinGame` without re-supplying a `player_name` —
    /// the server uses the registered name automatically. Supplying
    /// `player_name` in those messages overrides the registered name for
    /// that game (useful for per-game aliases), but the lobby-level
    /// reservation stays on the registered name.
    Register {
        /// Server password (must match server config if non-empty).
        password: String,
        /// Desired display name. Must be 1–32 printable non-whitespace-only
        /// ASCII characters. Validation is server-side; invalid names get a
        /// `RegisterResult { success: false, .. }`.
        player_name: String,
    },

    /// Initial authentication and deck submission.
    ///
    /// **Legacy single-game entry point** — kept for backwards compatibility
    /// with clients that pre-date the lobby. The server treats this as an
    /// implicit `JoinOrCreate` against a well-known game named
    /// [`DEFAULT_LOBBY_GAME`]: the first authenticator becomes that game's
    /// creator, the second joins it. New clients should use `Register` +
    /// `CreateGame` / `JoinGame` / `ListGames` for explicit lobby control.
    Authenticate {
        /// Server password
        password: String,
        /// Player's display name (None = let server assign a default name with suffix)
        player_name: Option<String>,
        /// Deck to use for the game
        deck: DeckSubmission,
    },

    /// List currently waiting (pre-game) lobby games.
    ///
    /// The server replies with a single [`ServerMessage::GameList`]. The
    /// connection remains open afterwards so the client can follow up with a
    /// `CreateGame` or `JoinGame`. Sending `Authenticate`/`CreateGame`/
    /// `JoinGame` after `ListGames` is the normal flow for a UI client that
    /// shows a lobby browser.
    ///
    /// `query` is optional. When omitted (or fully default) the server returns
    /// every waiting game (legacy behavior). When provided, the server applies
    /// the case-insensitive substring `filter` against game name OR creator
    /// name, then paginates with `limit`/`offset`. The reply's `total_count`
    /// reflects the post-filter total so the client can render
    /// "Showing N of M".
    ListGames {
        /// Server password (must match server config if non-empty)
        password: String,
        /// Optional filter + pagination. `None` ⇒ legacy "return all" behavior.
        #[serde(default)]
        query: Option<ListGamesQuery>,
    },

    /// List currently registered (logged-in) lobby players.
    ///
    /// The server replies with a single [`ServerMessage::PlayerList`]. The
    /// connection stays open afterwards, exactly like `ListGames`, so a UI
    /// client can poll both lists on its lobby refresh timer and then follow
    /// up with `CreateGame`/`JoinGame`.
    ///
    /// The set of players is the server's name registry — every connection
    /// that has sent a successful [`ClientMessage::Register`] and not yet
    /// disconnected. `query` mirrors `ListGames`: when omitted the server
    /// returns every registered player (no clamp); when present it applies the
    /// case-insensitive substring `filter` against the player name, then
    /// paginates with `limit`/`offset`. The reply's `total_count` is the
    /// post-filter total so the client can render "Showing N of M".
    ListPlayers {
        /// Server password (must match server config if non-empty)
        password: String,
        /// Optional filter + pagination. `None` ⇒ "return all" behavior.
        #[serde(default)]
        query: Option<ListPlayersQuery>,
    },

    /// Create a new pre-game lobby slot and become its creator.
    ///
    /// The connection then waits in the same way the legacy `Authenticate`
    /// flow does: the server sends `GameCreated` followed by
    /// `WaitingForOpponent`, and the game starts when a second player
    /// `JoinGame`s the same `game_name`.
    CreateGame {
        /// Server password (must match server config if non-empty)
        password: String,
        /// Optional human-friendly name for the game; if `None` the server
        /// generates a unique one (e.g., `"game-7"`).
        game_name: Option<String>,
        /// Optional per-game password — `JoinGame` must echo it.
        game_password: Option<String>,
        /// Player's display name (None = server assigns "Player1")
        player_name: Option<String>,
        /// Deck to use for the game
        deck: DeckSubmission,
        /// Opt in to the Variant-1 *rendezvous* waiting room (mtg-682).
        ///
        /// `false` (default — legacy / game-page clients): the game starts the
        /// instant a second player joins, exactly as before. `true` (the
        /// launcher's lobby socket): the game is created and LISTED but does
        /// NOT auto-start on join; instead both players SetReady, and when both
        /// are ready the server sends `WaitingRoomReady` to both and frees the
        /// slot so each can navigate to its game page. Pre-game only —
        /// determinism is untouched.
        #[serde(default)]
        waiting_room: bool,
    },

    /// Join an existing pre-game lobby slot.
    JoinGame {
        /// Server password (must match server config if non-empty)
        password: String,
        /// Name of the game to join (must already exist via `CreateGame`).
        game_name: String,
        /// Per-game password if the game was created with one.
        game_password: Option<String>,
        /// Player's display name (None = server assigns "Player2")
        player_name: Option<String>,
        /// Deck to use for the game
        deck: DeckSubmission,
        /// Opt in to the Variant-1 rendezvous waiting room (mtg-682). See
        /// [`ClientMessage::CreateGame::waiting_room`]. A joiner sets this to
        /// `true` when joining a game created by a launcher waiting room; the
        /// server then keeps both players in the waiting room until both
        /// `SetReady`, rather than starting immediately on join.
        #[serde(default)]
        waiting_room: bool,
    },

    /// Update the deck choice for this player in an active waiting room.
    ///
    /// Must be sent after a successful `CreateGame` or `JoinGame` response and
    /// before both players mark ready. The server updates its per-player state
    /// and broadcasts `WaitingRoomUpdate` to both players so each side sees
    /// the current deck selections and ready flags. Submitting a new deck
    /// implicitly resets the player's `ready` flag to `false` (so the other
    /// player cannot be surprised by a deck swap after the match starts).
    SetDeck {
        /// The deck the player wants to play. The server validates the deck
        /// (minimum size) and replies with `WaitingRoomUpdate`; an invalid
        /// deck leaves the previous selection in place and sends an
        /// `Error { fatal: false }` explaining the problem.
        deck: DeckSubmission,
    },

    /// Mark this player as ready (or unready) in the waiting room.
    ///
    /// When BOTH players are ready the server starts the match: it sends
    /// `GameStarted` to both, issues reconnect tokens, and transitions the
    /// game from `waiting_games` to `active_games`. A client that sends
    /// `SetReady { ready: true }` before setting a deck receives
    /// `Error { fatal: false }` (the game will not start until a valid deck
    /// is on record for both players).
    SetReady {
        /// `true` = ready, `false` = cancel ready (e.g. to switch deck).
        ready: bool,
    },

    /// Re-join an in-progress game after a connection drop.
    ///
    /// The client must supply the `ReconnectToken` it received when it
    /// originally joined. The server validates the token against its active
    /// game registry and, on success, re-attaches the WebSocket to the
    /// running game task. The response is a normal `GameStarted`-style
    /// snapshot of current state so the client can reconstruct its view.
    ///
    /// **Not yet fully wired to the in-game task** (mtg-682: the token
    /// lifecycle is implemented and tested; the actual mid-game
    /// resume of a running `GameLoop` is still a stub). A successful
    /// `Reconnect` currently responds with `ReconnectResult { success:
    /// true }` but the game may not yet continue on the reattached socket.
    Reconnect {
        /// The token received at game creation / join time.
        token: ReconnectToken,
        /// Game name (allows the server to look up the game before checking
        /// the token, avoiding a global O(n) search).
        game_name: String,
    },

    /// Submit a bug report to the server for local persistence
    BugReport {
        /// User-provided description of the issue
        description: String,
        /// Game log output captured on the client
        game_logs: String,
        /// Browser/dev console logs captured on the client
        console_logs: String,
        /// Optional trusted password for elevated handling
        trusted_password: Option<String>,
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
        /// VALIDATION ONLY (mtg-789). The server's canonical choice is ALWAYS
        /// the index-based lookup; when this field is present the server
        /// additionally asserts the index-selected ability `==` this
        /// `spell_ability` and treats any mismatch as a FATAL desync (early
        /// detection by CardId). It is NOT used "directly instead of the index".
        /// See `NetworkController` priority handling and
        /// `docs/NETWORK_ARCHITECTURE.md` ("desync is always fatal"). The
        /// native local controller populates this for all priority choices; the
        /// WASM/web client currently sends `None`, so the cross-check is a no-op
        /// on the deployed web path until it is threaded through (mtg-789 #2).
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
// BUFFERED FACT (minimal lazy protocol, mtg-752)
// ═══════════════════════════════════════════════════════════════════════════

/// One server→client fact in a `ChoiceRequest` buffer, stamped at its TRUE game
/// `action_count` (= the undo-log position of the action that produced it).
///
/// This is the minimal-lazy-protocol replacement for the eager server→client
/// message zoo: instead of firing `CardRevealed` / `LibraryReordered` /
/// `SearchCandidates` / `OpponentChoice` as separate messages at choice-accept
/// time (the dual-stamp source, mtg-752), the server collects every fact the
/// recipient needs to replay its shadow forward to the choice point into ONE
/// ascending-`ac` buffer carried by the next `ChoiceRequest`. The recipient
/// splits the buffer by variant into the two consumer logs it already owns:
/// the reveal-class variants map 1:1 onto [`crate::network::StateSyncEntry`]
/// (keyed by game `ac`), and [`BufferedFact::Choice`] maps onto
/// [`crate::network::ChoiceEntry`] (keyed by `choice_seq`).
///
/// During the additive dual-emit phase (mtg-752) the server sends both
/// this buffer AND the legacy eager messages — so old/new clients interoperate;
/// the buffer is authoritative and the eager copies are ignored by a
/// buffer-aware client.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)] // wire/transient buffer; CardReveal carries a full CardDefinition
pub enum BufferedFact {
    /// Hidden-info card identity the recipient is entitled to (own draw, an
    /// opponent's cast, a tutored card). Maps to `StateSyncEntry::RevealCard`.
    Reveal {
        owner: PlayerId,
        card: CardReveal,
        reason: RevealReason,
    },
    /// Server-authoritative library order after a shuffle / scry / surveil /
    /// search. Maps to `StateSyncEntry::LibraryReorder`. `new_order` is
    /// top-to-bottom.
    LibraryReorder { player: PlayerId, new_order: Vec<CardId> },
    /// Atomic-multi tutor candidate reveal (the N candidate identities a
    /// searcher sees, mtg-253). Maps to `StateSyncEntry::SearchCandidates`. One
    /// fact at the single search-resolution `ac` (N separate `Reveal`s at one
    /// `ac` would collide on the strictly-increasing state-sync log key).
    SearchCandidates { searcher: PlayerId, cards: Vec<CardReveal> },
    /// The OPPONENT's decision at this `ac`. Maps to `ChoiceEntry`. Carries the
    /// structured disambiguators the shadow's remote controller needs to replay
    /// the choice without seeing hidden information.
    Choice {
        choice_seq: u32,
        choice_type: ChoiceType,
        description: String,
        /// The structured decision payload (mtg-787): `choice_indices` plus the
        /// hidden-info disambiguators, flattened so the wire JSON is byte-compatible
        /// with the former inline fields. See [`crate::network::ChoicePayload`].
        #[serde(flatten)]
        payload: ChoicePayload,
    },
}

// ═══════════════════════════════════════════════════════════════════════════
// SERVER → CLIENT MESSAGES
// ═══════════════════════════════════════════════════════════════════════════

/// Messages sent from server to client
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)] // ChoiceRequest is the hot path; boxing adds overhead
pub enum ServerMessage {
    /// Result of a `ClientMessage::Register` request.
    ///
    /// Sent immediately after the server processes a `Register`. On success
    /// the name is reserved for the lifetime of this WebSocket connection;
    /// subsequent lobby actions inherit it.
    RegisterResult {
        /// `true` iff the name was accepted and reserved.
        success: bool,
        /// The name that was reserved (echoed from the request, so the client
        /// can correlate without tracking in-flight requests).
        player_name: String,
        /// Human-readable rejection reason when `success = false`
        /// (e.g., "Name already taken", "Name too long").
        error: Option<String>,
    },

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

    /// Phase-1 result of a bug report submission: the outcome of the server-side
    /// disk write, sent IMMEDIATELY after persistence is attempted and BEFORE any
    /// GitHub issue filing. This lets the client confirm the report is safely
    /// stored without waiting on a slow or unreachable GitHub (mtg-749).
    BugReportStored {
        /// Whether the report was successfully written to the server's disk.
        success: bool,
        /// Server-side directory the report was stored in (informational), when
        /// the write succeeded.
        report_dir: Option<String>,
        /// Error message if the disk write failed (the genuinely-bad case).
        error: Option<String>,
    },

    /// Phase-2 result of a bug report submission: the outcome of the GitHub
    /// issue-filing step, sent AFTER the (timeout-bounded) GitHub attempt
    /// completes. The report is already persisted by this point, so any failure
    /// here is non-fatal — the client surfaces it without ever spinning forever
    /// (mtg-749).
    BugReportIssueResult {
        /// URL of the filed GitHub issue, when filing succeeded.
        issue_url: Option<String>,
        /// Error / timeout message when filing did not produce an issue URL.
        error: Option<String>,
    },

    /// Waiting for opponent to connect
    WaitingForOpponent,

    /// Reply to `ClientMessage::ListGames`. Lists waiting (pre-game) lobby
    /// slots only — games already in progress are NOT advertised here.
    GameList {
        /// One entry per waiting game (post-filter, post-pagination). Order is
        /// server-defined (currently creation-time ascending); clients should
        /// not rely on it beyond stability across consecutive list calls.
        games: Vec<LobbyGameEntry>,
        /// Total number of games matching the filter BEFORE pagination was
        /// applied. Clients render "Showing games.len() of total_count".
        /// Defaults to `games.len()` for legacy decoders via `#[serde(default)]`
        /// on the client side — older servers that omit this field still
        /// deserialize fine.
        #[serde(default)]
        total_count: u32,
        /// Host system memory used as a percentage of total, if the server
        /// can read it (Linux only). Lets a UI show a "Server Full" warning
        /// before the user even tries to `CreateGame`.
        system_memory_used_percent: Option<u32>,
        /// Configured memory ceiling as a percentage (`0` = unlimited). New
        /// joins are denied with `ServerFull` once
        /// `system_memory_used_percent > max_memory_percent`.
        max_memory_percent: u32,
    },

    /// Reply to `ClientMessage::ListPlayers`. Lists registered (logged-in)
    /// lobby players — the server's name registry.
    ///
    /// Deliberately mirrors [`ServerMessage::GameList`] minus the game- and
    /// memory-specific fields: a player entry is just a display name.
    PlayerList {
        /// One entry per registered player (post-filter, post-pagination).
        /// Order is server-defined (currently case-insensitive by name);
        /// clients should not rely on it beyond stability across consecutive
        /// list calls.
        players: Vec<LobbyPlayerEntry>,
        /// Total number of players matching the filter BEFORE pagination was
        /// applied. Clients render "Showing players.len() of total_count".
        /// `#[serde(default)]` keeps older decoders that omit it working.
        #[serde(default)]
        total_count: u32,
    },

    /// Acknowledge a `CreateGame` succeeded — the client is now the creator
    /// of `game_name` and will receive `WaitingForOpponent` next, then the
    /// usual `GameStarted` flow once a second player joins.
    GameCreated {
        /// Final game name (server may rewrite an empty/duplicate request).
        game_name: String,
        /// Player ID assigned to the creator (always P1=0 for now).
        your_player_id: PlayerId,
        /// Display name the server settled on for the creator.
        your_name: Option<String>,
    },

    /// Snapshot of the waiting room state, sent to both players whenever
    /// either player's deck selection or ready flag changes.
    ///
    /// Both the creator and the joiner receive this message on every
    /// `SetDeck` / `SetReady` update so their UIs stay in sync without
    /// polling. The message is also sent once to the joiner when they first
    /// join, so they immediately see the creator's current state.
    WaitingRoomUpdate {
        /// Creator's current state.
        creator: WaitingRoomPlayerState,
        /// Joiner's current state. `None` until the second player joins.
        joiner: Option<WaitingRoomPlayerState>,
    },

    /// Both players in a waiting room have marked themselves ready — the match
    /// is cleared to start (Variant 1 auto-start, mtg-682).
    ///
    /// This is the pre-game *rendezvous* "go" signal for the launcher waiting
    /// room. The launcher holds a lobby WebSocket purely for the waiting-room
    /// handshake (CreateGame/JoinGame → SetDeck/SetReady → WaitingRoomUpdate);
    /// it is NOT a game-playing socket. When the server observes BOTH players
    /// ready it sends `WaitingRoomReady` to both launcher sockets, frees the
    /// pending slot, and closes the lobby connection. Each client then
    /// navigates to its chosen game page, which opens its OWN durable game
    /// WebSocket and performs the real `CreateGame` / `JoinGame` that drives
    /// `run_game` and emits the full `GameStarted` payload.
    ///
    /// Splitting the rendezvous (launcher) from the game socket (game page) is
    /// deliberate: a browser WebSocket cannot survive a full page navigation,
    /// and the in-game `Reconnect` resume path is still a stub. Keeping the
    /// handshake on a throwaway lobby socket lets the launcher show live join +
    /// ready status without entangling it with the game task. All of this is
    /// strictly PRE-GAME state — no game RNG, no controller decisions, no
    /// hidden information — so the network-determinism invariants are untouched.
    WaitingRoomReady {
        /// Final game name the two players agreed on (echoed so the launcher
        /// can build the game-page redirect even if the server rewrote it).
        game_name: String,
        /// `true` for the player who created the game (P1), `false` for the
        /// joiner (P2). The launcher uses this to decide whether the game page
        /// should `CreateGame` (creator) or `JoinGame` (joiner) on its own WS.
        is_creator: bool,
    },

    /// Result of a `ClientMessage::Reconnect` request.
    ///
    /// The token lifecycle is fully implemented (issue on
    /// CreateGame/JoinGame, validate on Reconnect, invalidate on game end),
    /// but the in-game task reattachment is stubbed. A successful reconnect
    /// returns `success: true`; mid-game resume wiring is still a stub (mtg-682).
    ReconnectResult {
        /// `true` iff the token was valid and the reconnect was accepted.
        success: bool,
        /// Game name the player reconnected to (echoed for correlation).
        game_name: String,
        /// Your player ID in the game (echoed for the UI).
        your_player_id: Option<PlayerId>,
        /// Human-readable error when `success = false`.
        error: Option<String>,
    },

    /// `CreateGame`/`JoinGame` rejected because host memory pressure is at or
    /// above the configured ceiling. The client should back off; a follow-up
    /// `ListGames` is fine. We deliberately reuse `Error { fatal: true }`
    /// semantics here — the connection closes after this message so the
    /// client can retry with a fresh socket later.
    ///
    /// SECURITY: The wire-visible payload is intentionally generic — host
    /// memory percentages and the configured ceiling are NOT exposed to the
    /// client (they would leak server infrastructure detail). The server
    /// logs the precise values at `warn` level for operators. See
    /// `build_server_full_message` in `network::lobby`.
    ServerFull {
        /// Generic, opaque reason intended for end users (e.g. "Server is
        /// full, try again later"). Must not include host memory metrics or
        /// any other operational telemetry.
        reason: String,
    },

    /// `JoinGame` rejected for a non-capacity reason.
    JoinFailed {
        /// Game name the client tried to join.
        game_name: String,
        /// Specific failure reason.
        reason: JoinFailReason,
    },

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
        /// Deterministic CardID ranges for late-binding architecture (mtg-218)
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
        /// Reconnect token for this player in this game.
        ///
        /// The client MUST persist this token. If the WebSocket drops during
        /// the game, the client can send `ClientMessage::Reconnect { token,
        /// game_name }` on a new connection to re-attach to the running game.
        /// The token is invalidated when the game ends (either naturally or
        /// by timeout).
        #[serde(default)]
        reconnect_token: Option<ReconnectToken>,
    },

    /// Card reveal event (draws, tutors, plays, etc.)
    CardRevealed {
        /// Who the card belongs to
        owner: PlayerId,
        /// The revealed card info
        card: CardReveal,
        /// Why this card is being revealed
        reason: RevealReason,
        /// **Effective game `action_count` (mtg-610).** The server stamps this
        /// with the `action_count` of the `ChoiceRequest` the reveal is bundled
        /// with (reveals are collected into the choice they precede via
        /// `collect_reveals_since_last_choice`). The shadow's reveal-history
        /// buffer keys the reveal at this value so its application + rewind
        /// reconstruction are a deterministic function of game position rather
        /// than wall-clock reveal/choice-message arrival order. `None` for
        /// reveals sent outside a choice context (e.g. opening-hand), which the
        /// shadow falls back to stamping at the next choice it receives.
        #[serde(default)]
        action_count: Option<u64>,
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
        /// **Game `action_count` at which this reorder takes effect (mtg-752).**
        /// The undo-log position of the reorder's own action (`ShuffleLibrary`
        /// for a shuffle, `ReorderLibrary` for scry/surveil). The shadow keys
        /// this entry in its game-`action_count`-indexed state-sync log so the
        /// new order is adopted at the SAME game position on the forward pass
        /// and on every rewind/replay — the reveal-as-choice alignment contract.
        /// `#[serde(default)]` → 0 for legacy/game-start sync (no prior actions).
        #[serde(default)]
        action_count: u64,
    },

    /// **Library-search candidate reveals (mtg-752 / mtg-253).**
    ///
    /// A single atomic-multi-delta: the N candidate identities a searching
    /// player sees when resolving a `LibrarySearchByName` choice. Replaces the
    /// prior loop of N `CardRevealed` messages all stamped at one search ac —
    /// which would collide in the shadow's game-`action_count`-keyed state-sync
    /// log (`ActionLog::push` requires strictly-increasing keys). One message =
    /// one log entry carrying `Vec<CardReveal>` at the single search-resolution
    /// `action_count`, consistent with "logs aligned modulo reveal-name
    /// visibility": the searcher gets real names; targeting keeps others' views
    /// dummy/masked as before.
    SearchCandidates {
        /// The player searching their own library (sees the real names).
        searcher: PlayerId,
        /// The candidate cards revealed by this search, in library order.
        cards: Vec<CardReveal>,
        /// Game `action_count` of the search-resolution choice these candidates
        /// belong to (the single ac all N share).
        #[serde(default)]
        action_count: u64,
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
        /// **Minimal lazy protocol buffer (mtg-752).** Every reveal-class and
        /// opponent-choice fact with `ac` in `(recipient's last choice,
        /// action_count]`, each at its TRUE game `action_count`, in
        /// ascending-`ac` order. This is the single catch-up payload that
        /// replaces the eager `CardRevealed` / `LibraryReordered` /
        /// `SearchCandidates` / `OpponentChoice` message zoo. A buffer-aware
        /// client routes these into its state-sync + opponent-choice logs and
        /// IGNORES the (still-sent, Phase-1 dual-emit) eager copies.
        /// `#[serde(default)]` → empty for legacy servers/clients.
        #[serde(default)]
        buffer: Vec<(u64, BufferedFact)>,
    },

    /// Notify client of opponent's choice (for sync)
    OpponentChoice {
        /// Choice sequence number
        choice_seq: u32,
        /// Which player made this choice (P1=0, P2=1)
        player: PlayerId,
        /// What type of choice was made
        choice_type: ChoiceType,
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
        /// The structured decision payload (mtg-787): `choice_indices`, the chosen
        /// `spell_ability`, the tutored `library_search_result`, and the chosen
        /// `target_card_ids`. Flattened so the wire JSON is byte-compatible with
        /// the former inline fields. See [`crate::network::ChoicePayload`].
        #[serde(flatten)]
        payload: ChoicePayload,
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
// LOBBY TYPES
// ═══════════════════════════════════════════════════════════════════════════

/// Per-player state inside a waiting room, broadcast in `WaitingRoomUpdate`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WaitingRoomPlayerState {
    /// Display name of this player.
    pub name: String,
    /// `true` iff this player has submitted a valid deck via `SetDeck`.
    pub deck_selected: bool,
    /// Summary of the selected deck (name/count pairs from the main deck).
    /// `None` until the player submits their first `SetDeck`.
    pub deck_summary: Option<Vec<(String, u8)>>,
    /// `true` iff this player has sent `SetReady { ready: true }` with a
    /// valid deck on record.
    pub ready: bool,
}

/// Well-known game name used when a legacy client connects with `Authenticate`.
///
/// First authenticator becomes the creator of this game; second authenticator
/// joins it. New clients should prefer explicit `CreateGame`/`JoinGame` with
/// their own `game_name` so multiple legacy-style sessions can coexist.
pub const DEFAULT_LOBBY_GAME: &str = "default";

/// Filter + pagination parameters for `ClientMessage::ListGames`.
///
/// Strong-typed alternative to a triple of loose fields. All fields are
/// optional on the wire (`#[serde(default)]`) so a client that just wants
/// "page 0 of 20 with no filter" can send `{}`. To return ALL games (the
/// legacy behavior) the client should omit `query` entirely on the parent
/// `ListGames` message — that signals "no pagination at all" to the server.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ListGamesQuery {
    /// Case-insensitive substring matched against `name` OR `creator_name`.
    /// `None` or empty string ⇒ no filter (return all matches for the page).
    #[serde(default)]
    pub filter: Option<String>,
    /// Maximum entries to return. Server clamps to `MAX_LIST_GAMES_LIMIT`
    /// (currently 100). `0` is treated as `DEFAULT_LIST_GAMES_LIMIT` (20).
    #[serde(default)]
    pub limit: u32,
    /// Number of entries to skip (after filtering). Defaults to 0.
    #[serde(default)]
    pub offset: u32,
}

/// Default `limit` when a `ListGamesQuery` arrives with `limit == 0`.
pub const DEFAULT_LIST_GAMES_LIMIT: u32 = 20;
/// Hard ceiling on the server-side `limit` regardless of what the client asks.
pub const MAX_LIST_GAMES_LIMIT: u32 = 100;

/// One entry in the lobby browser response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LobbyGameEntry {
    /// Game name (the key clients use in `JoinGame`).
    pub name: String,
    /// Display name of the player currently waiting.
    pub creator_name: String,
    /// `true` iff this game requires a password to join.
    pub has_password: bool,
    /// Wall-clock ms when the game was created (Unix epoch). Lets the UI
    /// show "waiting for 30s" instead of just "waiting".
    pub created_at_ms: u64,
}

/// Filter + pagination parameters for `ClientMessage::ListPlayers`.
///
/// The players-list analogue of [`ListGamesQuery`]. Kept as a distinct type
/// (rather than reusing `ListGamesQuery`) so the two lists can be tuned and
/// documented independently — the players filter matches a single name field,
/// whereas the games filter matches name OR creator. All fields are optional
/// on the wire (`#[serde(default)]`); send `{}` for "page 0, default limit, no
/// filter". To return ALL players, omit `query` on the parent `ListPlayers`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ListPlayersQuery {
    /// Case-insensitive substring matched against the player name. `None` or
    /// empty string ⇒ no filter (return all matches for the page).
    #[serde(default)]
    pub filter: Option<String>,
    /// Maximum entries to return. Server clamps to `MAX_LIST_PLAYERS_LIMIT`.
    /// `0` is treated as `DEFAULT_LIST_PLAYERS_LIMIT`.
    #[serde(default)]
    pub limit: u32,
    /// Number of entries to skip (after filtering). Defaults to 0.
    #[serde(default)]
    pub offset: u32,
}

/// Default `limit` when a `ListPlayersQuery` arrives with `limit == 0`.
pub const DEFAULT_LIST_PLAYERS_LIMIT: u32 = 20;
/// Hard ceiling on the server-side players `limit` regardless of the request.
pub const MAX_LIST_PLAYERS_LIMIT: u32 = 100;

/// One entry in the lobby players-list response. A registered player is
/// identified solely by display name (no game/password/memory fields), so this
/// is the players analogue of [`LobbyGameEntry`] with just the name.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LobbyPlayerEntry {
    /// Registered display name (the case the client supplied at `Register`).
    pub name: String,
}

/// Reasons a `JoinGame` may fail (other than `ServerFull`).
///
/// We use a closed enum so client UIs can render different messages per case
/// without parsing free-form `Error` strings.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum JoinFailReason {
    /// `game_name` did not match any waiting game. The client may have raced
    /// another joiner (the game already started) or the creator disconnected.
    NotFound,
    /// The game requires a password and the client either omitted it or
    /// supplied the wrong one. We do not distinguish "missing" vs "wrong" so
    /// password presence cannot be probed.
    BadPassword,
    /// The client tried to join a game it created itself (creator and joiner
    /// have the same connection identity — currently determined by
    /// connection-not-yet-detached state). Should be rare but can happen if a
    /// client double-fires `CreateGame` then `JoinGame`.
    AlreadyInGame,
    /// Server password missing or wrong. Distinct from `BadPassword` (which
    /// is per-game).
    BadServerPassword,
    /// Submitted deck failed validation (size, unknown cards, etc.). The
    /// detail string is human-readable.
    InvalidDeck { detail: String },
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
    /// Look at the top N cards of your library and choose top/bottom partition (CR 701.18)
    ///
    /// The server sends the actual top-N CardIds that were revealed
    /// (`revealed_card_ids`) so the client can render them in the
    /// scry UI. CR 701.18 says only the scrying player sees these
    /// cards — by construction the server only ever sends this
    /// `ChoiceType::Scry` to the scrying player's controller, so the
    /// embedded CardIds never leak to the opponent.
    ///
    /// ## Wire encoding (response)
    ///
    /// The client returns `indices` listing the positions (into
    /// `revealed_card_ids`) of the cards that should be put on the
    /// **bottom** of the library. Cards whose positions are not in
    /// `indices` stay on top, in the order they were revealed
    /// (`revealed_card_ids[0]` remains the new top card if not moved).
    ///
    /// Reordering on the bottom is implied by the order of `indices`:
    /// the first index in the response becomes the deepest bottom
    /// card; the last becomes the card just above the bottom.
    Scry {
        /// Number of cards being scried (always equals `revealed_card_ids.len()`,
        /// duplicated for protocol clarity)
        count: usize,
        /// Top-N CardIds of the scrying player's library, top-down.
        /// `revealed_card_ids[0]` is the current top of the library.
        revealed_card_ids: Vec<CardId>,
    },
    /// Look at the top N cards of your library and choose top/graveyard partition (CR 701.42)
    ///
    /// Same visibility rules as [`ChoiceType::Scry`] (only the
    /// surveiling player receives this request).
    ///
    /// ## Wire encoding (response)
    ///
    /// The client returns `indices` listing the positions (into
    /// `revealed_card_ids`) of the cards that should be put into the
    /// **graveyard**. Cards whose positions are not in `indices` stay
    /// on top in revealed order. The order of `indices` is the order
    /// in which the cards are moved to the graveyard (first index in
    /// the response ends up deepest in the graveyard pile).
    Surveil {
        /// Number of cards being surveiled (always equals
        /// `revealed_card_ids.len()`, duplicated for protocol clarity)
        count: usize,
        /// Top-N CardIds of the surveiling player's library, top-down.
        /// `revealed_card_ids[0]` is the current top of the library.
        revealed_card_ids: Vec<CardId>,
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
    /// SMART damage: Choose which blocker to kill first
    ///
    /// When an attacker has multiple blockers but not enough power to kill all,
    /// the controller must choose which blocker to kill first (prioritize lethal).
    LethalDamageAssignment {
        /// The attacking creature assigning damage
        attacker: CardId,
        /// Number of killable blockers to choose from
        killable_count: usize,
        /// Remaining attacker power
        remaining_power: i32,
    },
    /// SMART damage: Choose where to assign remaining non-lethal damage
    ///
    /// After killing all possible blockers, remaining damage must be assigned
    /// to a blocker that can't be killed. Usually doesn't matter strategically.
    RemainingDamageAssignment {
        /// The attacking creature assigning damage
        attacker: CardId,
        /// Number of remaining blockers
        blocker_count: usize,
        /// Damage left to assign (not enough to kill any)
        remaining_damage: i32,
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
            commanders: Vec::new(),
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
    /// Per-battlefield-card detail `(card_id, is_tapped, controller)`, sorted by
    /// card_id — EXACTLY the per-card fields hashed by `compute_view_hash`
    /// (mtg-728 class-A enumeration). When the coarse sizes all match but the
    /// view-hash still diverges, the diverging field is one of these (a tap-status
    /// or controller mismatch on a battlefield card) or `graveyard_ids` below.
    #[serde(default)]
    pub battlefield_detail: Vec<(u32, bool, u32)>,
    /// Per-player graveyard CardIds in order `[P1_gy_ids, P2_gy_ids]` — the
    /// graveyard CONTENTS hashed by `compute_view_hash` (the sizes alone, in
    /// `graveyard_sizes`, can match while the ids differ).
    #[serde(default)]
    pub graveyard_ids: [Vec<u32>; 2],
    /// Per-player LIBRARY CardIds `[P1_lib, P2_lib]` — diagnostic sibling of
    /// `graveyard_ids`. `compute_view_hash` hashes only library SIZE (contents are
    /// private), so a `library_sizes` off-by-one can desync the hash with every
    /// other field byte-identical; dumping the ids names the extra/missing card.
    /// Pinned the mtg-752 library-reorder-resurrection class (robots seed-5: a
    /// permanent cast to the battlefield reappearing in the shadow library).
    #[serde(default)]
    pub library_ids: [Vec<u32>; 2],
    /// Per-player KNOWN hand CardIds `[P0_hand, P1_hand]`, sorted. Unlike
    /// `requesting_player_hand_ids` (only the one requesting player), this dumps
    /// BOTH hands the side can see (the server sees both; a client sees its own
    /// fully + the materialized subset of the opponent's). When `hand_sizes`
    /// differs but every public zone is byte-identical, diffing these names the
    /// exact card the shadow LOST vs a pure stamping skew (mtg-799 seed-7).
    #[serde(default)]
    pub hand_ids: [Vec<u32>; 2],
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
            battlefield_detail: Vec::new(),
            graveyard_ids: [Vec::new(), Vec::new()],
            library_ids: [Vec::new(), Vec::new()],
            hand_ids: [Vec::new(), Vec::new()],
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
    #[allow(clippy::wildcard_enum_match_arm)]
    fn test_bug_report_client_message_serialization() {
        let msg = ClientMessage::BugReport {
            description: "UI froze after mulligan".to_string(),
            game_logs: "[GAMELOG] draw step".to_string(),
            console_logs: "TypeError: undefined is not a function".to_string(),
            trusted_password: Some("trusted".to_string()),
        };

        let json = serde_json::to_string(&msg).expect("serialize");
        let roundtrip: ClientMessage = serde_json::from_str(&json).expect("deserialize");

        match roundtrip {
            ClientMessage::BugReport {
                description,
                game_logs,
                console_logs,
                trusted_password,
            } => {
                assert_eq!(description, "UI froze after mulligan");
                assert_eq!(game_logs, "[GAMELOG] draw step");
                assert_eq!(console_logs, "TypeError: undefined is not a function");
                assert_eq!(trusted_password, Some("trusted".to_string()));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    #[allow(clippy::wildcard_enum_match_arm)]
    fn test_bug_report_two_phase_message_serialization() {
        // Phase 1: disk-write confirmation.
        let stored = ServerMessage::BugReportStored {
            success: true,
            report_dir: Some("bug_reports/1780537595823".to_string()),
            error: None,
        };
        let json = serde_json::to_string(&stored).expect("serialize stored");
        assert!(json.contains("bug_report_stored"), "tag should be snake_case: {json}");
        match serde_json::from_str::<ServerMessage>(&json).expect("deserialize stored") {
            ServerMessage::BugReportStored {
                success,
                report_dir,
                error,
            } => {
                assert!(success);
                assert_eq!(report_dir.as_deref(), Some("bug_reports/1780537595823"));
                assert_eq!(error, None);
            }
            other => panic!("unexpected variant: {other:?}"),
        }

        // Phase 2: GitHub issue result (failure case keeps a reason without a URL).
        let issue = ServerMessage::BugReportIssueResult {
            issue_url: None,
            error: Some("GitHub issue filing timed out after 15 seconds".to_string()),
        };
        let json = serde_json::to_string(&issue).expect("serialize issue");
        assert!(
            json.contains("bug_report_issue_result"),
            "tag should be snake_case: {json}"
        );
        match serde_json::from_str::<ServerMessage>(&json).expect("deserialize issue") {
            ServerMessage::BugReportIssueResult { issue_url, error } => {
                assert_eq!(issue_url, None);
                assert!(error.expect("error message").contains("timed out"));
            }
            other => panic!("unexpected variant: {other:?}"),
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
            buffer: Vec::new(),
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
            ServerMessage::BugReportStored {
                success: true,
                report_dir: Some("bug_reports/1700000000000".to_string()),
                error: None,
            },
            ServerMessage::BugReportIssueResult {
                issue_url: Some("https://github.com/example/repo/issues/7".to_string()),
                error: None,
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
                reconnect_token: None,
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
                buffer: Vec::new(),
            },
            ServerMessage::CardRevealed {
                owner: player_id,
                card: CardReveal {
                    card_id,
                    name: "Lightning Bolt".to_string(),
                    card_def: None,
                },
                reason: RevealReason::Draw,
                action_count: Some(0),
            },
            ServerMessage::OpponentChoice {
                choice_seq: 5,
                player: player_id,
                choice_type: ChoiceType::Priority { available_count: 0 },
                description: "Pass priority".to_string(),
                action_count: 0,
                timestamp_ms: 1234567891,
                payload: ChoicePayload {
                    choice_indices: vec![0],
                    spell_ability: None,
                    library_search_result: None,
                    target_card_ids: None,
                },
                state_hash_after: None,
                debug_info: None,
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
            ClientMessage::BugReport {
                description: "Network desync after combat".to_string(),
                game_logs: "combat log".to_string(),
                console_logs: "console log".to_string(),
                trusted_password: None,
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
            ChoiceType::Scry {
                count: 2,
                revealed_card_ids: vec![CardId::new(101), CardId::new(102)],
            },
            ChoiceType::Surveil {
                count: 3,
                revealed_card_ids: vec![CardId::new(201), CardId::new(202), CardId::new(203)],
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
