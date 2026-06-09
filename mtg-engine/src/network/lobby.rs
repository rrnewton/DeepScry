//! Lobby state for multi-game multiplayer servers.
//!
//! The legacy [`GameServer::run`] loop assumes a single game and a single
//! waiting player. The lobby refactor replaces that with a long-lived
//! `LobbyState` shared across connection-handler tasks:
//!
//! - Each TCP accept spawns its own task immediately (no head-of-line blocking
//!   on a 2-player handshake).
//! - That task reads its first message and hands off to the lobby:
//!   * `Authenticate` → legacy "default" game, treated as create-or-join.
//!   * `CreateGame {..}` → new pending game, registered in `waiting_games`.
//!   * `JoinGame {..}` → match against an existing pending game.
//!   * `ListGames` → snapshot reply, connection stays open.
//! - When two players are matched the lobby moves the entry from
//!   `waiting_games` to `active_games` and spawns the per-game task. The task
//!   removes its own entry on completion (best-effort cleanup).
//!
//! Capacity is gated by host memory pressure (see [`super::memory`]), not a
//! fixed `max_games`. When `(MemTotal-MemAvailable)/MemTotal*100` exceeds the
//! configured `max_memory_percent`, new joins are rejected with
//! `ServerMessage::ServerFull` until pressure drops. Existing games are
//! never killed by the gate — only new admissions.
//!
//! ## Per-game lifetime cap
//!
//! Each spawned game also runs under a wall-clock timeout (default 4 hours).
//! Without one a desync or a stuck client could keep an `Arc<GameState>`
//! resident forever, eventually starving the memory ceiling. On timeout the
//! game task aborts and removes itself from the registry.

use crate::network::protocol::{
    DeckSubmission, ListGamesQuery, ListPlayersQuery, LobbyGameEntry, LobbyPlayerEntry, ReconnectToken, ServerMessage,
    WaitingRoomPlayerState, DEFAULT_LIST_GAMES_LIMIT, DEFAULT_LIST_PLAYERS_LIMIT, MAX_LIST_GAMES_LIMIT,
    MAX_LIST_PLAYERS_LIMIT,
};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::TcpStream;
use tokio::sync::{oneshot, Mutex};
use tokio_tungstenite::WebSocketStream;

/// Default per-game wall-clock cap (4 hours). Exceeding this aborts the game
/// task and frees its `GameState`. Tournament-length matches comfortably fit
/// in this budget.
pub const DEFAULT_GAME_TIMEOUT: Duration = Duration::from_secs(4 * 60 * 60);

/// Default `max_memory_percent` ceiling.
///
/// 80 leaves comfortable headroom for the kernel page cache and any other
/// processes on the host. Operators tuning for a dedicated VM can raise this
/// (e.g., 90) at the cost of less reserve for spike allocations.
pub const DEFAULT_MAX_MEMORY_PERCENT: u32 = 80;

/// Identifier for a running or pending game. Monotonic, unique per server
/// process. We keep these out of the protocol — clients address games by
/// `name` (a string) so they can be human-typed.
pub type GameId = u64;

/// Information passed from the joiner's task to the creator's task at the
/// moment a game is matched.
///
/// The creator's connection task awaits a `oneshot::Receiver<JoinedPlayer>`
/// it left in [`PendingGame`]; the joiner's task removes the pending entry
/// and sends one of these through the corresponding `Sender`. The joiner's
/// task then exits, and the creator's task spawns the per-game task with
/// both WebSocket streams.
#[derive(Debug)]
pub struct JoinedPlayer {
    /// Display name the joiner picked (or the server-assigned default).
    pub name: String,
    /// Joiner's deck submission.
    pub deck: DeckSubmission,
    /// Joiner's still-open WebSocket. The creator's task takes ownership.
    pub ws_stream: WebSocketStream<TcpStream>,
}

/// Per-player mutable state inside a waiting game room.
///
/// Tracked server-side so both players always agree on the current deck
/// selections and ready flags before the match starts.
#[derive(Debug, Clone, Default)]
pub struct WaitingPlayerState {
    /// Most recent deck submitted via `SetDeck`, or `None` if not yet set.
    pub deck: Option<DeckSubmission>,
    /// `true` iff the player sent `SetReady { ready: true }` with a valid
    /// deck on record. Reset to `false` whenever a new `SetDeck` is received.
    pub ready: bool,
}

impl WaitingPlayerState {
    /// Produce the protocol snapshot of this player's state.
    pub fn to_protocol_state(&self, name: &str) -> WaitingRoomPlayerState {
        WaitingRoomPlayerState {
            name: name.to_string(),
            deck_selected: self.deck.is_some(),
            deck_summary: self.deck.as_ref().map(|d| d.main_deck.clone()),
            ready: self.ready,
        }
    }
}

