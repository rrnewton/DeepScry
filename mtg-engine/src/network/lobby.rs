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

use crate::network::protocol::{DeckSubmission, LobbyGameEntry, ServerMessage};
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
    /// Hand-off channel — see [`JoinedPlayer`]. `None` after the joiner takes
    /// it (which is also the moment the entry is removed from the map; this
    /// field exists as `Option` only so the Sender can be moved out without
    /// destructuring the surrounding `PendingGame`).
    pub handoff_tx: Option<oneshot::Sender<JoinedPlayer>>,
}

/// Information about a game that is currently being played.
///
/// Carries no game state — that lives inside the spawned game task. We track
/// active games so `ListGames` can return accurate "in progress" totals and
/// so the watchdog can enforce per-game timeouts.
#[derive(Debug)]
pub struct ActiveGame {
    pub id: GameId,
    pub name: String,
    pub p1_name: String,
    pub p2_name: String,
    pub started_at: Instant,
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
}

impl LobbyState {
    /// Create an empty lobby. The first id handed out is `1` (matches the
    /// legacy server behaviour).
    pub fn new() -> Self {
        Self {
            waiting_games: HashMap::new(),
            active_games: HashMap::new(),
            next_game_id: 1,
        }
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

    /// Snapshot of `waiting_games` for the `ListGames` reply.
    pub fn list_waiting(&self) -> Vec<LobbyGameEntry> {
        let mut out: Vec<LobbyGameEntry> = self
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
        out.sort_by_key(|e| e.created_at_ms);
        out
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
mod tests {
    use super::*;

    fn pending(id: GameId, name: &str, created_ms: u64, has_pw: bool) -> PendingGame {
        let (tx, _rx) = oneshot::channel();
        PendingGame {
            id,
            name: name.to_string(),
            creator_name: format!("Creator{id}"),
            has_password: has_pw,
            password_hash: has_pw.then(|| hash_game_password("secret")),
            created_at: Instant::now(),
            created_at_ms: created_ms,
            handoff_tx: Some(tx),
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
            },
        );
        assert_eq!(s.waiting_count(), 1);
        assert_eq!(s.active_count(), 1);
    }
}