/// Information about a game that is waiting for its second player.
///
/// We do not store the creator's WebSocket here — it stays with the creator's
/// connection task. Instead we hold a `oneshot::Sender<JoinedPlayer>` so the
/// joiner can hand its WebSocket over. This keeps each socket pinned to
/// exactly one task at any moment, which avoids surprising borrows.
///
/// `Debug` is `derive`d, but we manually skip `handoff_tx` (oneshot::Sender
/// already implements Debug, so no work needed — included for clarity).
#[derive(Debug)]
pub struct PendingGame {
    /// Stable id used for logging.
    pub id: GameId,
    /// Human-typed name used as the `JoinGame` key. Stored as the original
    /// case for display; the map key is `name.to_lowercase()` so joins are
    /// case-insensitive.
    pub name: String,
    /// Display name of the creator, as the lobby UI should show it.
    pub creator_name: String,
    /// `true` iff a per-game password was set by the creator.
    pub has_password: bool,
    /// Hash of the per-game password (`None` if the creator did not set one).
    /// We hash so we never keep the plaintext in process memory longer than
    /// necessary; the `JoinGame` path hashes the supplied password and
    /// compares. A single non-cryptographic hash is fine here — passwords
    /// only protect a lobby slot from accidental collisions, not secrets.
    pub password_hash: Option<u64>,
    /// `Instant` the entry was inserted; used by the watchdog to garbage
    /// collect stale waiting games (e.g., creator never finished sideboarding).
    pub created_at: Instant,
    /// Wall-clock ms used in the lobby wire format for "waiting for X".
    pub created_at_ms: u64,
    /// Creator's current deck + ready state (updated by `SetDeck`/`SetReady`).
    pub creator_state: WaitingPlayerState,
    /// Joiner's current deck + ready state. `None` until a second player joins.
    pub joiner_state: Option<WaitingPlayerState>,
    /// Joiner's display name. `None` until the second player joins.
    pub joiner_name: Option<String>,
    /// Sender used to deliver a `WaitingRoomUpdate` to the creator's task
    /// when the joiner (or joiner's state) changes.
    ///
    /// The creator's task receives these updates and forwards
    /// `ServerMessage::WaitingRoomUpdate` to the creator's WebSocket.
    pub creator_update_tx: Option<tokio::sync::watch::Sender<WaitingRoomSnapshot>>,
    /// Hand-off channel — see [`JoinedPlayer`]. `None` after the joiner takes
    /// it (which is also the moment the entry is removed from the map; this
    /// field exists as `Option` only so the Sender can be moved out without
    /// destructuring the surrounding `PendingGame`).
    pub handoff_tx: Option<oneshot::Sender<JoinedPlayer>>,
    /// `true` iff this game was created by a launcher waiting room (Variant 1
    /// rendezvous, mtg-682): the game must NOT auto-start when the joiner
    /// arrives; both players must `SetReady` first, after which the server
    /// emits `WaitingRoomReady` to both and frees the slot. `false` (legacy /
    /// game-page clients) starts the game immediately on join.
    pub rendezvous: bool,
}

/// A point-in-time snapshot of the waiting room state, distributed to both
/// players via the `creator_update_tx` watch channel whenever anything
/// changes. The server serialises this into `ServerMessage::WaitingRoomUpdate`.
#[derive(Debug, Clone, Default)]
pub struct WaitingRoomSnapshot {
    pub creator_name: String,
    pub creator_state: WaitingPlayerState,
    pub joiner_name: Option<String>,
    pub joiner_state: Option<WaitingPlayerState>,
    /// Internal-only flag (NOT serialized): set `true` on the snapshot that
    /// crosses the both-ready threshold in a Variant-1 rendezvous waiting room
    /// (mtg-682). The peer task watching this channel reacts by sending
    /// `WaitingRoomReady` to its own socket and exiting. The wire
    /// `WaitingRoomUpdate` produced by [`to_server_message`] never carries it.
    pub start_game: bool,
}

impl WaitingRoomSnapshot {
    /// Convert to the wire message.
    pub fn to_server_message(&self) -> ServerMessage {
        ServerMessage::WaitingRoomUpdate {
            creator: self.creator_state.to_protocol_state(&self.creator_name),
            joiner: self.joiner_name.as_deref().map(|name| {
                self.joiner_state
                    .as_ref()
                    .map(|s| s.to_protocol_state(name))
                    .unwrap_or_else(|| WaitingRoomPlayerState {
                        name: name.to_string(),
                        deck_selected: false,
                        deck_summary: None,
                        ready: false,
                    })
            }),
        }
    }
}

/// Information about a game that is currently being played.
///
/// Carries no game state — that lives inside the spawned game task. We track
/// active games so `ListGames` can return accurate "in progress" totals and
/// so the watchdog can enforce per-game timeouts. Reconnect tokens are stored
/// here so a dropped player can re-authenticate without restarting the game.
#[derive(Debug)]
pub struct ActiveGame {
    pub id: GameId,
    pub name: String,
    pub p1_name: String,
    pub p2_name: String,
    pub started_at: Instant,
    /// Reconnect token for P1 (creator). `None` until the game has started.
    pub p1_reconnect_token: Option<ReconnectToken>,
    /// Reconnect token for P2 (joiner). `None` until the game has started.
    pub p2_reconnect_token: Option<ReconnectToken>,
}

/// Registered username entry: name → held until the owning connection drops.
///
/// We store the lowercased canonical form as the map key; the display-case
/// form is stored in the value. Uniqueness is enforced case-insensitively so
/// "Alice" and "alice" collide. The `connection_id` is the monotonic
/// per-accept counter used to associate the reservation with a specific WS
/// connection so the cleanup path can verify ownership.
#[derive(Debug, Clone)]
pub struct RegisteredName {
    /// Display-case name as the client submitted it.
    pub display_name: String,
    /// Monotonic connection identifier assigned at accept time.
    pub connection_id: u64,
}

/// Mutable lobby state, shared via `Arc<Mutex<...>>` between the accept loop
/// and per-connection tasks.
#[derive(Debug, Default)]
pub struct LobbyState {
    /// Pre-game lobby slots, keyed by `name.to_lowercase()`.
    pub waiting_games: HashMap<String, PendingGame>,
    /// In-flight games, keyed by id.
    pub active_games: HashMap<GameId, ActiveGame>,
    /// Monotonic counter for `next_game_id()`.
    pub next_game_id: GameId,
    /// Monotonic counter for assigning `connection_id` values.
    pub next_connection_id: u64,
    /// Registered display names, keyed by lowercased canonical form.
    ///
    /// A connection that holds a registration here owns that name until it
    /// disconnects or explicitly deregisters. The connection task calls
    /// [`LobbyState::release_name`] in its `Drop`/cleanup path.
    pub registered_names: HashMap<String, RegisteredName>,
}

impl LobbyState {
    /// Create an empty lobby. The first id handed out is `1` (matches the
    /// legacy server behaviour).
    pub fn new() -> Self {
        Self {
            waiting_games: HashMap::new(),
            active_games: HashMap::new(),
            next_game_id: 1,
            next_connection_id: 1,
            registered_names: HashMap::new(),
        }
    }

    /// Allocate the next monotonic connection identifier.
    pub fn next_connection_id(&mut self) -> u64 {
        let id = self.next_connection_id;
        self.next_connection_id += 1;
        id
    }

    /// Try to register a display name for the given connection.
    ///
    /// Returns `Ok(())` on success. Returns `Err(reason)` — a human-readable
    /// explanation — when the name fails validation or is already taken
    /// (case-insensitively) by another connection.
    ///
    /// # Errors
    ///
    /// Returns `Err(String)` (non-fatal, wire-ready) in the following cases:
    /// - Name is empty or contains only whitespace.
    /// - Name exceeds 32 characters.
    /// - Name contains non-printable-ASCII characters.
    /// - The lowercased name is already registered by another connection.
    ///
    /// Validation rules:
    /// - Non-empty, ≤ 32 characters.
    /// - Contains at least one non-whitespace character (no blank names).
    /// - Only printable ASCII (`0x20..=0x7E`).
    pub fn try_register_name(&mut self, display_name: &str, connection_id: u64) -> Result<(), String> {
        // Validate format.
        if display_name.is_empty() {
            return Err("Name must not be empty".to_string());
        }
        if display_name.len() > 32 {
            return Err(format!("Name too long ({} chars; max 32)", display_name.len()));
        }
        if !display_name.chars().all(|c| (' '..='~').contains(&c)) {
            return Err("Name must contain only printable ASCII characters".to_string());
        }
        if display_name.trim().is_empty() {
            return Err("Name must not be blank".to_string());
        }

        let key = display_name.to_lowercase();
        if let Some(existing) = self.registered_names.get(&key) {
            return Err(format!("Name '{}' is already taken", existing.display_name));
        }
        self.registered_names.insert(
            key,
            RegisteredName {
                display_name: display_name.to_string(),
                connection_id,
            },
        );
        Ok(())
    }

    /// Release a name reservation held by the given connection.
    ///
    /// No-op if the name is not registered, or if it was taken over by a
    /// different connection (should not happen in practice — defensive guard).
    pub fn release_name(&mut self, display_name: &str, connection_id: u64) {
        let key = display_name.to_lowercase();
        if let Some(entry) = self.registered_names.get(&key) {
            if entry.connection_id == connection_id {
                self.registered_names.remove(&key);
            }
        }
    }

    /// Look up the canonical (display-case) registered name for a connection,
    /// if one exists.
    pub fn registered_name_for(&self, connection_id: u64) -> Option<&str> {
        self.registered_names
            .values()
            .find(|r| r.connection_id == connection_id)
            .map(|r| r.display_name.as_str())
    }

    /// Look up an active game's reconnect token by name + player id.
    ///
    /// Returns `None` if no such game exists or the token has not been set yet.
    pub fn find_reconnect_token(&self, game_name: &str, player_index: usize) -> Option<&ReconnectToken> {
        let key = game_name.to_lowercase();
        let game = self.active_games.values().find(|g| g.name.to_lowercase() == key)?;
        match player_index {
            0 => game.p1_reconnect_token.as_ref(),
            1 => game.p2_reconnect_token.as_ref(),
            _ => None,
        }
    }

    /// Validate a reconnect token against a named active game.
    ///
    /// Returns `Some(player_index)` (0 = P1, 1 = P2) if the token is valid,
    /// `None` if the game does not exist or the token does not match either
    /// player.
    pub fn validate_reconnect_token(&self, game_name: &str, token: &ReconnectToken) -> Option<(GameId, usize)> {
        let key = game_name.to_lowercase();
        let game = self.active_games.values().find(|g| g.name.to_lowercase() == key)?;
        if game.p1_reconnect_token.as_ref() == Some(token) {
            return Some((game.id, 0));
        }
        if game.p2_reconnect_token.as_ref() == Some(token) {
            return Some((game.id, 1));
        }
        None
    }

    /// Allocate the next game id.
    pub fn next_game_id(&mut self) -> GameId {
        let id = self.next_game_id;
        self.next_game_id += 1;
        id
    }

    /// Generate a default game name when the client did not supply one.
    /// Avoids collisions with existing waiting games by using the next game
    /// id as a numeric suffix.
    pub fn default_game_name(&mut self) -> String {
        // Don't consume an id just to name a game — peek instead.
        format!("game-{}", self.next_game_id)
    }

    /// Snapshot of `waiting_games` for the legacy `ListGames` reply
    /// (no filter, no pagination). Equivalent to
    /// [`Self::list_waiting_paged`] with `query = None`.
    pub fn list_waiting(&self) -> Vec<LobbyGameEntry> {
        let (out, _total) = self.list_waiting_paged(None);
        out
    }

    /// Snapshot of `waiting_games` with optional case-insensitive substring
    /// filter (against game name OR creator name) and pagination.
    ///
    /// Returns `(page, total_matching)` where `page.len() <= limit` and
    /// `total_matching` is the count after filtering but before paging.
    ///
    /// When `query` is `None` the legacy behavior applies: every waiting game
    /// is returned with no clamp.
    pub fn list_waiting_paged(&self, query: Option<&ListGamesQuery>) -> (Vec<LobbyGameEntry>, u32) {
        let mut all: Vec<LobbyGameEntry> = self
            .waiting_games
            .values()
            .map(|pg| LobbyGameEntry {
                name: pg.name.clone(),
                creator_name: pg.creator_name.clone(),
                has_password: pg.has_password,
                created_at_ms: pg.created_at_ms,
            })
            .collect();
        // Stable order for tests / clients: by creation time ascending.
        all.sort_by_key(|e| e.created_at_ms);

        let Some(q) = query else {
            let total = all.len() as u32;
            return (all, total);
        };

        // Filter (case-insensitive substring on game name OR creator name).
        // Note: this is straight substring matching on free-text user input,
        // which is the right tool — NOT structured-data parsing.
        if let Some(needle) = q.filter.as_deref().filter(|s| !s.is_empty()) {
            let needle_lc = needle.to_lowercase();
            all.retain(|e| {
                e.name.to_lowercase().contains(&needle_lc) || e.creator_name.to_lowercase().contains(&needle_lc)
            });
        }
        let total = all.len() as u32;

        // Paginate.
        let limit = if q.limit == 0 {
            DEFAULT_LIST_GAMES_LIMIT
        } else {
            q.limit.min(MAX_LIST_GAMES_LIMIT)
        };
        let offset = q.offset as usize;
        let end = (offset + limit as usize).min(all.len());
        let page = if offset >= all.len() {
            Vec::new()
        } else {
            all[offset..end].to_vec()
        };
        (page, total)
    }

    /// Snapshot of registered (logged-in) players with optional
    /// case-insensitive substring filter on the name and pagination.
    ///
    /// The players analogue of [`list_waiting_paged`](Self::list_waiting_paged):
    /// returns `(page, total_matching)` where `page.len() <= limit` and
    /// `total_matching` is the count after filtering but before paging. When
    /// `query` is `None`, every registered player is returned with no clamp.
    pub fn list_players_paged(&self, query: Option<&ListPlayersQuery>) -> (Vec<LobbyPlayerEntry>, u32) {
        let mut all: Vec<LobbyPlayerEntry> = self
            .registered_names
            .values()
            .map(|r| LobbyPlayerEntry {
                name: r.display_name.clone(),
            })
            .collect();
        // Stable order for tests / clients: case-insensitive by display name.
        all.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

        let Some(q) = query else {
            let total = all.len() as u32;
            return (all, total);
        };

        // Filter (case-insensitive substring on player name). This is
        // free-text user input, so substring matching is the right tool —
        // NOT structured-data parsing.
        if let Some(needle) = q.filter.as_deref().filter(|s| !s.is_empty()) {
            let needle_lc = needle.to_lowercase();
            all.retain(|e| e.name.to_lowercase().contains(&needle_lc));
        }
        let total = all.len() as u32;

        // Paginate.
        let limit = if q.limit == 0 {
            DEFAULT_LIST_PLAYERS_LIMIT
        } else {
            q.limit.min(MAX_LIST_PLAYERS_LIMIT)
        };
        let offset = q.offset as usize;
        let end = (offset + limit as usize).min(all.len());
        let page = if offset >= all.len() {
            Vec::new()
        } else {
            all[offset..end].to_vec()
        };
        (page, total)
    }

    /// Number of games currently being played.
    pub fn active_count(&self) -> usize {
        self.active_games.len()
    }

    /// Number of games waiting for a second player.
    pub fn waiting_count(&self) -> usize {
        self.waiting_games.len()
    }
}

/// Cheap hasher for the per-game password. Not cryptographically secure;
/// see `PendingGame::password_hash` for the rationale.
pub fn hash_game_password(plain: &str) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    plain.hash(&mut h);
    h.finish()
}

/// Build the rejection message for a memory-pressure refusal.
///
/// Centralised so the legacy `Authenticate` path and the new
/// `CreateGame`/`JoinGame` paths produce identical wire output.
///
/// SECURITY: The client-visible `reason` is intentionally generic. Detailed
/// memory metrics (`used_percent`, `ceiling_percent`) are logged server-side
/// at `warn` level only — they are never put on the wire. Leaking host
/// memory percentages to anonymous WebSocket peers would help attackers
/// fingerprint the host and time DoS pressure. See `protocol::ServerMessage::ServerFull`.
pub fn build_server_full_message(used_percent: Option<u32>, ceiling_percent: u32) -> ServerMessage {
    // Operator-visible diagnostic — stays in server logs only.
    match used_percent {
        Some(used) => {
            log::warn!("Rejecting connection: host memory at {used}% used (ceiling {ceiling_percent}%)");
        }
        None => {
            log::warn!("Rejecting connection: host memory cannot be measured (ceiling {ceiling_percent}%)");
        }
    }

    ServerMessage::ServerFull {
        reason: "Server is full, try again later.".to_string(),
    }
}

/// Convenience: lock-free "shared lobby" type alias. Mainly for keeping
/// signatures readable in `server.rs`.
pub type SharedLobby = Arc<Mutex<LobbyState>>;

/// Construct an empty shared lobby.
pub fn new_shared_lobby() -> SharedLobby {
    Arc::new(Mutex::new(LobbyState::new()))
}

#[cfg(test)]
// Unit tests pattern-match the one variant they care about and bail on
// anything else; the wildcard arm IS the assertion. Spelling out every other
// variant would add noise without changing semantics.
#[allow(clippy::wildcard_enum_match_arm)]
mod tests {
    use super::*;

    fn pending(id: GameId, name: &str, created_ms: u64, has_pw: bool) -> PendingGame {
        let (tx, _rx) = oneshot::channel();
        let (update_tx, _update_rx) = tokio::sync::watch::channel(WaitingRoomSnapshot::default());
        PendingGame {
            id,
            name: name.to_string(),
            creator_name: format!("Creator{id}"),
            has_password: has_pw,
            password_hash: has_pw.then(|| hash_game_password("secret")),
            created_at: Instant::now(),
            created_at_ms: created_ms,
            creator_state: WaitingPlayerState::default(),
            joiner_state: None,
            joiner_name: None,
            creator_update_tx: Some(update_tx),
            handoff_tx: Some(tx),
            rendezvous: false,
        }
    }

    #[test]
    fn next_game_id_is_monotonic_starting_at_one() {
        let mut s = LobbyState::new();
        assert_eq!(s.next_game_id(), 1);
        assert_eq!(s.next_game_id(), 2);
        assert_eq!(s.next_game_id(), 3);
    }

    #[test]
    fn next_connection_id_is_monotonic_starting_at_one() {
        let mut s = LobbyState::new();
        assert_eq!(s.next_connection_id(), 1);
        assert_eq!(s.next_connection_id(), 2);
        assert_eq!(s.next_connection_id(), 3);
    }

    // ── Name registration ────────────────────────────────────────────────────

    #[test]
    fn register_name_succeeds_for_unique_name() {
        let mut s = LobbyState::new();
        let conn_id = s.next_connection_id();
        assert!(s.try_register_name("Alice", conn_id).is_ok());
        assert!(s.registered_names.contains_key("alice"));
    }

    #[test]
    fn register_name_rejects_duplicate_case_insensitively() {
        let mut s = LobbyState::new();
        let conn1 = s.next_connection_id();
        s.try_register_name("Alice", conn1).unwrap();
        let conn2 = s.next_connection_id();
        let err = s.try_register_name("ALICE", conn2).unwrap_err();
        assert!(err.contains("Alice"), "error should mention the existing holder: {err}");
    }

    #[test]
    fn register_name_rejects_empty_name() {
        let mut s = LobbyState::new();
        let conn = s.next_connection_id();
        assert!(s.try_register_name("", conn).is_err());
    }

    #[test]
    fn register_name_rejects_blank_name() {
        let mut s = LobbyState::new();
        let conn = s.next_connection_id();
        assert!(s.try_register_name("   ", conn).is_err());
    }

    #[test]
    fn register_name_rejects_too_long_name() {
        let mut s = LobbyState::new();
        let conn = s.next_connection_id();
        let long_name = "A".repeat(33);
        let err = s.try_register_name(&long_name, conn).unwrap_err();
        assert!(err.contains("too long"), "error: {err}");
    }

    #[test]
    fn register_name_rejects_non_printable_ascii() {
        let mut s = LobbyState::new();
        let conn = s.next_connection_id();
        // Tab character is not in ' '..='~' (printable range)
        assert!(s.try_register_name("Alice\tSmith", conn).is_err());
    }

    #[test]
    fn release_name_frees_reservation() {
        let mut s = LobbyState::new();
        let conn = s.next_connection_id();
        s.try_register_name("Alice", conn).unwrap();
        s.release_name("Alice", conn);
        assert!(!s.registered_names.contains_key("alice"));
        // Another connection can now claim it.
        let conn2 = s.next_connection_id();
        assert!(s.try_register_name("Alice", conn2).is_ok());
    }

    #[test]
    fn release_name_is_noop_for_wrong_connection() {
        let mut s = LobbyState::new();
        let conn1 = s.next_connection_id();
        s.try_register_name("Alice", conn1).unwrap();
        let conn2 = s.next_connection_id();
        // conn2 does NOT own the name — release should not remove it.
        s.release_name("Alice", conn2);
        assert!(
            s.registered_names.contains_key("alice"),
            "name should still be reserved"
        );
    }

    #[test]
    fn registered_name_for_returns_display_name() {
        let mut s = LobbyState::new();
        let conn = s.next_connection_id();
        s.try_register_name("MixedCase", conn).unwrap();
        assert_eq!(s.registered_name_for(conn), Some("MixedCase"));
        assert_eq!(s.registered_name_for(conn + 99), None);
    }

    // ── Reconnect token validation ───────────────────────────────────────────

    #[test]
    fn validate_reconnect_token_accepts_valid_p1_token() {
        let mut s = LobbyState::new();
        let token = ReconnectToken("abcd1234abcd1234abcd1234abcd1234".to_string());
        s.active_games.insert(
            1,
            ActiveGame {
                id: 1,
                name: "my-game".to_string(),
                p1_name: "alice".to_string(),
                p2_name: "bob".to_string(),
                started_at: Instant::now(),
                p1_reconnect_token: Some(token.clone()),
                p2_reconnect_token: None,
            },
        );
        assert_eq!(s.validate_reconnect_token("my-game", &token), Some((1, 0)));
        assert_eq!(s.validate_reconnect_token("MY-GAME", &token), Some((1, 0)));
    }

    #[test]
    fn validate_reconnect_token_accepts_valid_p2_token() {
        let mut s = LobbyState::new();
        let p2_token = ReconnectToken("ff00ff00ff00ff00ff00ff00ff00ff00".to_string());
        s.active_games.insert(
            2,
            ActiveGame {
                id: 2,
                name: "other-game".to_string(),
                p1_name: "alice".to_string(),
                p2_name: "bob".to_string(),
                started_at: Instant::now(),
                p1_reconnect_token: None,
                p2_reconnect_token: Some(p2_token.clone()),
            },
        );
        assert_eq!(s.validate_reconnect_token("other-game", &p2_token), Some((2, 1)));
    }

    #[test]
    fn validate_reconnect_token_rejects_wrong_token() {
        let mut s = LobbyState::new();
        let real = ReconnectToken("aaaa0000aaaa0000aaaa0000aaaa0000".to_string());
        let fake = ReconnectToken("bbbb1111bbbb1111bbbb1111bbbb1111".to_string());
        s.active_games.insert(
            3,
            ActiveGame {
                id: 3,
                name: "game3".to_string(),
                p1_name: "alice".to_string(),
                p2_name: "bob".to_string(),
                started_at: Instant::now(),
                p1_reconnect_token: Some(real),
                p2_reconnect_token: None,
            },
        );
        assert_eq!(s.validate_reconnect_token("game3", &fake), None);
    }

    #[test]
    fn validate_reconnect_token_rejects_unknown_game() {
        let s = LobbyState::new();
        let token = ReconnectToken("aaaa0000aaaa0000aaaa0000aaaa0000".to_string());
        assert_eq!(s.validate_reconnect_token("no-such-game", &token), None);
    }

    // ── WaitingPlayerState / WaitingRoomSnapshot ─────────────────────────────

    #[test]
    fn waiting_player_state_default_is_not_ready() {
        let state = WaitingPlayerState::default();
        assert!(!state.ready);
        assert!(state.deck.is_none());
        let proto = state.to_protocol_state("Alice");
        assert_eq!(proto.name, "Alice");
        assert!(!proto.deck_selected);
        assert!(!proto.ready);
        assert!(proto.deck_summary.is_none());
    }

    #[test]
    fn waiting_room_snapshot_to_server_message_without_joiner() {
        let snap = WaitingRoomSnapshot {
            creator_name: "Alice".to_string(),
            creator_state: WaitingPlayerState::default(),
            joiner_name: None,
            joiner_state: None,
            start_game: false,
        };
        let msg = snap.to_server_message();
        match msg {
            ServerMessage::WaitingRoomUpdate { creator, joiner } => {
                assert_eq!(creator.name, "Alice");
                assert!(joiner.is_none());
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn waiting_room_snapshot_to_server_message_with_joiner() {
        let snap = WaitingRoomSnapshot {
            creator_name: "Alice".to_string(),
            creator_state: WaitingPlayerState::default(),
            joiner_name: Some("Bob".to_string()),
            joiner_state: Some(WaitingPlayerState::default()),
            start_game: false,
        };
        let msg = snap.to_server_message();
        match msg {
            ServerMessage::WaitingRoomUpdate { creator: _, joiner } => {
                let j = joiner.expect("joiner should be present");
                assert_eq!(j.name, "Bob");
                assert!(!j.ready);
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn default_game_name_uses_next_id_without_consuming() {
        let mut s = LobbyState::new();
        assert_eq!(s.default_game_name(), "game-1");
        assert_eq!(s.next_game_id(), 1, "naming must not consume the id");
        assert_eq!(s.default_game_name(), "game-2");
    }

    #[test]
    fn list_waiting_is_sorted_by_creation_time() {
        let mut s = LobbyState::new();
        s.waiting_games.insert("b".to_string(), pending(1, "b", 200, false));
        s.waiting_games.insert("a".to_string(), pending(2, "a", 100, true));
        s.waiting_games.insert("c".to_string(), pending(3, "c", 300, false));
        let list = s.list_waiting();
        assert_eq!(
            list.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            vec!["a", "b", "c"]
        );
        assert!(list[0].has_password);
        assert!(!list[1].has_password);
    }

    #[test]
    fn list_waiting_paged_filters_case_insensitive_on_name_or_creator() {
        let mut s = LobbyState::new();
        // Names: alpha-game, bravo-game, charlie-game
        // Creators: Creator1, Creator2, Creator3 (from `pending` helper)
        s.waiting_games
            .insert("alpha-game".into(), pending(1, "alpha-game", 100, false));
        s.waiting_games
            .insert("BRAVO-game".into(), pending(2, "BRAVO-game", 200, false));
        s.waiting_games
            .insert("charlie-game".into(), pending(3, "charlie-game", 300, false));

        // Filter on name substring, case-insensitive.
        let q = ListGamesQuery {
            filter: Some("BRAVO".into()),
            limit: 0,
            offset: 0,
        };
        let (page, total) = s.list_waiting_paged(Some(&q));
        assert_eq!(total, 1);
        assert_eq!(page.len(), 1);
        assert_eq!(page[0].name, "BRAVO-game");

        // Filter on creator name (Creator2).
        let q = ListGamesQuery {
            filter: Some("creator2".into()),
            limit: 0,
            offset: 0,
        };
        let (page, total) = s.list_waiting_paged(Some(&q));
        assert_eq!(total, 1);
        assert_eq!(page[0].creator_name, "Creator2");

        // Empty filter = no filter (matches all).
        let q = ListGamesQuery {
            filter: Some(String::new()),
            limit: 0,
            offset: 0,
        };
        let (_, total) = s.list_waiting_paged(Some(&q));
        assert_eq!(total, 3);
    }

    #[test]
    fn list_waiting_paged_respects_limit_and_offset() {
        let mut s = LobbyState::new();
        for i in 0..5 {
            let name = format!("g{i}");
            s.waiting_games
                .insert(name.clone(), pending(i + 1, &name, 100 + i, false));
        }
        let q = ListGamesQuery {
            filter: None,
            limit: 2,
            offset: 1,
        };
        let (page, total) = s.list_waiting_paged(Some(&q));
        assert_eq!(total, 5);
        assert_eq!(page.len(), 2);
        assert_eq!(page[0].name, "g1");
        assert_eq!(page[1].name, "g2");
    }

    #[test]
    fn list_waiting_paged_clamps_limit_to_max() {
        let mut s = LobbyState::new();
        s.waiting_games.insert("x".into(), pending(1, "x", 100, false));
        let q = ListGamesQuery {
            filter: None,
            limit: 9999, // far above MAX_LIST_GAMES_LIMIT
            offset: 0,
        };
        let (page, total) = s.list_waiting_paged(Some(&q));
        assert_eq!(page.len(), 1);
        assert_eq!(total, 1);
    }

    #[test]
    fn list_waiting_paged_none_query_returns_all() {
        let mut s = LobbyState::new();
        for i in 0..3 {
            let name = format!("g{i}");
            s.waiting_games
                .insert(name.clone(), pending(i + 1, &name, 100 + i, false));
        }
        let (page, total) = s.list_waiting_paged(None);
        assert_eq!(page.len(), 3);
        assert_eq!(total, 3);
    }

    /// Register `n` distinct names on distinct connection ids; returns the
    /// lobby. Mirrors the `pending`-helper style used by the games tests.
    fn lobby_with_players(names: &[&str]) -> LobbyState {
        let mut s = LobbyState::new();
        for (i, name) in names.iter().enumerate() {
            s.try_register_name(name, i as u64 + 1)
                .unwrap_or_else(|e| panic!("register {name} failed: {e}"));
        }
        s
    }

    #[test]
    fn list_players_paged_filters_case_insensitive_on_name() {
        let s = lobby_with_players(&["Alice", "BOB", "carol"]);

        // Case-insensitive substring on the player name.
        let q = ListPlayersQuery {
            filter: Some("bo".into()),
            limit: 0,
            offset: 0,
        };
        let (page, total) = s.list_players_paged(Some(&q));
        assert_eq!(total, 1);
        assert_eq!(page.len(), 1);
        assert_eq!(page[0].name, "BOB");

        // Empty filter = no filter (matches all).
        let q = ListPlayersQuery {
            filter: Some(String::new()),
            limit: 0,
            offset: 0,
        };
        let (_, total) = s.list_players_paged(Some(&q));
        assert_eq!(total, 3);
    }

    #[test]
    fn list_players_paged_respects_limit_and_offset() {
        // Names sort case-insensitively: p0, p1, p2, p3, p4.
        let s = lobby_with_players(&["p0", "p1", "p2", "p3", "p4"]);
        let q = ListPlayersQuery {
            filter: None,
            limit: 2,
            offset: 1,
        };
        let (page, total) = s.list_players_paged(Some(&q));
        assert_eq!(total, 5);
        assert_eq!(page.len(), 2);
        assert_eq!(page[0].name, "p1");
        assert_eq!(page[1].name, "p2");
    }

    #[test]
    fn list_players_paged_clamps_limit_to_max() {
        let s = lobby_with_players(&["solo"]);
        let q = ListPlayersQuery {
            filter: None,
            limit: 9999, // far above MAX_LIST_PLAYERS_LIMIT
            offset: 0,
        };
        let (page, total) = s.list_players_paged(Some(&q));
        assert_eq!(page.len(), 1);
        assert_eq!(total, 1);
    }

    #[test]
    fn list_players_paged_none_query_returns_all() {
        let s = lobby_with_players(&["a", "b", "c"]);
        let (page, total) = s.list_players_paged(None);
        assert_eq!(page.len(), 3);
        assert_eq!(total, 3);
    }

    #[test]
    fn list_players_paged_offset_past_end_is_empty() {
        let s = lobby_with_players(&["only"]);
        let q = ListPlayersQuery {
            filter: None,
            limit: 10,
            offset: 5,
        };
        let (page, total) = s.list_players_paged(Some(&q));
        assert!(page.is_empty());
        assert_eq!(total, 1);
    }

    #[test]
    fn hash_game_password_is_deterministic_per_string() {
        assert_eq!(hash_game_password("hunter2"), hash_game_password("hunter2"));
        assert_ne!(hash_game_password("hunter2"), hash_game_password("hunter3"));
    }

    /// SECURITY: the wire payload must be a fixed, generic string. Memory
    /// percentages and the configured ceiling must NOT appear in the
    /// client-visible `reason` — they are logged server-side only.
    #[test]
    fn build_server_full_message_does_not_leak_host_memory() {
        for (used, ceiling) in [(Some(85u32), 80u32), (None, 90u32), (Some(99), 1)] {
            let msg = build_server_full_message(used, ceiling);
            match msg {
                ServerMessage::ServerFull { reason } => {
                    // Generic, operator-free message.
                    assert!(
                        reason.eq_ignore_ascii_case("Server is full, try again later."),
                        "reason was {reason:?} (must be the fixed generic string)"
                    );
                    // Defence-in-depth: explicitly forbid leaking metrics.
                    assert!(!reason.contains('%'), "reason leaks a percent sign: {reason}");
                    if let Some(u) = used {
                        assert!(
                            !reason.contains(&u.to_string()),
                            "reason leaks used_percent={u}: {reason}"
                        );
                    }
                    assert!(
                        !reason.contains(&ceiling.to_string()),
                        "reason leaks ceiling_percent={ceiling}: {reason}"
                    );
                    assert!(
                        !reason.to_lowercase().contains("memory"),
                        "reason leaks the word 'memory': {reason}"
                    );
                    assert!(
                        !reason.to_lowercase().contains("ceiling"),
                        "reason leaks the word 'ceiling': {reason}"
                    );
                }
                other => panic!("expected ServerFull, got {other:?}"),
            }
        }
    }

    #[test]
    fn active_and_waiting_counts() {
        let mut s = LobbyState::new();
        s.waiting_games.insert("g1".to_string(), pending(1, "g1", 100, false));
        s.active_games.insert(
            2,
            ActiveGame {
                id: 2,
                name: "g2".to_string(),
                p1_name: "alice".to_string(),
                p2_name: "bob".to_string(),
                started_at: Instant::now(),
                p1_reconnect_token: None,
                p2_reconnect_token: None,
            },
        );
        assert_eq!(s.waiting_count(), 1);
        assert_eq!(s.active_count(), 1);
    }
}
