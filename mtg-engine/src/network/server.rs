//! WebSocket game server for multiplayer MTG
//!
//! Implements a server that:
//! - Accepts client connections over WebSocket
//! - Handles authentication and deck submission
//! - Matches players (first waits for second)
//! - Runs authoritative game state with NetworkControllers
//! - Broadcasts card reveals and opponent choices

use crate::core::{CardId, PlayerId, SpellAbility};
use crate::game::state_hash::compute_network_state_hash;
use crate::game::{GameEndReason, GameLoop, GameResult, GameState};
use crate::loader::{AsyncCardDatabase, DeckEntry, DeckList, GameInitializer};
use crate::network::lobby::{
    build_server_full_message, hash_game_password, new_shared_lobby, ActiveGame, JoinedPlayer, PendingGame,
    SharedLobby, WaitingPlayerState, WaitingRoomSnapshot, DEFAULT_GAME_TIMEOUT,
};
use crate::network::memory::{check_memory_admission, current_system_memory, AdmissionVerdict};
use crate::network::protocol::{
    now_ms, BufferedFact, CardReveal, ChoiceType, ClientMessage, DeckListInfo, DeckSubmission, JoinFailReason,
    ReconnectToken, RevealReason, ServerMessage, DEFAULT_LOBBY_GAME,
};
use crate::network::{CardRevealInfo, ChoiceRequest, ChoiceResponse, NetworkController, DEFAULT_PORT};
use crate::zones::Zone;
use anyhow::{anyhow, Result};
use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use std::collections::HashSet;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use tokio::fs;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc as tokio_mpsc, oneshot, Mutex};
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
    /// Optional password for elevated/trusted bug report handling
    pub trusted_bug_report_password: String,
    /// Maximum concurrent games (0 = unlimited).
    ///
    /// **Deprecated**: kept for backwards compatibility with older config
    /// files. The lobby admission gate now uses `max_memory_percent` against
    /// host system memory rather than a fixed game count, because the right
    /// concurrency limit depends on host size and other workloads. New code
    /// should leave this at `0`.
    pub max_games: usize,
    /// Maximum host memory utilisation, as a percentage in `0..=100`.
    ///
    /// New `CreateGame`/`JoinGame` requests are denied with
    /// `ServerMessage::ServerFull` when
    /// `(MemTotal - MemAvailable) / MemTotal * 100` exceeds this value.
    /// `0` disables the gate. Existing in-flight games are NEVER killed — we
    /// only refuse new admissions, so a transient spike does not lose live
    /// matches.
    ///
    /// Default `80` leaves comfortable headroom for the kernel page cache
    /// and other tenants on the host. See `network::memory` for the exact
    /// calculation and platform support.
    pub max_memory_percent: u32,
    /// Starting life total
    pub starting_life: i32,
    /// Whether to share deck lists between players (tournament mode)
    pub deck_visibility: bool,
    /// Path to cardsfolder for loading cards
    pub cardsfolder: PathBuf,
    /// Fixed seed for RNG (None = random seed each game)
    pub seed: Option<u64>,
    /// Tag official game action logs with [GAMELOG TurnN STEP] prefix
    pub tag_gamelogs: bool,
    /// Verbosity level for game output
    pub verbosity: crate::game::VerbosityLevel,
    /// Enable network debug mode - populates debug fields in protocol messages
    pub network_debug: bool,
    /// Disable ANSI colored log output
    pub no_color_logs: bool,
    /// Loop mode: keep running and accept new games after each one completes
    pub loop_mode: bool,
    /// Directory where submitted bug reports are stored
    pub bug_reports_dir: PathBuf,
    /// Host portion of the listen socket.
    ///
    /// Default is `0.0.0.0`, which preserves the historical behaviour of
    /// `mtg server`. The unified `mtg server-web` flow overrides this to
    /// `127.0.0.1` so the embedded lobby is reachable only via the axum
    /// proxy on the public bind. Changing the default would break
    /// existing deploy scripts and the e2e network tests.
    pub bind_host: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            port: DEFAULT_PORT,
            password: String::new(),
            trusted_bug_report_password: String::new(),
            max_games: 0,
            max_memory_percent: crate::network::lobby::DEFAULT_MAX_MEMORY_PERCENT,
            starting_life: 20,
            deck_visibility: false,
            cardsfolder: PathBuf::from("cardsfolder"),
            seed: None,
            tag_gamelogs: false,
            verbosity: crate::game::VerbosityLevel::Normal,
            network_debug: false,
            no_color_logs: false,
            loop_mode: false,
            bug_reports_dir: PathBuf::from("bug_reports"),
            bind_host: "0.0.0.0".to_string(),
        }
    }
}

/// GitHub repository that bug-report issues are filed against.
///
/// Passed EXPLICITLY via `gh ... -R <repo>` on every repo-scoped command so the
/// server never relies on `GH_REPO` being present in the systemd unit env or on
/// `gh`'s cwd-based repo auto-detection (mtg-587).
const BUG_REPORT_GITHUB_REPO: &str = "rrnewton/DeepScry";

#[derive(Debug, Clone)]
struct BugReportRequest {
    description: String,
    game_logs: String,
    console_logs: String,
    trusted_password: Option<String>,
}

#[derive(Debug, Serialize)]
struct BugReportMetadata {
    timestamp_ms: u64,
    reporter_player_id: Option<u32>,
    trusted: bool,
    trusted_password_supplied: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CommandOutput {
    success: bool,
    stdout: String,
    stderr: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GitHubIssueOutcome {
    issue_url: String,
    warning: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StoredBugReport {
    report_dir: PathBuf,
    trusted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AutoFixLaunchRequest {
    issue_url: String,
    prompt: String,
}

// ═══════════════════════════════════════════════════════════════════════════
// WAITING PLAYER
// ═══════════════════════════════════════════════════════════════════════════

/// A player waiting for an opponent
struct WaitingPlayer {
    /// Player's display name (None = server should assign default with suffix)
    name: Option<String>,
    /// Submitted deck
    deck: DeckSubmission,
    /// WebSocket connection
    ws_stream: WebSocketStream<TcpStream>,
    /// Reconnect token issued for this player at game-start time.
    reconnect_token: Option<ReconnectToken>,
}

// ═══════════════════════════════════════════════════════════════════════════
// PLAYER CONNECTION
// ═══════════════════════════════════════════════════════════════════════════

/// Connection handler for a single player
/// Information about how a game ended, sent to WebSocket handlers
#[derive(Clone, Debug)]
struct GameEndInfo {
    winner: Option<PlayerId>,
    reason: GameEndReason,
    final_hash: u64,
    action_count: u64,
}

/// Info about an opponent's choice, broadcast to the other player
#[derive(Clone, Debug)]
struct OpponentChoiceInfo {
    /// Choice sequence number
    choice_seq: u32,
    /// Which player made this choice (P1=0, P2=1)
    player: PlayerId,
    /// Type of choice
    choice_type: ChoiceType,
    /// Indices of the chosen options (multiple for attackers/blockers/discard)
    choice_indices: Vec<usize>,
    /// Human-readable description
    description: String,
    /// Action count at time of choice (for sync validation)
    action_count: u64,
    /// The actual spell ability chosen (for Priority choices)
    /// Allows client to execute the ability directly without computing from hidden hand
    spell_ability: Option<SpellAbility>,
    /// For LibrarySearchByName choices: the specific CardId that was chosen
    /// Allows client's shadow game to know which card moved to hand
    library_search_result: Option<CardId>,
    /// Actual target CardIds for target choices
    /// Client uses these directly instead of mapping indices
    target_card_ids: Option<Vec<CardId>>,
}

/// Card reveal info to broadcast to a player
/// NOTE: Currently unused - reveal broadcasts are disabled to prevent ordering issues.
/// Kept for potential future use when async reveal ordering is fixed.
#[derive(Clone)]
#[allow(dead_code)]
struct RevealBroadcast {
    /// Owner of the card
    owner: PlayerId,
    /// Card ID
    card_id: CardId,
    /// Zone the card moved to
    to_zone: Zone,
}

// ═══════════════════════════════════════════════════════════════════════════
// NETWORK DEBUG HELPERS
// ═══════════════════════════════════════════════════════════════════════════

use crate::network::protocol::DebugSyncInfo;

/// Log detailed state hash mismatch information for debugging network sync issues.
/// Called when server and client state hashes differ in network debug mode.
fn log_state_hash_mismatch(
    player: &str,
    choice_seq: u32,
    action_count: u64,
    server_hash: u64,
    client_hash: u64,
    server_debug_info: &Option<DebugSyncInfo>,
    client_debug_info: &Option<DebugSyncInfo>,
) {
    log::error!("╔══════════════════════════════════════════════════════════════════╗");
    log::error!(
        "║ NETWORK SYNC MISMATCH DETECTED - {} choice_seq={:<16} ║",
        player,
        choice_seq
    );
    log::error!("╠══════════════════════════════════════════════════════════════════╣");
    log::error!(
        "║ Server hash: {:016x}  Client hash: {:016x} ║",
        server_hash,
        client_hash
    );
    log::error!(
        "║ Action count: {} (both should match)                             ║",
        action_count
    );

    // Log server debug info
    if let Some(ref srv) = server_debug_info {
        log_debug_sync_info("SERVER", srv);
    }

    // Log client debug info
    if let Some(ref cli) = client_debug_info {
        log_debug_sync_info("CLIENT", cli);
    }

    // Compare and highlight differences
    if let (Some(ref srv), Some(ref cli)) = (server_debug_info, client_debug_info) {
        log_state_differences(srv, cli);
    }

    log::error!("╚══════════════════════════════════════════════════════════════════╝");
}

/// Log a single DebugSyncInfo block with a label
fn log_debug_sync_info(label: &str, info: &DebugSyncInfo) {
    log::error!("╠══════════════════════════════════════════════════════════════════╣");
    log::error!("║ {} STATE:", label);
    log::error!(
        "║   Turn {} {:?} active={:?}",
        info.turn,
        info.phase,
        info.active_player
    );
    log::error!(
        "║   Life: {:?}  Hands: {:?}  Libs: {:?}",
        info.life_totals,
        info.hand_sizes,
        info.library_sizes
    );
    log::error!(
        "║   Battlefield: {}  Stack: {}  Graveyards: {:?}",
        info.battlefield_count,
        info.stack_size,
        info.graveyard_sizes
    );
    if !info.requesting_player_hand_ids.is_empty() {
        log::error!("║   Hand CardIds: {:?}", info.requesting_player_hand_ids);
    }
    if !info.battlefield_detail.is_empty() {
        log::error!("║   Battlefield (id,tapped,ctrl): {:?}", info.battlefield_detail);
    }
    if info.graveyard_ids.iter().any(|g| !g.is_empty()) {
        log::error!("║   Graveyard CardIds: {:?}", info.graveyard_ids);
    }
    if info.library_ids.iter().any(|g| !g.is_empty()) {
        let mut p0 = info.library_ids[0].clone();
        let mut p1 = info.library_ids[1].clone();
        p0.sort_unstable();
        p1.sort_unstable();
        log::error!("║   Library CardIds(sorted): P0={:?}", p0);
        log::error!("║   Library CardIds(sorted): P1={:?}", p1);
    }
}

/// Compare two DebugSyncInfo and log specific differences
fn log_state_differences(server: &DebugSyncInfo, client: &DebugSyncInfo) {
    log::error!("╠══════════════════════════════════════════════════════════════════╣");
    log::error!("║ DIFFERENCES:");
    if server.life_totals != client.life_totals {
        log::error!("║   - Life totals DIFFER");
    }
    if server.hand_sizes != client.hand_sizes {
        log::error!("║   - Hand sizes DIFFER");
    }
    if server.library_sizes != client.library_sizes {
        log::error!("║   - Library sizes DIFFER");
    }
    if server.battlefield_count != client.battlefield_count {
        log::error!("║   - Battlefield count DIFFERS");
    }
    if server.graveyard_sizes != client.graveyard_sizes {
        log::error!("║   - Graveyard sizes DIFFER");
    }
    if server.requesting_player_hand_ids != client.requesting_player_hand_ids {
        log::error!("║   - Hand CardIds DIFFER");
        log::error!("║      Server: {:?}", server.requesting_player_hand_ids);
        log::error!("║      Client: {:?}", client.requesting_player_hand_ids);
    }
    // mtg-mb668 class-A: pinpoint the per-card battlefield divergence (a tap-status
    // or controller mismatch on a single card) when the coarse sizes all match.
    if server.battlefield_detail != client.battlefield_detail {
        log::error!("║   - Battlefield (id,tapped,ctrl) DIFFERS");
        let s: std::collections::BTreeMap<u32, (bool, u32)> = server
            .battlefield_detail
            .iter()
            .map(|&(id, t, c)| (id, (t, c)))
            .collect();
        let c: std::collections::BTreeMap<u32, (bool, u32)> = client
            .battlefield_detail
            .iter()
            .map(|&(id, t, c)| (id, (t, c)))
            .collect();
        for (id, sv) in &s {
            match c.get(id) {
                Some(cv) if cv == sv => {}
                Some(cv) => log::error!("║      card {id}: server(tapped,ctrl)={sv:?} client={cv:?}"),
                None => log::error!("║      card {id}: on SERVER bf {sv:?}, MISSING on client"),
            }
        }
        for id in c.keys() {
            if !s.contains_key(id) {
                log::error!("║      card {id}: on CLIENT bf, MISSING on server");
            }
        }
    }
    if server.graveyard_ids != client.graveyard_ids {
        log::error!("║   - Graveyard CardIds DIFFER");
        log::error!("║      Server: {:?}", server.graveyard_ids);
        log::error!("║      Client: {:?}", client.graveyard_ids);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SINGLE-CHANNEL ARCHITECTURE (mtg-228)
// ═══════════════════════════════════════════════════════════════════════════
//
// Each player handler has exactly ONE channel from the game coordinator and
// ONE channel back. This eliminates all `tokio::select!` over multiple channels
// and ensures completely deterministic message ordering.
//
// Design principles:
// 1. Linear control transfer: At any moment, exactly ONE entity has "control"
// 2. Sequential handler loop: Waits for game_rx, then handles message
// 3. Opponent choices flow through game coordinator: Not directly between handlers
// 4. WebSocket I/O is naturally sequential: One message at a time
//
// Architecture:
//
// ┌─────────────────┐     sync      ┌─────────────────┐     async     ┌─────────────────┐
// │ NetworkController├──────────────►│   Coordinator   ├──────────────►│ PlayerHandler   │
// │   (P1)          │◄──────────────┤   Task          │◄──────────────┤   (P1)          │
// └─────────────────┘               │                 │               └─────────────────┘
//                                   │                 │
// ┌─────────────────┐     sync      │                 │     async     ┌─────────────────┐
// │ NetworkController├──────────────►│                 ├──────────────►│ PlayerHandler   │
// │   (P2)          │◄──────────────┤                 │◄──────────────┤   (P2)          │
// └─────────────────┘               └─────────────────┘               └─────────────────┘
//
// Handler loop:
//   loop {
//       // Select between game messages and websocket I/O
//       select! {
//           msg = game_rx.recv() => handle_game_message(msg),
//           ws_msg = ws_rx.next() => handle_ws_message(ws_msg),
//       }
//   }
//
// When ChoiceRequest arrives, handler:
//   1. Sends ChoiceRequest to client
//   2. Waits for SubmitChoice (or queues pending_choice if it arrived early)
//   3. Validates action_count/state_hash
//   4. Sends ChoiceResponse to coordinator
//   5. Coordinator sends ChoiceAccepted + OpponentMadeChoice

/// Messages from game coordinator to player handler.
///
/// All game state messages flow through this single channel, ensuring
/// total ordering and eliminating race conditions.
#[derive(Debug)]
enum GameToHandler {
    /// Server needs this player to make a choice.
    /// Handler should forward to client, wait for response, send back via HandlerToGame.
    /// Boxed: `ChoiceRequest` is the largest variant (carries reveals + optional
    /// `DebugSyncInfo`); boxing keeps `GameToHandler` small (clippy large_enum_variant).
    ChoiceRequest(Box<ChoiceRequest>),
    /// Opponent made a choice - handler should forward to client.
    /// No response expected.
    OpponentMadeChoice(OpponentChoiceInfo),
    /// Acknowledge that player's choice was applied to game state.
    /// Handler should forward to client.
    ChoiceAccepted {
        choice_seq: u32,
        action_count: u64,
        timestamp_ms: u64,
        /// For LibrarySearchByName choices: the specific CardId that was chosen
        library_search_result: Option<CardId>,
    },
    /// Server wants to broadcast a library reorder to this client (e.g., after
    /// the engine ran a hidden-info-dependent scry/surveil heuristic on the
    /// authoritative game state). Handler forwards as
    /// `ServerMessage::LibraryReordered`. See mtg-420.
    LibraryReordered {
        player: PlayerId,
        new_order: Vec<CardId>,
        /// Game `action_count` of the reorder's own undo action (mtg-o99ow).
        action_count: u64,
    },
    /// Game has ended normally.
    /// Handler should forward to client and exit.
    GameEnded(GameEndInfo),
    /// Fatal error occurred (desync, disconnect, etc).
    /// Handler should forward to client and exit.
    FatalError(String),
}

/// Messages from player handler to game coordinator.
#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub enum HandlerToGame {
    /// Player submitted their choice response.
    ChoiceResponse {
        response: ChoiceResponse,
        /// Client-reported action count for validation
        client_action_count: u64,
        /// Client state hash (for network_debug validation)
        client_state_hash: Option<u64>,
        /// Debug info from client
        client_debug_info: Option<crate::network::protocol::DebugSyncInfo>,
    },
    /// Client disconnected gracefully or due to error.
    ClientDisconnected,
    /// Client sent invalid data.
    ClientError(String),
}

/// A choice that arrived before the corresponding ChoiceRequest.
/// In synchronized GameLoop mode, the client may compute and submit
/// their choice before the server's request arrives.
#[derive(Debug)]
struct PendingChoice {
    choice_seq: u32,
    choice_indices: Vec<usize>,
    action_count: u64,
    client_state_hash: Option<u64>,
    client_debug_info: Option<crate::network::protocol::DebugSyncInfo>,
    /// The actual spell ability chosen (for Priority choices)
    spell_ability: Option<SpellAbility>,
    /// Actual target CardIds for target choices
    target_card_ids: Option<Vec<CardId>>,
}

/// Player connection with single-channel architecture.
///
/// Each handler has exactly:
/// - One rx from game coordinator (game_rx)
/// - One tx to game coordinator (game_tx)
/// - WebSocket I/O (ws_tx, handled separately)
struct PlayerConnection {
    /// Player ID in the game
    player_id: PlayerId,
    /// WebSocket sender
    ws_tx: futures_util::stream::SplitSink<WebSocketStream<TcpStream>, Message>,
    /// SINGLE channel to receive all messages from game coordinator
    game_rx: tokio_mpsc::Receiver<GameToHandler>,
    /// SINGLE channel to send all messages to game coordinator
    game_tx: tokio_mpsc::Sender<HandlerToGame>,
    /// Current pending choice from client (arrived before ChoiceRequest)
    pending_choice: Option<PendingChoice>,
}

impl PlayerConnection {
    /// Send a server message to this player
    async fn send(&mut self, msg: &ServerMessage) -> Result<()> {
        let json = serde_json::to_string(msg)?;

        // Log at DEBUG level with truncation for long messages
        if log::log_enabled!(log::Level::Debug) {
            let truncated = if json.len() > 500 {
                format!("{}... ({} bytes total)", &json[..500], json.len())
            } else {
                json.clone()
            };
            log::debug!("[SERVER->P{}] {}", self.player_id, truncated);
        }

        self.ws_tx.send(Message::Text(json.into())).await?;
        Ok(())
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// GAME SERVER
// ═══════════════════════════════════════════════════════════════════════════

/// MTG game server (single-game: runs one game then exits)
pub struct GameServer {
    /// Server configuration
    config: ServerConfig,
    /// Card database (shared across games via `Arc`)
    card_db: Option<Arc<AsyncCardDatabase>>,
    /// Multi-game lobby state shared across per-connection tasks.
    ///
    /// Replaces the old `waiting_player: Option<WaitingPlayer>` (single-slot)
    /// design. See `network::lobby` for layout and the
    /// `server-lobby-multiplexing` task for the rationale.
    lobby: SharedLobby,
}

impl GameServer {
    /// Create a new game server
    pub fn new(config: ServerConfig) -> Self {
        Self {
            config,
            card_db: None,
            lobby: new_shared_lobby(),
        }
    }

    /// Clone the shared lobby handle so observers (e.g. the `/health`
    /// HTTP endpoint in `web_server`) can read counts without owning the
    /// `GameServer` itself. The returned `Arc` shares state with the
    /// running server.
    pub fn lobby_handle(&self) -> SharedLobby {
        std::sync::Arc::clone(&self.lobby)
    }

    /// Run the server (long-lived).
    ///
    /// Each TCP accept spawns its own connection task immediately, so two
    /// players completing auth do NOT block other players from doing so. The
    /// shared [`SharedLobby`] is the rendezvous point; see
    /// `network::lobby` and the `server-lobby-multiplexing` task notes for
    /// the full design.
    ///
    /// **Behavioural change vs. the legacy single-game server:** `run()` no
    /// longer exits after one game completes. The server accepts connections
    /// until it is killed. The previous `loop_mode` flag therefore only
    /// affects logging now (it is still respected so existing config files
    /// keep working).
    ///
    /// # Errors
    ///
    /// Returns an error if card database loading or TCP binding fails.
    /// Per-connection errors are logged and dropped.
    ///
    /// # Panics
    ///
    /// Panics if the card database `Arc` is somehow `None` after the load
    /// step above succeeded — this would indicate a programmer error in
    /// `Server::run` (the load above unconditionally stores `Some(...)`),
    /// not a recoverable failure.
    pub async fn run(&mut self) -> Result<()> {
        // Load card database (shared across all games via Arc)
        log::info!("Loading card database from {:?}...", self.config.cardsfolder);
        let card_db = AsyncCardDatabase::new(self.config.cardsfolder.clone());
        card_db.eager_load().await?;
        log::info!("Card database loaded");
        self.card_db = Some(Arc::new(card_db));

        // Start listening
        let addr = format!("{}:{}", self.config.bind_host, self.config.port);
        let listener = TcpListener::bind(&addr).await?;
        // Log the ACTUAL bound address (listener.local_addr()), not the requested
        // `addr` — with `--port 0` the OS assigns an ephemeral port, so the
        // requested string would read ":0". Reporting the real port lets callers
        // (e.g. the network-equiv e2e harness, mtg-ibj22) bind :0 atomically and
        // then discover the chosen port from this line — eliminating the
        // RANDOM-port-collision false-positive class with no TOCTOU.
        let bound = listener
            .local_addr()
            .map(|a| a.to_string())
            .unwrap_or_else(|_| addr.clone());
        log::info!("MTG Server listening on {}", bound);
        log::info!("Password required: {}", !self.config.password.is_empty());
        log::info!(
            "Lobby memory ceiling: {}% (0 = unlimited)",
            self.config.max_memory_percent
        );

        // Per-connection accept loop. Accepts cannot block waiting for game
        // completion any more — that work happens entirely inside the spawned
        // task.
        loop {
            match listener.accept().await {
                Ok((stream, peer)) => {
                    log::info!("New connection from {}", peer);
                    let lobby = Arc::clone(&self.lobby);
                    let card_db = Arc::clone(self.card_db.as_ref().expect("card db loaded above"));
                    let config = self.config.clone();
                    tokio::spawn(async move {
                        if let Err(e) = handle_lobby_connection(stream, lobby, card_db, config).await {
                            log::warn!("Connection from {peer} ended with error: {e}");
                        }
                    });
                }
                Err(e) => {
                    log::error!("Accept error: {}", e);
                }
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// LOBBY CONNECTION HANDLING
// ═══════════════════════════════════════════════════════════════════════════

/// How long a `CreateGame` waits for a joiner before giving up.
///
/// 30 minutes is generous for a casual game-finding flow but short enough that
/// orphan entries do not pin `Arc<AsyncCardDatabase>` references forever.
const WAIT_FOR_JOINER: std::time::Duration = std::time::Duration::from_secs(30 * 60);

/// Top-level dispatcher for one freshly-accepted WebSocket connection.
///
/// Reads the first message and routes it. Lobby-only messages (`ListGames`,
/// `Register`) can repeat; the connection stays open until the client sends a
/// "commitment" message (`CreateGame`/`JoinGame`/`Authenticate`/`BugReport`/
/// `Reconnect`). On every exit path — including errors and panics — any
/// registered name held by this connection is released from the lobby so the
/// name slot becomes available for a new client.
async fn handle_lobby_connection(
    stream: TcpStream,
    lobby: SharedLobby,
    card_db: Arc<AsyncCardDatabase>,
    config: ServerConfig,
) -> Result<()> {
    let ws_stream = accept_async(stream).await?;

    // Assign a stable per-connection ID so we can clean up registrations on
    // disconnect without storing a mutable reference into the lobby map.
    let connection_id = {
        let mut l = lobby.lock().await;
        l.next_connection_id()
    };
    // The name registered by this connection (if any), kept here for cleanup.
    let mut registered_name: Option<String> = None;

    let result = run_lobby_dispatch(
        ws_stream,
        &lobby,
        &card_db,
        &config,
        connection_id,
        &mut registered_name,
    )
    .await;

    // Always release the name on any exit (normal, error, or panic guard).
    if let Some(ref name) = registered_name {
        let mut l = lobby.lock().await;
        l.release_name(name, connection_id);
    }

    result
}

/// Inner dispatch loop extracted so the cleanup in `handle_lobby_connection`
/// can run unconditionally (the `?` operator would skip cleanup if left in the
/// outer function body).
///
/// Takes `ws_stream` by value. Functions that need to own the stream (such as
/// `run_create_flow`) receive it directly; once they return this function
/// terminates.
#[allow(clippy::too_many_arguments)]
async fn run_lobby_dispatch(
    mut ws_stream: WebSocketStream<TcpStream>,
    lobby: &SharedLobby,
    card_db: &Arc<AsyncCardDatabase>,
    config: &ServerConfig,
    connection_id: u64,
    registered_name: &mut Option<String>,
) -> Result<()> {
    loop {
        let msg = read_one_lobby_message(&mut ws_stream).await?;
        match msg {
            // ── New: unique-name registration ────────────────────────────────
            ClientMessage::Register { password, player_name } => {
                if !check_server_password(config, &password) {
                    send_message(
                        &mut ws_stream,
                        &ServerMessage::RegisterResult {
                            success: false,
                            player_name: player_name.clone(),
                            error: Some("Invalid server password".to_string()),
                        },
                    )
                    .await?;
                    return Ok(());
                }
                let outcome = {
                    let mut l = lobby.lock().await;
                    l.try_register_name(&player_name, connection_id)
                };
                match outcome {
                    Ok(()) => {
                        *registered_name = Some(player_name.clone());
                        send_message(
                            &mut ws_stream,
                            &ServerMessage::RegisterResult {
                                success: true,
                                player_name,
                                error: None,
                            },
                        )
                        .await?;
                        // Loop: client now browses the lobby.
                    }
                    Err(reason) => {
                        send_message(
                            &mut ws_stream,
                            &ServerMessage::RegisterResult {
                                success: false,
                                player_name,
                                error: Some(reason),
                            },
                        )
                        .await?;
                        // Non-fatal: client can retry with a different name.
                    }
                }
            }

            // ── Reconnect (dropped player re-joining an in-progress game) ────
            ClientMessage::Reconnect { token, game_name } => {
                let outcome = {
                    let l = lobby.lock().await;
                    l.validate_reconnect_token(&game_name, &token)
                };
                match outcome {
                    Some((_game_id, player_index)) => {
                        let your_player_id = Some(PlayerId::new(player_index as u32));
                        send_message(
                            &mut ws_stream,
                            &ServerMessage::ReconnectResult {
                                success: true,
                                game_name: game_name.clone(),
                                your_player_id,
                                error: None,
                            },
                        )
                        .await?;
                        // Phase 1 stub: token is valid but in-game task
                        // reattachment is deferred to Phase 3. The connection
                        // can be kept alive here for a future resume handshake.
                        log::info!(
                            "Reconnect accepted for game '{}' player {} (Phase 3 resume pending)",
                            game_name,
                            player_index
                        );
                        return Ok(());
                    }
                    None => {
                        send_message(
                            &mut ws_stream,
                            &ServerMessage::ReconnectResult {
                                success: false,
                                game_name,
                                your_player_id: None,
                                error: Some("Invalid or expired reconnect token".to_string()),
                            },
                        )
                        .await?;
                        return Ok(());
                    }
                }
            }

            ClientMessage::ListGames { password, query } => {
                if !check_server_password(config, &password) {
                    // We send AuthResult { success: false, .. } so legacy
                    // clients can decode it; ListGames is intentionally a
                    // pre-auth probe, but the server password still gates it.
                    send_message(
                        &mut ws_stream,
                        &ServerMessage::AuthResult {
                            success: false,
                            error: Some("Invalid server password".to_string()),
                            your_player_id: None,
                            your_name: None,
                        },
                    )
                    .await?;
                    return Ok(());
                }
                let (games, total_count) = {
                    let l = lobby.lock().await;
                    l.list_waiting_paged(query.as_ref())
                };
                let mem = current_system_memory();
                send_message(
                    &mut ws_stream,
                    &ServerMessage::GameList {
                        games,
                        total_count,
                        system_memory_used_percent: mem.map(|m| m.used_percent()),
                        max_memory_percent: config.max_memory_percent,
                    },
                )
                .await?;
                // Loop: connection stays open for a follow-up Create/Join.
            }

            ClientMessage::CreateGame {
                password,
                game_name,
                game_password,
                player_name,
                deck,
                waiting_room,
            } => {
                // Prefer the registered name if the client didn't supply one.
                let resolved_name = player_name.or_else(|| registered_name.clone());
                return run_create_flow(
                    ws_stream,
                    SharedLobby::clone(lobby),
                    Arc::clone(card_db),
                    config.clone(),
                    password,
                    game_name,
                    game_password,
                    resolved_name,
                    deck,
                    connection_id,
                    waiting_room,
                )
                .await;
            }

            ClientMessage::JoinGame {
                password,
                game_name,
                game_password,
                player_name,
                deck,
                waiting_room,
            } => {
                let resolved_name = player_name.or_else(|| registered_name.clone());
                return run_join_flow(
                    ws_stream,
                    SharedLobby::clone(lobby),
                    config.clone(),
                    password,
                    game_name,
                    game_password,
                    resolved_name,
                    deck,
                    waiting_room,
                )
                .await;
            }

            ClientMessage::Authenticate {
                password,
                player_name,
                deck,
            } => {
                // Legacy single-game flow: the first authenticator creates the
                // well-known DEFAULT_LOBBY_GAME, the second joins it. Once two
                // legacy clients have paired up we treat further Authenticates
                // as fresh CreateGames (so a 3rd legacy client doesn't error
                // immediately — they'll wait for a 4th).
                let exists = {
                    let l = lobby.lock().await;
                    l.waiting_games.contains_key(DEFAULT_LOBBY_GAME)
                };
                let resolved_name = player_name.or_else(|| registered_name.clone());
                if exists {
                    return run_join_flow(
                        ws_stream,
                        SharedLobby::clone(lobby),
                        config.clone(),
                        password,
                        DEFAULT_LOBBY_GAME.to_string(),
                        None,
                        resolved_name,
                        deck,
                        false, // legacy auto-match: start immediately on join
                    )
                    .await;
                } else {
                    return run_create_flow(
                        ws_stream,
                        SharedLobby::clone(lobby),
                        Arc::clone(card_db),
                        config.clone(),
                        password,
                        Some(DEFAULT_LOBBY_GAME.to_string()),
                        None,
                        resolved_name,
                        deck,
                        connection_id,
                        false, // legacy auto-match: start immediately on join
                    )
                    .await;
                }
            }

            ClientMessage::BugReport {
                description,
                game_logs,
                console_logs,
                trusted_password,
            } => {
                // Two-phase bug report (mtg-5ejgo): flush the disk-write
                // confirmation IMMEDIATELY, then attempt the (timeout-bounded)
                // GitHub filing and send its result. The two sends are inlined
                // rather than wrapped in a single helper because phase 1 must
                // reach the client BEFORE the phase-2 GitHub await begins — that
                // ordering is the whole point of the fix.
                let report = BugReportRequest {
                    description,
                    game_logs,
                    console_logs,
                    trusted_password,
                };
                let stored = store_bug_report(config, &report, None).await;
                send_message(&mut ws_stream, &bug_report_stored_message(&stored, None)).await?;
                if let Ok(stored_report) = &stored {
                    let issue_msg = file_bug_report_issue(&report, stored_report, None).await;
                    send_message(&mut ws_stream, &issue_msg).await?;
                }
                return Ok(());
            }

            // SetDeck / SetReady are only valid inside a waiting room
            // (after CreateGame/JoinGame). At the bare lobby level they are
            // unexpected — tell the client and close.
            ClientMessage::SetDeck { .. } | ClientMessage::SetReady { .. } => {
                send_error(
                    &mut ws_stream,
                    "SetDeck/SetReady are only valid after joining a game waiting room",
                    false,
                )
                .await?;
                // Non-fatal: client can correct the flow.
            }

            // Anything else at the lobby level is an error — Submit/Disconnect/
            // Ping belong inside an active game. Spelled out (rather than `_`)
            // so that adding a new `ClientMessage` variant forces a compile
            // error here, ensuring lobby intent is reviewed.
            other @ (ClientMessage::SubmitChoice { .. } | ClientMessage::Disconnect | ClientMessage::Ping { .. }) => {
                send_error(
                    &mut ws_stream,
                    &format!("Unexpected pre-game message: {:?}", std::mem::discriminant(&other)),
                    true,
                )
                .await?;
                return Ok(());
            }
        }
    }
}

/// Read a single ClientMessage off the WebSocket. Used by the lobby loop and
/// `handle_lobby_connection`.
async fn read_one_lobby_message(ws_stream: &mut WebSocketStream<TcpStream>) -> Result<ClientMessage> {
    use futures_util::StreamExt;
    loop {
        match ws_stream.next().await {
            Some(Ok(Message::Text(text))) => {
                if log::log_enabled!(log::Level::Debug) {
                    let truncated = if text.len() > 500 {
                        format!("{}... ({} bytes total)", &text[..500], text.len())
                    } else {
                        text.to_string()
                    };
                    log::debug!("[CLIENT->SERVER lobby] {}", truncated);
                }
                return Ok(serde_json::from_str::<ClientMessage>(&text)?);
            }
            // Skip binary, ping, pong frames silently — tungstenite handles
            // pong frames automatically; we just loop and read again.
            Some(Ok(_)) => continue,
            Some(Err(e)) => return Err(e.into()),
            None => return Err(anyhow!("Connection closed before message")),
        }
    }
}

/// Server password gate, shared by all lobby entry points.
fn check_server_password(config: &ServerConfig, supplied: &str) -> bool {
    config.password.is_empty() || supplied == config.password
}

/// Helper: respond to a creator/joiner that the host is at its memory ceiling
/// and close the connection. Returns once the message has been sent.
async fn refuse_with_server_full(
    ws_stream: &mut WebSocketStream<TcpStream>,
    used_percent: Option<u32>,
    ceiling_percent: u32,
) -> Result<()> {
    let msg = build_server_full_message(used_percent, ceiling_percent);
    send_message(ws_stream, &msg).await?;
    Ok(())
}

/// Run the "creator" half of a lobby match.
///
/// 1. Validate server password / memory ceiling / deck.
/// 2. Allocate game id, register a `PendingGame` carrying a oneshot
///    `Sender<JoinedPlayer>`.
/// 3. Notify the client (`GameCreated` + `WaitingForOpponent`).
/// 4. Await the joiner via the oneshot (with `WAIT_FOR_JOINER` timeout).
/// 5. Move from `waiting_games` → `active_games` and run the game with a
///    `DEFAULT_GAME_TIMEOUT` cap.
/// 6. On any exit path, clear our entry from the lobby state.
///
/// `connection_id` is the per-connection monotonic ID assigned in
/// `handle_lobby_connection`. It is stored with the pending game so the
/// watchdog / eviction path can verify ownership.
#[allow(clippy::too_many_arguments)]
async fn run_create_flow(
    mut ws_stream: WebSocketStream<TcpStream>,
    lobby: SharedLobby,
    card_db: Arc<AsyncCardDatabase>,
    config: ServerConfig,
    server_password: String,
    requested_game_name: Option<String>,
    game_password: Option<String>,
    player_name: Option<String>,
    deck: DeckSubmission,
    _connection_id: u64,
    rendezvous: bool,
) -> Result<()> {
    if !check_server_password(&config, &server_password) {
        send_message(
            &mut ws_stream,
            &ServerMessage::AuthResult {
                success: false,
                error: Some("Invalid server password".to_string()),
                your_player_id: None,
                your_name: None,
            },
        )
        .await?;
        return Ok(());
    }

    // Memory gate (best-effort on non-Linux — see network::memory).
    if let AdmissionVerdict::Reject {
        memory,
        ceiling_percent,
    } = check_memory_admission(config.max_memory_percent)
    {
        return refuse_with_server_full(&mut ws_stream, Some(memory.used_percent()), ceiling_percent).await;
    }

    if deck.main_deck_size() < 40 {
        send_message(
            &mut ws_stream,
            &ServerMessage::JoinFailed {
                game_name: requested_game_name.clone().unwrap_or_default(),
                reason: JoinFailReason::InvalidDeck {
                    detail: format!("Deck too small: {} cards (minimum 40)", deck.main_deck_size()),
                },
            },
        )
        .await?;
        return Ok(());
    }

    let creator_name = player_name.unwrap_or_else(|| "Player1".to_string());
    let (handoff_tx, mut handoff_rx) = tokio::sync::oneshot::channel::<JoinedPlayer>();

    // Watch channel for waiting-room state updates (SetDeck/SetReady from either
    // player). The creator's task awaits updates on the receiver; the joiner's
    // task (and SetDeck/SetReady handlers) send on the sender stored in
    // PendingGame.
    let initial_snapshot = WaitingRoomSnapshot {
        creator_name: creator_name.clone(),
        creator_state: WaitingPlayerState {
            deck: Some(deck.clone()),
            ready: false,
        },
        joiner_name: None,
        joiner_state: None,
        start_game: false,
    };
    let (update_tx, mut update_rx) = tokio::sync::watch::channel(initial_snapshot);

    // Allocate id + name and register the pending entry. Unique-name check is
    // done under the same lock to avoid a TOCTOU race against a concurrent
    // CreateGame for the same name.
    let (game_id, game_name) = {
        let mut l = lobby.lock().await;
        let id = l.next_game_id();
        let name = requested_game_name.unwrap_or_else(|| l.default_game_name());
        let key = name.to_lowercase();
        if l.waiting_games.contains_key(&key) {
            drop(l);
            send_message(
                &mut ws_stream,
                &ServerMessage::JoinFailed {
                    game_name: name.clone(),
                    reason: JoinFailReason::InvalidDeck {
                        detail: format!("Game name '{name}' is already waiting"),
                    },
                },
            )
            .await?;
            return Ok(());
        }
        l.waiting_games.insert(
            key.clone(),
            PendingGame {
                id,
                name: name.clone(),
                creator_name: creator_name.clone(),
                has_password: game_password.is_some(),
                password_hash: game_password.as_deref().map(hash_game_password),
                created_at: std::time::Instant::now(),
                created_at_ms: now_ms(),
                creator_state: WaitingPlayerState {
                    deck: Some(deck.clone()),
                    ready: false,
                },
                joiner_state: None,
                joiner_name: None,
                creator_update_tx: Some(update_tx),
                handoff_tx: Some(handoff_tx),
                rendezvous,
            },
        );
        (id, name)
    };

    log::info!(
        "Game {} ({}): created by {} (password={})",
        game_id,
        game_name,
        creator_name,
        game_password.is_some()
    );

    send_message(
        &mut ws_stream,
        &ServerMessage::GameCreated {
            game_name: game_name.clone(),
            your_player_id: PlayerId::new(0),
            your_name: Some(creator_name.clone()),
        },
    )
    .await?;
    send_message(&mut ws_stream, &ServerMessage::WaitingForOpponent).await?;

    // Send the initial waiting-room state so the creator's UI can render it.
    let initial_update = update_rx.borrow().to_server_message();
    send_message(&mut ws_stream, &initial_update).await?;

    // Wait for joiner with a long timeout, while simultaneously:
    // (a) forwarding WaitingRoomUpdate notifications from the watch channel, and
    // (b) evicting the game from waiting_games if the creator's WS drops or
    //     if the WAIT_FOR_JOINER deadline expires.
    //
    // We must always remove our PendingGame from the map on every exit path,
    // otherwise it leaks.
    let key = game_name.to_lowercase();

    // Drive the waiting loop: poll the WS for SetDeck/SetReady/Ping from the
    // creator, and forward WaitingRoomUpdate notifications from the watch channel.
    let joiner = loop {
        tokio::select! {
            // Joiner arrived (or sender dropped / timeout).
            //
            // The `if !rendezvous` guard DISABLES this arm in rendezvous mode:
            // there the joiner never uses the handoff channel (it runs its own
            // waiting-room loop), so the handoff Sender lives inside the
            // PendingGame and is dropped when the slot is freed on both-ready.
            // That drop would otherwise resolve `handoff_rx` with `Err` and be
            // mis-read as "joiner died". With the arm disabled, the rendezvous
            // exit is driven solely by `update_rx` (start_game) / the WS read.
            joiner_result = tokio::time::timeout(WAIT_FOR_JOINER, &mut handoff_rx), if !rendezvous => {
                match joiner_result {
                    Ok(Ok(j)) => break j,
                    Ok(Err(_)) => {
                        // Sender was dropped without sending — joiner's task likely died.
                        let mut l = lobby.lock().await;
                        l.waiting_games.remove(&key);
                        drop(l);
                        let _ = send_error(&mut ws_stream, "Internal error: joiner dropped before pairing", true).await;
                        return Ok(());
                    }
                    Err(_) => {
                        // Wait timeout — evict and tell the client.
                        let mut l = lobby.lock().await;
                        l.waiting_games.remove(&key);
                        drop(l);
                        let _ = send_error(
                            &mut ws_stream,
                            &format!("No opponent joined within {} minutes", WAIT_FOR_JOINER.as_secs() / 60),
                            true,
                        )
                        .await;
                        return Ok(());
                    }
                }
            }

            // Waiting-room state changed (joiner updated their deck/ready).
            _ = update_rx.changed() => {
                // In rendezvous mode, the joiner's SetReady that crossed the
                // both-ready threshold sets `start_game` on the broadcast
                // snapshot. React by sending the "go" signal to the creator and
                // exiting (the joiner's task does the same on its side).
                let start_game = rendezvous && update_rx.borrow().start_game;
                let snapshot = update_rx.borrow().to_server_message();
                if send_message(&mut ws_stream, &snapshot).await.is_err() {
                    // Creator's WS dropped — evict the waiting game.
                    let mut l = lobby.lock().await;
                    l.waiting_games.remove(&key);
                    return Ok(());
                }
                if start_game {
                    log::info!(
                        "Game {} ({}): both ready (rendezvous) — signalling creator to proceed",
                        game_id, game_name
                    );
                    let _ = send_message(
                        &mut ws_stream,
                        &ServerMessage::WaitingRoomReady {
                            game_name: game_name.clone(),
                            is_creator: true,
                        },
                    )
                    .await;
                    // The joiner's task frees the slot from waiting_games; be
                    // defensive and remove here too (idempotent).
                    let mut l = lobby.lock().await;
                    l.waiting_games.remove(&key);
                    return Ok(());
                }
            }

            // Creator sends us a message (SetDeck, SetReady, Ping, etc.).
            msg = read_one_lobby_message(&mut ws_stream) => {
                match msg {
                    Err(_) => {
                        // Creator disconnected — evict the waiting game immediately.
                        let mut l = lobby.lock().await;
                        l.waiting_games.remove(&key);
                        log::info!(
                            "Game {} ({}): creator disconnected, evicting from waiting list",
                            game_id, game_name
                        );
                        return Ok(());
                    }
                    Ok(ClientMessage::SetDeck { deck: new_deck }) => {
                        if new_deck.main_deck_size() < 40 {
                            let _ = send_error(
                                &mut ws_stream,
                                &format!("Deck too small: {} cards (minimum 40)", new_deck.main_deck_size()),
                                false,
                            )
                            .await;
                        } else {
                            let snapshot = {
                                let mut l = lobby.lock().await;
                                if let Some(pg) = l.waiting_games.get_mut(&key) {
                                    pg.creator_state.deck = Some(new_deck);
                                    pg.creator_state.ready = false; // reset on deck change
                                    let snap = WaitingRoomSnapshot {
                                        creator_name: pg.creator_name.clone(),
                                        creator_state: pg.creator_state.clone(),
                                        joiner_name: pg.joiner_name.clone(),
                                        joiner_state: pg.joiner_state.clone(),
                                        start_game: false,
                                    };
                                    if let Some(tx) = &pg.creator_update_tx {
                                        let _ = tx.send(snap.clone());
                                    }
                                    Some(snap)
                                } else {
                                    None
                                }
                            };
                            if let Some(snap) = snapshot {
                                let _ = send_message(&mut ws_stream, &snap.to_server_message()).await;
                            }
                        }
                    }
                    Ok(ClientMessage::SetReady { ready }) => {
                        let (snapshot, both_ready) = {
                            let mut l = lobby.lock().await;
                            if let Some(pg) = l.waiting_games.get_mut(&key) {
                                if ready && pg.creator_state.deck.is_none() {
                                    (None, false)
                                } else {
                                    pg.creator_state.ready = ready;
                                    let joiner_ready = pg.joiner_state.as_ref().map(|s| s.ready).unwrap_or(false);
                                    let creator_ready = pg.creator_state.ready;
                                    let both = creator_ready && joiner_ready;
                                    let snap = WaitingRoomSnapshot {
                                        creator_name: pg.creator_name.clone(),
                                        creator_state: pg.creator_state.clone(),
                                        joiner_name: pg.joiner_name.clone(),
                                        joiner_state: pg.joiner_state.clone(),
                                        // Notify the joiner's task to proceed too.
                                        start_game: rendezvous && both,
                                    };
                                    if let Some(tx) = &pg.creator_update_tx {
                                        let _ = tx.send(snap.clone());
                                    }
                                    (Some(snap), both)
                                }
                            } else {
                                (None, false)
                            }
                        };
                        if ready && snapshot.is_none() {
                            let _ = send_error(&mut ws_stream, "Cannot ready without a deck", false).await;
                        } else if let Some(snap) = snapshot {
                            let _ = send_message(&mut ws_stream, &snap.to_server_message()).await;
                            if both_ready {
                                if rendezvous {
                                    // Variant-1 rendezvous: the creator's ready
                                    // crossed the threshold. Send the "go" signal
                                    // here (the joiner's task reacts to the
                                    // start_game snapshot it just broadcast) and
                                    // exit; the game runs on the GAME PAGE's own
                                    // socket, not this lobby socket.
                                    log::info!(
                                        "Game {} ({}): both ready (rendezvous) — signalling creator to proceed",
                                        game_id, game_name
                                    );
                                    let _ = send_message(
                                        &mut ws_stream,
                                        &ServerMessage::WaitingRoomReady {
                                            game_name: game_name.clone(),
                                            is_creator: true,
                                        },
                                    )
                                    .await;
                                    let mut l = lobby.lock().await;
                                    l.waiting_games.remove(&key);
                                    return Ok(());
                                }
                                // Legacy mode: the joiner's SetReady already
                                // triggered the handoff via the watch channel;
                                // we'll receive the joiner via handoff_rx in the
                                // next loop iteration.
                                log::info!(
                                    "Game {} ({}): both players ready, awaiting game start handoff",
                                    game_id, game_name
                                );
                            }
                        }
                    }
                    Ok(ClientMessage::Ping { timestamp_ms }) => {
                        let _ = send_message(&mut ws_stream, &ServerMessage::Pong { timestamp_ms }).await;
                    }
                    Ok(ClientMessage::Disconnect) => {
                        let mut l = lobby.lock().await;
                        l.waiting_games.remove(&key);
                        return Ok(());
                    }
                    Ok(_) => {
                        // Other messages (SubmitChoice, etc.) are invalid here.
                        let _ = send_error(&mut ws_stream, "Unexpected message in waiting room", false).await;
                    }
                }
            }
        }
    };

    // Issue reconnect tokens for both players.
    let p1_token = ReconnectToken::generate();
    let p2_token = ReconnectToken::generate();

    // Joiner's task already removed us from waiting_games; promote to active.
    {
        let mut l = lobby.lock().await;
        l.active_games.insert(
            game_id,
            ActiveGame {
                id: game_id,
                name: game_name.clone(),
                p1_name: creator_name.clone(),
                p2_name: joiner.name.clone(),
                started_at: std::time::Instant::now(),
                p1_reconnect_token: Some(p1_token.clone()),
                p2_reconnect_token: Some(p2_token.clone()),
            },
        );
    }

    log::info!(
        "Game {} ({}): starting {} vs {}",
        game_id,
        game_name,
        creator_name,
        joiner.name
    );

    let p1 = WaitingPlayer {
        name: Some(creator_name.clone()),
        deck,
        ws_stream,
        reconnect_token: Some(p1_token),
    };
    let p2 = WaitingPlayer {
        name: Some(joiner.name.clone()),
        deck: joiner.deck,
        ws_stream: joiner.ws_stream,
        reconnect_token: Some(p2_token),
    };

    // Per-game wall-clock cap so a stuck/desynced game never holds memory
    // forever. Errors and timeouts are logged but don't propagate further —
    // the connection task is the right place to absorb them.
    let game_fut = run_game(game_id, p1, p2, card_db, config);
    match tokio::time::timeout(DEFAULT_GAME_TIMEOUT, game_fut).await {
        Ok(Ok(())) => log::info!("Game {} ({}): completed", game_id, game_name),
        Ok(Err(e)) => log::error!("Game {} ({}): error: {}", game_id, game_name, e),
        Err(_) => log::warn!(
            "Game {} ({}): timed out after {:?}",
            game_id,
            game_name,
            DEFAULT_GAME_TIMEOUT
        ),
    }

    // Always remove from active_games, even on error/timeout.
    {
        let mut l = lobby.lock().await;
        l.active_games.remove(&game_id);
    }

    Ok(())
}

/// Run the "joiner" half of a lobby match.
///
/// 1. Validate server password / memory ceiling / deck.
/// 2. Lock lobby; look up the game by name; verify the per-game password.
/// 3. On match: remove the entry, send `AuthResult { success: true, .. }`,
///    hand the WebSocket to the creator's task via the pending oneshot, and
///    exit. The creator's task drives the per-game lifecycle from here on.
#[allow(clippy::too_many_arguments)]
async fn run_join_flow(
    mut ws_stream: WebSocketStream<TcpStream>,
    lobby: SharedLobby,
    config: ServerConfig,
    server_password: String,
    game_name: String,
    game_password: Option<String>,
    player_name: Option<String>,
    deck: DeckSubmission,
    rendezvous: bool,
) -> Result<()> {
    if !check_server_password(&config, &server_password) {
        send_message(
            &mut ws_stream,
            &ServerMessage::JoinFailed {
                game_name: game_name.clone(),
                reason: JoinFailReason::BadServerPassword,
            },
        )
        .await?;
        return Ok(());
    }

    if let AdmissionVerdict::Reject {
        memory,
        ceiling_percent,
    } = check_memory_admission(config.max_memory_percent)
    {
        return refuse_with_server_full(&mut ws_stream, Some(memory.used_percent()), ceiling_percent).await;
    }

    if deck.main_deck_size() < 40 {
        send_message(
            &mut ws_stream,
            &ServerMessage::JoinFailed {
                game_name,
                reason: JoinFailReason::InvalidDeck {
                    detail: format!("Deck too small: {} cards (minimum 40)", deck.main_deck_size()),
                },
            },
        )
        .await?;
        return Ok(());
    }

    let joiner_name = player_name.unwrap_or_else(|| "Player2".to_string());
    let key = game_name.to_lowercase();

    // Atomically: look up + password-check. We do NOT remove yet — we update
    // the joiner's state in the pending entry first so the creator's watch
    // channel fires a WaitingRoomUpdate, then we either:
    //   - (legacy) take the handoff sender + remove the entry, OR
    //   - (rendezvous) leave the entry in place and subscribe to the watch
    //     channel so the joiner can run its own waiting-room loop.
    let (handoff_tx, creator_update_tx, joiner_update_rx, initial_snapshot, is_rendezvous) = {
        let mut l = lobby.lock().await;
        let Some(pg) = l.waiting_games.get_mut(&key) else {
            drop(l);
            send_message(
                &mut ws_stream,
                &ServerMessage::JoinFailed {
                    game_name,
                    reason: JoinFailReason::NotFound,
                },
            )
            .await?;
            return Ok(());
        };

        if pg.has_password {
            let supplied = game_password.as_deref().map(hash_game_password);
            if supplied != pg.password_hash {
                drop(l);
                send_message(
                    &mut ws_stream,
                    &ServerMessage::JoinFailed {
                        game_name,
                        reason: JoinFailReason::BadPassword,
                    },
                )
                .await?;
                return Ok(());
            }
        }

        // The pending game's own flag is authoritative; OR the joiner's request
        // in case the joiner opted in but the game was created legacy.
        let is_rendezvous = pg.rendezvous || rendezvous;

        // Register joiner state in the pending entry.
        let joiner_state = WaitingPlayerState {
            deck: Some(deck.clone()),
            ready: false,
        };
        pg.joiner_name = Some(joiner_name.clone());
        pg.joiner_state = Some(joiner_state);

        let snap = WaitingRoomSnapshot {
            creator_name: pg.creator_name.clone(),
            creator_state: pg.creator_state.clone(),
            joiner_name: pg.joiner_name.clone(),
            joiner_state: pg.joiner_state.clone(),
            start_game: false,
        };

        // Notify the creator task that a joiner arrived.
        if let Some(tx) = &pg.creator_update_tx {
            let _ = tx.send(snap.clone());
        }

        if is_rendezvous {
            // Rendezvous: keep the entry (slot stays listed until both ready),
            // subscribe to the watch channel, and run a joiner waiting loop.
            let joiner_rx = pg.creator_update_tx.as_ref().map(|tx| tx.subscribe());
            let update_tx = pg.creator_update_tx.clone();
            (None, update_tx, joiner_rx, snap, true)
        } else {
            // Legacy: take the handoff sender + remove the entry; the creator's
            // task picks up the joiner and starts the game immediately.
            let handoff_tx = pg.handoff_tx.take().expect("handoff_tx must be present");
            let update_tx = pg.creator_update_tx.clone();
            l.waiting_games.remove(&key);
            (Some(handoff_tx), update_tx, None, snap, false)
        }
    };

    send_message(
        &mut ws_stream,
        &ServerMessage::AuthResult {
            success: true,
            error: None,
            your_player_id: Some(PlayerId::new(1)),
            your_name: Some(joiner_name.clone()),
        },
    )
    .await?;

    // Send the initial waiting-room state to the joiner.
    let update_msg = initial_snapshot.to_server_message();
    send_message(&mut ws_stream, &update_msg).await?;

    if is_rendezvous {
        // Variant-1 rendezvous: the joiner runs its own waiting-room loop on
        // its lobby socket. The game runs on each player's GAME PAGE socket,
        // not here, so this loop never calls run_game — on both-ready it sends
        // WaitingRoomReady and returns. See WaitingRoomReady protocol docs.
        let result = run_joiner_waiting_room(&mut ws_stream, &lobby, &key, &game_name, joiner_update_rx).await;
        drop(creator_update_tx);
        return result;
    }

    // Hand the WebSocket to the creator's task. If the Sender is gone (creator
    // died after we removed the entry) we lose the joiner — close cleanly.
    let payload = JoinedPlayer {
        name: joiner_name,
        deck,
        ws_stream,
    };
    if let Some(handoff_tx) = handoff_tx {
        if handoff_tx.send(payload).is_err() {
            log::error!("Pending game '{game_name}' creator gone — joiner dropped");
        }
    }
    drop(creator_update_tx); // release our handle so the watch channel closes cleanly
    Ok(())
}

/// Joiner-side waiting-room loop for the Variant-1 rendezvous (mtg-682).
///
/// Mirrors the creator's in-loop `SetDeck`/`SetReady`/`Ping` handling but for
/// the joiner socket. Updates the joiner's slice of the shared `PendingGame`
/// state, broadcasts `WaitingRoomUpdate` to the creator via the watch channel,
/// and forwards creator-side updates to the joiner. When both players are
/// ready it sends `WaitingRoomReady` to the joiner and returns (the creator's
/// task does the same on its side); the actual game runs on the GAME PAGE.
async fn run_joiner_waiting_room(
    ws_stream: &mut WebSocketStream<TcpStream>,
    lobby: &SharedLobby,
    key: &str,
    game_name: &str,
    update_rx: Option<tokio::sync::watch::Receiver<WaitingRoomSnapshot>>,
) -> Result<()> {
    let Some(mut update_rx) = update_rx else {
        // No watch channel — the creator already vanished; nothing to wait on.
        let _ = send_error(ws_stream, "Waiting room is no longer available", true).await;
        return Ok(());
    };

    /// Rebuild a snapshot from the live PendingGame after mutating the joiner's
    /// state, broadcast it, and report whether both players are now ready.
    fn rebuild_and_broadcast(pg: &PendingGame, rendezvous_both: bool) -> (WaitingRoomSnapshot, bool) {
        let joiner_ready = pg.joiner_state.as_ref().map(|s| s.ready).unwrap_or(false);
        let both = pg.creator_state.ready && joiner_ready;
        let snap = WaitingRoomSnapshot {
            creator_name: pg.creator_name.clone(),
            creator_state: pg.creator_state.clone(),
            joiner_name: pg.joiner_name.clone(),
            joiner_state: pg.joiner_state.clone(),
            start_game: rendezvous_both && both,
        };
        (snap, both)
    }

    loop {
        tokio::select! {
            // Creator-side state changed (deck/ready) — forward to the joiner.
            changed = update_rx.changed() => {
                if changed.is_err() {
                    // Creator's task dropped the channel — the game is gone.
                    let _ = send_error(ws_stream, "Host left the waiting room", true).await;
                    return Ok(());
                }
                let start_game = update_rx.borrow().start_game;
                let msg = update_rx.borrow().to_server_message();
                if send_message(ws_stream, &msg).await.is_err() {
                    let mut l = lobby.lock().await;
                    l.waiting_games.remove(key);
                    return Ok(());
                }
                if start_game {
                    log::info!("Game ({game_name}): both ready (rendezvous) — signalling joiner to proceed");
                    let _ = send_message(
                        ws_stream,
                        &ServerMessage::WaitingRoomReady {
                            game_name: game_name.to_string(),
                            is_creator: false,
                        },
                    )
                    .await;
                    let mut l = lobby.lock().await;
                    l.waiting_games.remove(key);
                    return Ok(());
                }
            }

            // Joiner sends a message (SetDeck / SetReady / Ping / Disconnect).
            msg = read_one_lobby_message(ws_stream) => {
                match msg {
                    Err(_) => {
                        // Joiner disconnected — clear the joiner slice so the
                        // creator sees "opponent left" and the slot reverts to
                        // waiting-for-opponent rather than leaking.
                        let mut l = lobby.lock().await;
                        if let Some(pg) = l.waiting_games.get_mut(key) {
                            pg.joiner_name = None;
                            pg.joiner_state = None;
                            let snap = WaitingRoomSnapshot {
                                creator_name: pg.creator_name.clone(),
                                creator_state: pg.creator_state.clone(),
                                joiner_name: None,
                                joiner_state: None,
                                start_game: false,
                            };
                            if let Some(tx) = &pg.creator_update_tx {
                                let _ = tx.send(snap);
                            }
                        }
                        return Ok(());
                    }
                    Ok(ClientMessage::SetDeck { deck: new_deck }) => {
                        if new_deck.main_deck_size() < 40 {
                            let _ = send_error(
                                ws_stream,
                                &format!("Deck too small: {} cards (minimum 40)", new_deck.main_deck_size()),
                                false,
                            )
                            .await;
                        } else {
                            let snapshot = {
                                let mut l = lobby.lock().await;
                                if let Some(pg) = l.waiting_games.get_mut(key) {
                                    if let Some(js) = pg.joiner_state.as_mut() {
                                        js.deck = Some(new_deck);
                                        js.ready = false; // reset on deck change
                                    }
                                    let (snap, _) = rebuild_and_broadcast(pg, true);
                                    if let Some(tx) = &pg.creator_update_tx {
                                        let _ = tx.send(snap.clone());
                                    }
                                    Some(snap)
                                } else {
                                    None
                                }
                            };
                            if let Some(snap) = snapshot {
                                let _ = send_message(ws_stream, &snap.to_server_message()).await;
                            }
                        }
                    }
                    Ok(ClientMessage::SetReady { ready }) => {
                        let (snapshot, both_ready) = {
                            let mut l = lobby.lock().await;
                            if let Some(pg) = l.waiting_games.get_mut(key) {
                                let has_deck = pg.joiner_state.as_ref().and_then(|s| s.deck.as_ref()).is_some();
                                if ready && !has_deck {
                                    (None, false)
                                } else {
                                    if let Some(js) = pg.joiner_state.as_mut() {
                                        js.ready = ready;
                                    }
                                    let (snap, both) = rebuild_and_broadcast(pg, true);
                                    if let Some(tx) = &pg.creator_update_tx {
                                        let _ = tx.send(snap.clone());
                                    }
                                    (Some(snap), both)
                                }
                            } else {
                                (None, false)
                            }
                        };
                        if ready && snapshot.is_none() {
                            let _ = send_error(ws_stream, "Cannot ready without a deck", false).await;
                        } else if let Some(snap) = snapshot {
                            let _ = send_message(ws_stream, &snap.to_server_message()).await;
                            if both_ready {
                                log::info!("Game ({game_name}): both ready (rendezvous) — signalling joiner to proceed");
                                let _ = send_message(
                                    ws_stream,
                                    &ServerMessage::WaitingRoomReady {
                                        game_name: game_name.to_string(),
                                        is_creator: false,
                                    },
                                )
                                .await;
                                let mut l = lobby.lock().await;
                                l.waiting_games.remove(key);
                                return Ok(());
                            }
                        }
                    }
                    Ok(ClientMessage::Ping { timestamp_ms }) => {
                        let _ = send_message(ws_stream, &ServerMessage::Pong { timestamp_ms }).await;
                    }
                    Ok(ClientMessage::Disconnect) => {
                        let mut l = lobby.lock().await;
                        if let Some(pg) = l.waiting_games.get_mut(key) {
                            pg.joiner_name = None;
                            pg.joiner_state = None;
                        }
                        return Ok(());
                    }
                    Ok(_) => {
                        let _ = send_error(ws_stream, "Unexpected message in waiting room", false).await;
                    }
                }
            }
        }
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
        commanders: Vec::new(),
    }
}

/// Run a single game between two players
///
/// Note: Wildcard is intentional - GameEndReason enum has several variants;
/// we handle PlayerDeath/Decking specially and derive from winner for others.
#[allow(clippy::wildcard_enum_match_arm)]
async fn run_game(
    game_id: u64,
    p1: WaitingPlayer,
    p2: WaitingPlayer,
    card_db: Arc<AsyncCardDatabase>,
    config: ServerConfig,
) -> Result<()> {
    // Extract final names (should always be Some at this point)
    let p1_name = p1.name.clone().unwrap_or_else(|| "Player1".to_string());
    let p2_name = p2.name.clone().unwrap_or_else(|| "Player2".to_string());
    let p1_reconnect_token = p1.reconnect_token.clone();
    let p2_reconnect_token = p2.reconnect_token.clone();
    log::info!("Game {}: Initializing {} vs {}", game_id, p1_name, p2_name);

    // ═══════════════════════════════════════════════════════════════════════
    // SINGLE-CHANNEL ARCHITECTURE (mtg-228)
    // ═══════════════════════════════════════════════════════════════════════
    //
    // The architecture has three layers:
    // 1. NetworkControllers (sync) - run in spawn_blocking, drive game loop
    // 2. Coordinator task (async) - bridges sync/async, routes messages
    // 3. Player handlers (async) - handle WebSocket I/O
    //
    // Messages flow:
    //   NetworkController --sync--> Coordinator --async--> Handler --WS--> Client
    //   NetworkController <--sync-- Coordinator <--async-- Handler <--WS-- Client

    // Create sync channels for NetworkControllers (used by game loop in blocking thread)
    let (p1_request_tx, p1_sync_request_rx) = std::sync::mpsc::channel::<ChoiceRequest>();
    let (p1_response_tx, p1_response_rx) = std::sync::mpsc::channel::<ChoiceResponse>();
    let (p2_request_tx, p2_sync_request_rx) = std::sync::mpsc::channel::<ChoiceRequest>();
    let (p2_response_tx, p2_response_rx) = std::sync::mpsc::channel::<ChoiceResponse>();

    // Create SINGLE async channel pairs for handler communication (mtg-228)
    // Each player has exactly one rx and one tx with the coordinator
    let (p1_to_handler_tx, p1_game_rx) = tokio_mpsc::channel::<GameToHandler>(16);
    let (p1_from_handler_tx, p1_handler_rx) = tokio_mpsc::channel::<HandlerToGame>(16);
    let (p2_to_handler_tx, p2_game_rx) = tokio_mpsc::channel::<GameToHandler>(16);
    let (p2_from_handler_tx, p2_handler_rx) = tokio_mpsc::channel::<HandlerToGame>(16);

    // Split WebSocket streams
    let (p1_ws_tx, p1_ws_rx) = p1.ws_stream.split();
    let (p2_ws_tx, p2_ws_rx) = p2.ws_stream.split();

    // Create PlayerConnections with single-channel architecture
    let mut p1_conn = PlayerConnection {
        player_id: PlayerId::new(0),
        ws_tx: p1_ws_tx,
        game_rx: p1_game_rx,
        game_tx: p1_from_handler_tx,
        pending_choice: None,
    };
    let mut p2_conn = PlayerConnection {
        player_id: PlayerId::new(1),
        ws_tx: p2_ws_tx,
        game_rx: p2_game_rx,
        game_tx: p2_from_handler_tx,
        pending_choice: None,
    };

    // Convert deck submissions to DeckList format
    let p1_decklist = submission_to_decklist(&p1.deck);
    let p2_decklist = submission_to_decklist(&p2.deck);

    // Debug: log deck order for entity ID verification
    log::debug!(
        "Server init: p1_deck entries={}, p2_deck entries={}",
        p1_decklist.main_deck.len(),
        p2_decklist.main_deck.len()
    );
    if log::log_enabled!(log::Level::Trace) {
        for (i, entry) in p1_decklist.main_deck.iter().enumerate() {
            log::trace!("P1 deck[{}]: {}x {}", i, entry.count, entry.card_name);
        }
        for (i, entry) in p2_decklist.main_deck.iter().enumerate() {
            log::trace!("P2 deck[{}]: {}x {}", i, entry.count, entry.card_name);
        }
    }

    // Seed RNG and shuffle libraries BEFORE assigning CardIDs (positional ID architecture)
    // This ensures CardID 0 = top card of P1's shuffled library, etc.
    let seed = config.seed.unwrap_or_else(rand::random::<u64>);
    log::info!("Game {}: Using seed {}", game_id, seed);

    // Create game state using GameInitializer with positional CardIDs
    // This shuffles decks BEFORE assigning CardIDs so that:
    // - P1's cards get CardIDs [0..P1_deck_size)
    // - P2's cards get CardIDs [P1_deck_size..total)
    // Clients use init_game_reserve_only with the same ranges
    let initializer = GameInitializer::new(&card_db);
    let mut game = initializer
        .init_game_with_positional_ids(
            p1_name.clone(),
            &p1_decklist,
            p2_name.clone(),
            &p2_decklist,
            config.starting_life,
            seed,
        )
        .await?;

    // Enable reveal logging for network games.
    // RevealCard actions are logged during draw_card, play_land, etc.
    // These are collected by NetworkController and sent to clients as CardRevealed messages.
    game.set_skip_reveals(false);

    let p1_id = game.players[0].id;
    let p2_id = game.players[1].id;

    // Peek at opening hands WITHOUT drawing
    // We don't draw yet because that would add actions to undo_log before GameLoop starts.
    // Both server and client GameLoops need to start with identical (empty) undo_logs
    // so they can draw synchronously and produce matching action sequences.
    let p1_hand = peek_opening_hand(&game, p1_id)?;
    let p2_hand = peek_opening_hand(&game, p2_id)?;

    // Compute initial state hash
    let initial_hash = compute_network_hash(&game);

    // ALWAYS send deck lists for synchronized GameLoop mode.
    // Clients need the opponent's deck list to create matching card IDs.
    // The deck_visibility config is a separate UI concern (whether players
    // can VIEW opponent's decklist), not whether we transmit it for sync.
    let p1_deck_info = Some(DeckListInfo::from_submission(&p1.deck));
    let p2_deck_info = Some(DeckListInfo::from_submission(&p2.deck));

    // Compute deck CardID ranges for late-binding architecture (Phase 3)
    // P1's deck: CardIDs [0, p1_deck_size)
    // P2's deck: CardIDs [p1_deck_size, p1_deck_size + p2_deck_size)
    let p1_deck_size = p1.deck.main_deck_size();
    let p2_deck_size = p2.deck.main_deck_size();
    let deck_card_ids = Some(crate::network::protocol::DeckCardIdRanges::from_deck_sizes(
        p1_deck_size,
        p2_deck_size,
    ));

    // Send GameStarted to both players
    let p1_lib_size = game.player_zones[0].1.library.len();
    let p2_lib_size = game.player_zones[1].1.library.len();

    // Build token_definitions map for network transmission
    // Convert from HashMap<String, Arc<CardDefinition>> to HashMap<String, CardDefinition>
    let token_definitions: std::collections::HashMap<String, crate::loader::CardDefinition> = game
        .token_definitions
        .iter()
        .map(|(k, v)| (k.clone(), (**v).clone()))
        .collect();

    // Serialize RNG state for clients to initialize their shadow RNG
    // This ensures subsequent shuffles (tutors, etc.) produce identical results
    let rng_state = {
        let rng = game.rng.borrow();
        bincode::serialize(&*rng).unwrap_or_else(|e| {
            log::error!("Failed to serialize RNG state: {}", e);
            Vec::new()
        })
    };
    log::debug!(
        "Game {}: Serialized RNG state ({} bytes) for client sync",
        game_id,
        rng_state.len()
    );

    p1_conn
        .send(&ServerMessage::GameStarted {
            your_player_id: p1_id,
            opponent_name: p2_name.clone(),
            opening_hand: p1_hand.clone(),
            opponent_hand_count: p2_hand.len(),
            library_size: p1_lib_size,
            opponent_library_size: p2_lib_size,
            opponent_decklist: p2_deck_info.clone(),
            starting_life: config.starting_life,
            initial_state_hash: initial_hash,
            network_debug: config.network_debug,
            deck_card_ids: deck_card_ids.clone(),
            token_definitions: token_definitions.clone(),
            rng_state: rng_state.clone(),
            reconnect_token: p1_reconnect_token,
        })
        .await?;

    p2_conn
        .send(&ServerMessage::GameStarted {
            your_player_id: p2_id,
            opponent_name: p1_name.clone(),
            opening_hand: p2_hand.clone(),
            opponent_hand_count: p1_hand.len(),
            library_size: p2_lib_size,
            opponent_library_size: p1_lib_size,
            opponent_decklist: p1_deck_info.clone(),
            starting_life: config.starting_life,
            initial_state_hash: initial_hash,
            network_debug: config.network_debug,
            deck_card_ids: deck_card_ids.clone(),
            token_definitions: token_definitions.clone(),
            rng_state: rng_state.clone(),
            reconnect_token: p2_reconnect_token,
        })
        .await?;

    log::info!("Game {}: Sent GameStarted to both players", game_id);

    // Send LibraryReordered messages to sync initial library order with clients.
    // The server shuffles deck definitions BEFORE assigning CardIDs (via init_game_with_positional_ids).
    // This means CardID 0 is the top of the shuffled P1 library, not the first card in the deck file.
    // Clients use init_game_reserve_only which creates sequential CardIDs without shuffle knowledge.
    // Without this sync, clients would have [0, 1, 2, ...] as top-to-bottom order, causing desync
    // when the GameLoop draws cards.
    //
    // Format: top-to-bottom (reversed from internal Vec representation which is bottom-to-top)
    let p1_lib_order: Vec<CardId> = game
        .get_player_zones(p1_id)
        .map(|z| z.library.cards.iter().rev().copied().collect())
        .unwrap_or_default();
    let p2_lib_order: Vec<CardId> = game
        .get_player_zones(p2_id)
        .map(|z| z.library.cards.iter().rev().copied().collect())
        .unwrap_or_default();

    // Both clients receive both library orders for symmetry
    // Game-start library sync: no undo actions have been logged yet, so the
    // alignment ac is 0 (mtg-o99ow). The shadow adopts this order before any
    // draw. Real reorder acs (shuffle / scry / surveil mid-game) are stamped at
    // their own undo-log position by the reorder-emission paths below.
    p1_conn
        .send(&ServerMessage::LibraryReordered {
            player: p1_id,
            new_order: p1_lib_order.clone(),
            action_count: 0,
        })
        .await?;
    p1_conn
        .send(&ServerMessage::LibraryReordered {
            player: p2_id,
            new_order: p2_lib_order.clone(),
            action_count: 0,
        })
        .await?;
    p2_conn
        .send(&ServerMessage::LibraryReordered {
            player: p1_id,
            new_order: p1_lib_order,
            action_count: 0,
        })
        .await?;
    p2_conn
        .send(&ServerMessage::LibraryReordered {
            player: p2_id,
            new_order: p2_lib_order,
            action_count: 0,
        })
        .await?;

    log::debug!(
        "Game {}: Sent initial LibraryReordered to sync client library zones",
        game_id
    );

    // Send CardRevealed messages for opening hands (for synchronized GameLoop mode)
    // ALL players receive reveals for ALL cards to keep action_count in sync.
    // But opponent's cards are sent as "dummy reveals" with name stripped.
    //
    // HIDDEN INFO ARCHITECTURE (mtg-218):
    // - Own cards: real reveal with name (player can see their hand)
    // - Opponent cards: dummy reveal with empty name (keeps count synced, reveals nothing)

    // mtg-o99ow L2c: stamp each opening-hand reveal at its REAL per-draw game
    // `action_count` (the undo-log position of that draw's `RevealCard`), NOT a
    // bundled `Some(0)`. The shadow runs the SAME `skip_opening_hands` GameLoop
    // and draws P1's 7 then P2's 7 via `draw_card_silent`, each draw logging
    // EXACTLY three undo actions in this order: `RevealCard`, `MoveCard`,
    // `SetCardsDrawnThisTurn` (empirically pinned — see `peek_opening_hand` draw
    // order is top-of-library-first, matching the GameLoop's draw order). So the
    // k-th globally-drawn card's `RevealCard` lands at undo position
    // `OPENING_DRAW_UNDO_ACTIONS * k`, where P1's cards are k = 0..p1_hand.len()
    // and P2's are k = p1_hand.len()..(p1+p2). Distinct per-draw acs are REQUIRED
    // by the game-ac-keyed shadow log (L3): the prior `Some(0)` would collide N
    // reveals at one ac and panic `ActionLog::push`.
    const OPENING_DRAW_UNDO_ACTIONS: u64 = 3;
    let p1_count = p1_hand.len() as u64;
    let opening_reveal_ac = |global_draw_idx: u64| Some(global_draw_idx * OPENING_DRAW_UNDO_ACTIONS);

    // P1 receives: own hand (real reveals) + P2's hand (dummy reveals)
    for (i, card) in p1_hand.iter().enumerate() {
        p1_conn
            .send(&ServerMessage::CardRevealed {
                owner: p1_id,
                card: card.clone(), // Real reveal - P1 sees own cards
                reason: RevealReason::Draw,
                action_count: opening_reveal_ac(i as u64),
            })
            .await?;
    }
    for (j, card) in p2_hand.iter().enumerate() {
        // Dummy reveal: strip name so P1 can't see P2's hand
        let dummy_reveal = CardReveal {
            card_id: card.card_id,
            name: String::new(), // Hidden - P1 doesn't know what card this is
            card_def: None,      // No definition for dummy reveals
        };
        p1_conn
            .send(&ServerMessage::CardRevealed {
                owner: p2_id,
                card: dummy_reveal,
                reason: RevealReason::Draw,
                action_count: opening_reveal_ac(p1_count + j as u64),
            })
            .await?;
    }

    // P2 receives: P1's hand (dummy reveals) + own hand (real reveals)
    for (i, card) in p1_hand.iter().enumerate() {
        // Dummy reveal: strip name so P2 can't see P1's hand
        let dummy_reveal = CardReveal {
            card_id: card.card_id,
            name: String::new(), // Hidden - P2 doesn't know what card this is
            card_def: None,      // No definition for dummy reveals
        };
        p2_conn
            .send(&ServerMessage::CardRevealed {
                owner: p1_id,
                card: dummy_reveal,
                reason: RevealReason::Draw,
                action_count: opening_reveal_ac(i as u64),
            })
            .await?;
    }
    for (j, card) in p2_hand.iter().enumerate() {
        p2_conn
            .send(&ServerMessage::CardRevealed {
                owner: p2_id,
                card: card.clone(), // Real reveal - P2 sees own cards
                reason: RevealReason::Draw,
                action_count: opening_reveal_ac(p1_count + j as u64),
            })
            .await?;
    }

    log::info!("Game {}: Sent opening hand CardRevealed messages", game_id);

    // Calculate baseline reveal index to skip the opening hand draws.
    //
    // `shared_reveal_index` is an UNDO-LOG ACTION index (collect_reveals_since_last_choice
    // does `actions.iter().skip(idx)` and `forward_idx < idx => break`), NOT a card count.
    // Each opening-hand draw logs EXACTLY `OPENING_DRAW_UNDO_ACTIONS` undo actions
    // (RevealCard, MoveCard, SetCardsDrawnThisTurn — same empirical pin as the L2c
    // reveal-ac stamping), so the GameLoop's skip_opening_hands draws occupy undo
    // positions 0..(total_cards * OPENING_DRAW_UNDO_ACTIONS). We pre-sent ALL opening-hand
    // reveals above at their real per-draw acs, so the controller MUST skip that entire
    // action span — otherwise it re-collects each player's own opening-hand draws at the
    // SAME forward_idx the pre-send already used, which under the game-`action_count`-keyed
    // shadow log (mtg-o99ow L3) is a duplicate/out-of-order `ActionLog::push` => FATAL
    // (last>new). The prior `opening_hand_count` (CARD count) value under-skipped by 2x and
    // only survived because the legacy synthetic key tolerated the re-collected duplicates.
    let opening_hand_count = p1_hand.len() + p2_hand.len();
    let opening_hand_action_count = opening_hand_count * OPENING_DRAW_UNDO_ACTIONS as usize;
    log::debug!(
        "Game {}: Opening hand = {} cards / {} undo actions (skip in first ChoiceRequest)",
        game_id,
        opening_hand_count,
        opening_hand_action_count
    );

    // Create shared reveal indices for NetworkControllers (undo-ACTION index).
    let p1_reveal_index = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(opening_hand_action_count));
    let p2_reveal_index = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(opening_hand_action_count));

    // Create NetworkControllers with shared reveal indices
    let mut p1_controller = NetworkController::new(p1_id, p1_request_tx, p1_response_rx, Arc::clone(&p1_reveal_index));
    let mut p2_controller = NetworkController::new(p2_id, p2_request_tx, p2_response_rx, Arc::clone(&p2_reveal_index));
    p1_controller.set_network_debug(config.network_debug);
    p2_controller.set_network_debug(config.network_debug);

    // Wrap game state for sharing between tasks
    let game = Arc::new(Mutex::new(game));

    // Spawn WebSocket handlers for each player
    let game_clone = Arc::clone(&game);
    let p1_config = config.clone();
    let mut p1_handler =
        tokio::spawn(async move { handle_player_websocket(p1_conn, p1_ws_rx, game_clone, p1_config).await });

    let game_clone = Arc::clone(&game);
    let p2_config = config.clone();
    let mut p2_handler =
        tokio::spawn(async move { handle_player_websocket(p2_conn, p2_ws_rx, game_clone, p2_config).await });

    // Create channel for game end notification to coordinator
    let (game_end_tx, game_end_rx) = oneshot::channel::<GameEndInfo>();

    // Spawn coordinator task that bridges sync NetworkControllers to async handlers
    // The coordinator receives ChoiceRequests from NetworkControllers (via sync channels),
    // forwards them to handlers (via async channels), and routes responses back.
    let coordinator_network_debug = config.network_debug;
    let mut coordinator_handle = tokio::spawn(async move {
        run_coordinator(
            p1_sync_request_rx,
            p1_response_tx,
            p1_to_handler_tx,
            p1_handler_rx,
            p2_sync_request_rx,
            p2_response_tx,
            p2_to_handler_tx,
            p2_handler_rx,
            coordinator_network_debug,
            game_end_rx,
        )
        .await
    });

    // Run game loop in blocking thread (uses sync channels)
    let game_clone = Arc::clone(&game);
    let tag_gamelogs = config.tag_gamelogs;
    let verbosity = config.verbosity;
    let no_color_logs = config.no_color_logs;
    let game_loop_handle = tokio::task::spawn_blocking(move || {
        run_game_loop(
            game_clone,
            p1_controller,
            p2_controller,
            tag_gamelogs,
            verbosity,
            no_color_logs,
        )
    });

    // Wait for game to complete, OR for any critical task to fail
    // This prevents the server from hanging when a desync is detected
    let result: Result<GameResult> = tokio::select! {
        // Game loop completed (success or error)
        game_result = game_loop_handle => {
            match game_result {
                Ok(r) => r,
                Err(e) => Err(anyhow!("Game loop panic: {}", e)),
            }
        }
        // Coordinator exited (error means fatal issue)
        coord_result = &mut coordinator_handle => {
            log::error!("Game {}: Coordinator exited unexpectedly: {:?}", game_id, coord_result);
            Err(anyhow!("Coordinator terminated unexpectedly"))
        }
        // P1 WebSocket handler exited (error means fatal issue like desync)
        p1_result = &mut p1_handler => {
            log::error!("Game {}: P1 handler exited unexpectedly: {:?}", game_id, p1_result);
            Err(anyhow!("P1 connection terminated unexpectedly"))
        }
        // P2 WebSocket handler exited (error means fatal issue like desync)
        p2_result = &mut p2_handler => {
            log::error!("Game {}: P2 handler exited unexpectedly: {:?}", game_id, p2_result);
            Err(anyhow!("P2 connection terminated unexpectedly"))
        }
    };

    // Get final state hash for the GameEnded message
    let final_hash = {
        let game_guard = game.lock().await;
        compute_network_hash(&game_guard)
    };

    // Send game end notification to coordinator, which will forward to handlers
    match &result {
        Ok(game_result) => {
            log::info!(
                "Game {}: Completed, winner = {:?}, action_count = {}",
                game_id,
                game_result.winner,
                game_result.action_count
            );

            // Derive reason from game result
            let reason = match game_result.end_reason {
                GameEndReason::PlayerDeath(_) | GameEndReason::Decking(_) => game_result.end_reason.clone(),
                _ => match game_result.winner {
                    Some(winner_id) => {
                        let loser_id = if winner_id == PlayerId::new(0) {
                            PlayerId::new(1)
                        } else {
                            PlayerId::new(0)
                        };
                        GameEndReason::PlayerDeath(loser_id)
                    }
                    None => GameEndReason::Draw,
                },
            };

            let end_info = GameEndInfo {
                winner: game_result.winner,
                reason,
                final_hash,
                action_count: game_result.action_count,
            };
            let _ = game_end_tx.send(end_info);
        }
        Err(e) => {
            log::error!("Game {}: Error - {}", game_id, e);
            let end_info = GameEndInfo {
                winner: None,
                reason: GameEndReason::Draw,
                final_hash,
                action_count: 0,
            };
            let _ = game_end_tx.send(end_info);
        }
    }

    // Wait for coordinator and handlers to process GameEnded
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    coordinator_handle.abort();
    p1_handler.abort();
    p2_handler.abort();

    // Return the final result
    result.map(|_| ())
}

// ═══════════════════════════════════════════════════════════════════════════
// COORDINATOR TASK (mtg-228)
// ═══════════════════════════════════════════════════════════════════════════
//
// The coordinator bridges sync NetworkControllers to async WebSocket handlers.
// It runs a blocking receiver in spawn_blocking, and routes messages to handlers.
//
// Key responsibilities:
// 1. Receive ChoiceRequest from NetworkController (sync) via spawn_blocking bridge
// 2. Forward to appropriate handler (async)
// 3. Receive ChoiceResponse from handler (async)
// 4. Send response to NetworkController (sync)
// 5. Route OpponentMadeChoice to the other handler
// 6. Send ChoiceAccepted to the originating handler

/// Run the coordinator task that bridges sync NetworkControllers to async handlers.
///
/// This task uses spawn_blocking to receive from sync channels, then routes
/// messages through the single async channel to each handler.
#[allow(clippy::too_many_arguments)]
async fn run_coordinator(
    p1_sync_request_rx: std::sync::mpsc::Receiver<ChoiceRequest>,
    p1_response_tx: std::sync::mpsc::Sender<ChoiceResponse>,
    p1_to_handler_tx: tokio_mpsc::Sender<GameToHandler>,
    mut p1_handler_rx: tokio_mpsc::Receiver<HandlerToGame>,
    p2_sync_request_rx: std::sync::mpsc::Receiver<ChoiceRequest>,
    p2_response_tx: std::sync::mpsc::Sender<ChoiceResponse>,
    p2_to_handler_tx: tokio_mpsc::Sender<GameToHandler>,
    mut p2_handler_rx: tokio_mpsc::Receiver<HandlerToGame>,
    network_debug: bool,
    mut game_end_rx: oneshot::Receiver<GameEndInfo>,
) -> Result<()> {
    // Spawn blocking bridge tasks that convert sync channel messages to async
    let (p1_bridge_tx, mut p1_bridge_rx) = tokio_mpsc::channel::<ChoiceRequest>(4);
    let (p2_bridge_tx, mut p2_bridge_rx) = tokio_mpsc::channel::<ChoiceRequest>(4);

    let _p1_bridge = tokio::task::spawn_blocking(move || {
        while let Ok(request) = p1_sync_request_rx.recv() {
            if p1_bridge_tx.blocking_send(request).is_err() {
                break;
            }
        }
    });

    let _p2_bridge = tokio::task::spawn_blocking(move || {
        while let Ok(request) = p2_sync_request_rx.recv() {
            if p2_bridge_tx.blocking_send(request).is_err() {
                break;
            }
        }
    });

    log::debug!("Coordinator: Started, network_debug={}", network_debug);

    // Main coordinator loop: wait for choice requests from either player OR game end
    loop {
        tokio::select! {
            // Game ended - forward to both handlers and exit
            end_info = &mut game_end_rx => {
                match end_info {
                    Ok(info) => {
                        log::info!("Coordinator: Received GameEnded, winner={:?}", info.winner);
                        // Send GameEnded to both handlers
                        let _ = p1_to_handler_tx.send(GameToHandler::GameEnded(info.clone())).await;
                        let _ = p2_to_handler_tx.send(GameToHandler::GameEnded(info)).await;
                        return Ok(());
                    }
                    Err(_) => {
                        log::debug!("Coordinator: Game end channel closed unexpectedly");
                        return Ok(());
                    }
                }
            }

            // P1 NetworkController sent a ChoiceRequest
            request = p1_bridge_rx.recv() => {
                match request {
                    Some(choice_request) => {
                        log::debug!(
                            "Coordinator: P1 ChoiceRequest seq={} action_count={}",
                            choice_request.choice_seq, choice_request.action_count
                        );

                        // Store request info for later (for OpponentMadeChoice)
                        let choice_seq = choice_request.choice_seq;
                        let choice_type = choice_request.choice_type.clone();
                        let action_count = choice_request.action_count;
                        let abilities = choice_request.abilities.clone();
                        let library_search_cards = choice_request.library_search_cards.clone();
                        // For network debug: capture server's state hash and debug info
                        let server_state_hash = choice_request.state_hash;
                        let server_debug_info = choice_request.debug_info.clone();

                        // mtg-420: Broadcast any pending LibraryReorders (from server-side
                        // scry/surveil) to BOTH clients BEFORE forwarding the ChoiceRequest.
                        // The client's sync_callback drains LibraryReordered queue at its
                        // priority sync point, ensuring its shadow library matches the server
                        // before ability enumeration. Sending to both clients keeps the
                        // opponent's shadow game in sync as well.
                        if !choice_request.library_reorders.is_empty() {
                            log::debug!(
                                "Coordinator: Broadcasting {} library reorder(s) before P1 ChoiceRequest",
                                choice_request.library_reorders.len()
                            );
                            for (player, new_order, action_count) in &choice_request.library_reorders {
                                let msg_p1 = GameToHandler::LibraryReordered {
                                    player: *player,
                                    new_order: new_order.clone(),
                                    action_count: *action_count,
                                };
                                let msg_p2 = GameToHandler::LibraryReordered {
                                    player: *player,
                                    new_order: new_order.clone(),
                                    action_count: *action_count,
                                };
                                if p1_to_handler_tx.send(msg_p1).await.is_err() {
                                    log::error!("Coordinator: Failed to send LibraryReordered to P1");
                                    return Err(anyhow!("P1 handler channel closed"));
                                }
                                if p2_to_handler_tx.send(msg_p2).await.is_err() {
                                    log::error!("Coordinator: Failed to send LibraryReordered to P2");
                                    return Err(anyhow!("P2 handler channel closed"));
                                }
                            }
                        }

                        // Forward to P1 handler
                        if p1_to_handler_tx.send(GameToHandler::ChoiceRequest(Box::new(choice_request))).await.is_err() {
                            log::error!("Coordinator: Failed to send ChoiceRequest to P1 handler");
                            return Err(anyhow!("P1 handler channel closed"));
                        }

                        // Wait for P1's response
                        match p1_handler_rx.recv().await {
                            Some(HandlerToGame::ChoiceResponse { response, client_action_count, client_state_hash, client_debug_info }) => {
                                log::debug!(
                                    "Coordinator: P1 ChoiceResponse seq={} indices={:?}",
                                    response.choice_seq, response.choice_indices
                                );
                                // mtg-mb668 class-A snapshot verification: ground-truth of what the
                                // server RECEIVED for this P1 response vs what it expects, so the
                                // WASM_SUBMIT (seq,ac,hash) can be aligned and choice_seq↔ac↔hash
                                // misalignment detected. network_debug-gated (opt-in diagnostics).
                                if network_debug {
                                    log::warn!(
                                        "SRV_P1_RECV seq={} client_ac={} expected_ac={} client_hash={} server_hash={:016x}",
                                        response.choice_seq,
                                        client_action_count,
                                        action_count,
                                        client_state_hash.map(|h| format!("{h:016x}")).unwrap_or_else(|| "none".to_string()),
                                        server_state_hash,
                                    );
                                }

                                // Validate action_count
                                if client_action_count != action_count {
                                    let error_msg = format!(
                                        "FATAL: P1 action_count mismatch! client={} expected={}",
                                        client_action_count, action_count
                                    );
                                    log::error!("Coordinator: {}", error_msg);
                                    // Send fatal error to both handlers
                                    let _ = p1_to_handler_tx.send(GameToHandler::FatalError(error_msg.clone())).await;
                                    let _ = p2_to_handler_tx.send(GameToHandler::FatalError(error_msg.clone())).await;
                                    return Err(anyhow!("{}", error_msg));
                                }

                                // Validate state hash in network debug mode
                                if network_debug {
                                    if let Some(client_hash) = client_state_hash {
                                        if client_hash != server_state_hash {
                                            log_state_hash_mismatch(
                                                "P1",
                                                choice_seq,
                                                action_count,
                                                server_state_hash,
                                                client_hash,
                                                &server_debug_info,
                                                &client_debug_info,
                                            );
                                            // Per NETWORK_ARCHITECTURE.md: desync is ALWAYS fatal
                                            let error_msg = format!(
                                                "FATAL: P1 state hash mismatch! server={:016x} client={:016x} at choice_seq={} action_count={}",
                                                server_state_hash, client_hash, choice_seq, action_count
                                            );
                                            log::error!("Coordinator: {}", error_msg);
                                            let _ = p1_to_handler_tx.send(GameToHandler::FatalError(error_msg.clone())).await;
                                            let _ = p2_to_handler_tx.send(GameToHandler::FatalError(error_msg.clone())).await;
                                            return Err(anyhow!("{}", error_msg));
                                        }
                                    }
                                }

                                // Send response to NetworkController
                                if p1_response_tx.send(response.clone()).is_err() {
                                    log::error!("Coordinator: Failed to send response to P1 NetworkController");
                                    return Err(anyhow!("P1 NetworkController channel closed"));
                                }

                                // Extract spell_ability for Priority choices
                                let spell_ability = if matches!(choice_type, ChoiceType::Priority { .. }) {
                                    let idx = response.choice_indices.first().copied().unwrap_or(0);
                                    abilities.as_ref()
                                        .and_then(|abs| abs.get(idx).cloned())
                                        .flatten()
                                } else {
                                    None
                                };

                                // Extract library_search_result for LibrarySearchByName choices
                                // Client sends [name_idx+1, instance_idx] where name_idx+1=0 means "Decline"
                                // We decode using name_counts to find the flat index in library_search_cards
                                let library_search_result = if let ChoiceType::LibrarySearchByName { ref name_counts, .. } = choice_type {
                                    let name_idx_raw = response.choice_indices.first().copied().unwrap_or(0);
                                    let instance_idx = response.choice_indices.get(1).copied().unwrap_or(0);
                                    log::debug!(
                                        "Coordinator P1: LibrarySearchByName name_idx_raw={}, instance_idx={}, name_counts={:?}, library_search_cards={:?}",
                                        name_idx_raw, instance_idx, name_counts, library_search_cards
                                    );
                                    if name_idx_raw > 0 {
                                        let name_idx = name_idx_raw - 1;
                                        // Calculate flat index: sum of counts for names before name_idx, plus instance_idx
                                        let base_offset: usize = name_counts.iter().take(name_idx).sum();
                                        let flat_idx = base_offset + instance_idx;
                                        let result = library_search_cards.as_ref()
                                            .and_then(|cards| cards.get(flat_idx).copied());
                                        log::debug!(
                                            "Coordinator P1: name_idx={}, base_offset={}, flat_idx={}, result={:?}",
                                            name_idx, base_offset, flat_idx, result
                                        );
                                        result
                                    } else {
                                        None // Declined to find
                                    }
                                } else {
                                    None
                                };

                                // Send ChoiceAccepted to P1
                                let _ = p1_to_handler_tx.send(GameToHandler::ChoiceAccepted {
                                    choice_seq,
                                    action_count,
                                    timestamp_ms: now_ms(),
                                    library_search_result,
                                }).await;

                                // Send OpponentMadeChoice to P2
                                let opponent_info = OpponentChoiceInfo {
                                    choice_seq,
                                    player: PlayerId::new(0),
                                    choice_type,
                                    choice_indices: response.choice_indices.clone(),
                                    description: format!("P1 choice #{}", choice_seq),
                                    action_count,
                                    spell_ability,
                                    library_search_result,
                                    target_card_ids: response.target_card_ids,
                                };
                                let _ = p2_to_handler_tx.send(GameToHandler::OpponentMadeChoice(opponent_info)).await;
                            }
                            Some(HandlerToGame::ClientDisconnected) => {
                                log::info!("Coordinator: P1 disconnected");
                                let _ = p2_to_handler_tx.send(GameToHandler::FatalError("Opponent disconnected".to_string())).await;
                                return Err(anyhow!("P1 disconnected"));
                            }
                            Some(HandlerToGame::ClientError(msg)) => {
                                log::error!("Coordinator: P1 client error: {}", msg);
                                let _ = p2_to_handler_tx.send(GameToHandler::FatalError(format!("Opponent error: {}", msg))).await;
                                return Err(anyhow!("P1 client error: {}", msg));
                            }
                            None => {
                                log::error!("Coordinator: P1 handler channel closed");
                                return Err(anyhow!("P1 handler channel closed unexpectedly"));
                            }
                        }
                    }
                    None => {
                        // Bridge channel closed - game loop has ended
                        // Don't break immediately, wait for game_end_rx which will arrive shortly
                        log::debug!("Coordinator: P1 bridge closed, waiting for game_end");
                    }
                }
            }

            // P2 NetworkController sent a ChoiceRequest
            request = p2_bridge_rx.recv() => {
                match request {
                    Some(choice_request) => {
                        log::debug!(
                            "Coordinator: P2 ChoiceRequest seq={} action_count={}",
                            choice_request.choice_seq, choice_request.action_count
                        );

                        // Store request info for later
                        let choice_seq = choice_request.choice_seq;
                        let choice_type = choice_request.choice_type.clone();
                        let action_count = choice_request.action_count;
                        let abilities = choice_request.abilities.clone();
                        let library_search_cards = choice_request.library_search_cards.clone();
                        // For network debug: capture server's state hash and debug info
                        let server_state_hash = choice_request.state_hash;
                        let server_debug_info = choice_request.debug_info.clone();

                        // mtg-420: Broadcast pending LibraryReorders to BOTH clients
                        // before forwarding the ChoiceRequest. See P1 branch above.
                        if !choice_request.library_reorders.is_empty() {
                            log::debug!(
                                "Coordinator: Broadcasting {} library reorder(s) before P2 ChoiceRequest",
                                choice_request.library_reorders.len()
                            );
                            for (player, new_order, action_count) in &choice_request.library_reorders {
                                let msg_p1 = GameToHandler::LibraryReordered {
                                    player: *player,
                                    new_order: new_order.clone(),
                                    action_count: *action_count,
                                };
                                let msg_p2 = GameToHandler::LibraryReordered {
                                    player: *player,
                                    new_order: new_order.clone(),
                                    action_count: *action_count,
                                };
                                if p1_to_handler_tx.send(msg_p1).await.is_err() {
                                    log::error!("Coordinator: Failed to send LibraryReordered to P1");
                                    return Err(anyhow!("P1 handler channel closed"));
                                }
                                if p2_to_handler_tx.send(msg_p2).await.is_err() {
                                    log::error!("Coordinator: Failed to send LibraryReordered to P2");
                                    return Err(anyhow!("P2 handler channel closed"));
                                }
                            }
                        }

                        // Forward to P2 handler
                        if p2_to_handler_tx.send(GameToHandler::ChoiceRequest(Box::new(choice_request))).await.is_err() {
                            log::error!("Coordinator: Failed to send ChoiceRequest to P2 handler");
                            return Err(anyhow!("P2 handler channel closed"));
                        }

                        // Wait for P2's response
                        match p2_handler_rx.recv().await {
                            Some(HandlerToGame::ChoiceResponse { response, client_action_count, client_state_hash, client_debug_info }) => {
                                log::debug!(
                                    "Coordinator: P2 ChoiceResponse seq={} indices={:?}",
                                    response.choice_seq, response.choice_indices
                                );

                                // Validate action_count
                                if client_action_count != action_count {
                                    let error_msg = format!(
                                        "FATAL: P2 action_count mismatch! client={} expected={}",
                                        client_action_count, action_count
                                    );
                                    log::error!("Coordinator: {}", error_msg);
                                    // Send fatal error to both handlers
                                    let _ = p1_to_handler_tx.send(GameToHandler::FatalError(error_msg.clone())).await;
                                    let _ = p2_to_handler_tx.send(GameToHandler::FatalError(error_msg.clone())).await;
                                    return Err(anyhow!("{}", error_msg));
                                }

                                // Validate state hash in network debug mode
                                if network_debug {
                                    if let Some(client_hash) = client_state_hash {
                                        if client_hash != server_state_hash {
                                            log_state_hash_mismatch(
                                                "P2",
                                                choice_seq,
                                                action_count,
                                                server_state_hash,
                                                client_hash,
                                                &server_debug_info,
                                                &client_debug_info,
                                            );
                                            // Per NETWORK_ARCHITECTURE.md: desync is ALWAYS fatal
                                            let error_msg = format!(
                                                "FATAL: P2 state hash mismatch! server={:016x} client={:016x} at choice_seq={} action_count={}",
                                                server_state_hash, client_hash, choice_seq, action_count
                                            );
                                            log::error!("Coordinator: {}", error_msg);
                                            let _ = p1_to_handler_tx.send(GameToHandler::FatalError(error_msg.clone())).await;
                                            let _ = p2_to_handler_tx.send(GameToHandler::FatalError(error_msg.clone())).await;
                                            return Err(anyhow!("{}", error_msg));
                                        }
                                    }
                                }

                                // Send response to NetworkController
                                if p2_response_tx.send(response.clone()).is_err() {
                                    log::error!("Coordinator: Failed to send response to P2 NetworkController");
                                    return Err(anyhow!("P2 NetworkController channel closed"));
                                }

                                // Extract spell_ability for Priority choices
                                let spell_ability = if matches!(choice_type, ChoiceType::Priority { .. }) {
                                    let idx = response.choice_indices.first().copied().unwrap_or(0);
                                    abilities.as_ref()
                                        .and_then(|abs| abs.get(idx).cloned())
                                        .flatten()
                                } else {
                                    None
                                };

                                // Extract library_search_result for LibrarySearchByName choices
                                // Client sends [name_idx+1, instance_idx] where name_idx+1=0 means "Decline"
                                // We decode using name_counts to find the flat index in library_search_cards
                                let library_search_result = if let ChoiceType::LibrarySearchByName { ref name_counts, .. } = choice_type {
                                    let name_idx_raw = response.choice_indices.first().copied().unwrap_or(0);
                                    let instance_idx = response.choice_indices.get(1).copied().unwrap_or(0);
                                    log::debug!(
                                        "Coordinator P2: LibrarySearchByName name_idx_raw={}, instance_idx={}, name_counts={:?}",
                                        name_idx_raw, instance_idx, name_counts
                                    );
                                    if name_idx_raw > 0 {
                                        let name_idx = name_idx_raw - 1;
                                        // Calculate flat index: sum of counts for names before name_idx, plus instance_idx
                                        let base_offset: usize = name_counts.iter().take(name_idx).sum();
                                        let flat_idx = base_offset + instance_idx;
                                        let result = library_search_cards.as_ref()
                                            .and_then(|cards| cards.get(flat_idx).copied());
                                        log::debug!(
                                            "Coordinator P2: name_idx={}, base_offset={}, flat_idx={}, result={:?}",
                                            name_idx, base_offset, flat_idx, result
                                        );
                                        result
                                    } else {
                                        None // Declined to find
                                    }
                                } else {
                                    None
                                };

                                // Send ChoiceAccepted to P2
                                let _ = p2_to_handler_tx.send(GameToHandler::ChoiceAccepted {
                                    choice_seq,
                                    action_count,
                                    timestamp_ms: now_ms(),
                                    library_search_result,
                                }).await;

                                // Send OpponentMadeChoice to P1
                                let opponent_info = OpponentChoiceInfo {
                                    choice_seq,
                                    player: PlayerId::new(1),
                                    choice_type,
                                    choice_indices: response.choice_indices.clone(),
                                    description: format!("P2 choice #{}", choice_seq),
                                    action_count,
                                    spell_ability,
                                    library_search_result,
                                    target_card_ids: response.target_card_ids,
                                };
                                let _ = p1_to_handler_tx.send(GameToHandler::OpponentMadeChoice(opponent_info)).await;
                            }
                            Some(HandlerToGame::ClientDisconnected) => {
                                log::info!("Coordinator: P2 disconnected");
                                let _ = p1_to_handler_tx.send(GameToHandler::FatalError("Opponent disconnected".to_string())).await;
                                return Err(anyhow!("P2 disconnected"));
                            }
                            Some(HandlerToGame::ClientError(msg)) => {
                                log::error!("Coordinator: P2 client error: {}", msg);
                                let _ = p1_to_handler_tx.send(GameToHandler::FatalError(format!("Opponent error: {}", msg))).await;
                                return Err(anyhow!("P2 client error: {}", msg));
                            }
                            None => {
                                log::error!("Coordinator: P2 handler channel closed");
                                return Err(anyhow!("P2 handler channel closed unexpectedly"));
                            }
                        }
                    }
                    None => {
                        // Bridge channel closed - game loop has ended
                        // Don't break immediately, wait for game_end_rx which will arrive shortly
                        log::debug!("Coordinator: P2 bridge closed, waiting for game_end");
                    }
                }
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// PLAYER WEBSOCKET HANDLER (mtg-228)
// ═══════════════════════════════════════════════════════════════════════════
//
// New single-channel architecture:
// - All game state messages come through conn.game_rx (GameToHandler)
// - All responses go through conn.game_tx (HandlerToGame)
// - WebSocket is just for client I/O
// - No direct handler-to-handler communication

/// Handle WebSocket messages for a player using single-channel architecture.
///
/// The handler loop is simple:
/// 1. Wait for game_rx message (from coordinator) OR websocket message (from client)
/// 2. Handle appropriately:
///    - ChoiceRequest: forward to client, wait for response
///    - OpponentMadeChoice: forward to client
///    - ChoiceAccepted: forward to client
///    - GameEnded/FatalError: forward and exit
///    - Client messages: process or queue
async fn handle_player_websocket(
    mut conn: PlayerConnection,
    mut ws_rx: futures_util::stream::SplitStream<WebSocketStream<TcpStream>>,
    game: Arc<Mutex<GameState>>,
    server_config: ServerConfig,
) -> Result<()> {
    log::debug!("Handler P{}: Started", conn.player_id);

    // Track if we're currently waiting for a choice from the client
    let mut waiting_for_choice: Option<ChoiceRequest> = None;

    // ── Minimal lazy protocol (mtg-o99ow), Phase 1 dual-emit accumulators ──
    // Facts that arrive eagerly from the coordinator (opponent choices via
    // `OpponentMadeChoice`, library reorders via `LibraryReordered`) since this
    // client's last `ChoiceRequest`. They are drained into the next
    // `ChoiceRequest`'s `buffer` (alongside the per-recipient reveals already
    // carried on the internal `ChoiceRequest`). We accumulate the eager
    // messages rather than re-deriving from the undo log because the coordinator
    // is the only place the choice payload (`choice_seq` / `choice_indices` /
    // `description`) and the new library order exist (BLOCKER 1 / BLOCKER 2).
    // The eager messages are STILL sent (dual-emit); a buffer-aware client
    // ignores those copies and consumes the buffer.
    let mut pending_choice_facts: Vec<OpponentChoiceInfo> = Vec::new();
    let mut pending_reorder_facts: Vec<(PlayerId, Vec<CardId>, u64)> = Vec::new();

    loop {
        tokio::select! {
            // Messages from coordinator (game state)
            game_msg = conn.game_rx.recv() => {
                match game_msg {
                    Some(GameToHandler::ChoiceRequest(choice_request)) => {
                        log::debug!(
                            "Handler P{}: Received ChoiceRequest seq={} action_count={}",
                            conn.player_id, choice_request.choice_seq, choice_request.action_count
                        );

                        // Send CardRevealed messages for reveals in this request
                        if !choice_request.reveals.is_empty() {
                            let game_guard = game.lock().await;
                            for reveal_info in &choice_request.reveals {
                                if let Some(card_reveal) = build_card_reveal(&game_guard, reveal_info) {
                                    let reason = zone_to_reveal_reason(reveal_info.to_zone);
                                    conn.send(&ServerMessage::CardRevealed {
                                        owner: reveal_info.owner,
                                        card: card_reveal,
                                        reason,
                                        // mtg-o99ow: stamp with the reveal's OWN
                                        // game action_count (the undo-log position
                                        // of its RevealCard action), not the
                                        // bundling choice's action_count (prior
                                        // mtg-610 effective-ac behaviour). The
                                        // reveal occurred strictly before this
                                        // choice, so the shadow applies the
                                        // monotone info at the draw rather than the
                                        // later choice, and rewind retains it at
                                        // its true position.
                                        action_count: Some(reveal_info.action_count),
                                    }).await?;
                                }
                            }
                        }

                        // For LibrarySearchByName choices, reveal all searchable library cards
                        // BEFORE sending ChoiceRequest so client can filter them (mtg-253 fix).
                        //
                        // mtg-o99ow L2d: emit ONE `SearchCandidates` message carrying the full
                        // `Vec<CardReveal>` at the single search-choice ac, NOT N separate
                        // `CardRevealed` messages all stamped at that one ac. Under the
                        // game-`action_count`-keyed shadow log (L3) the N-at-one-ac form would
                        // collide on `ActionLog::push` (strict-monotonicity). One atomic
                        // multi-delta = one log entry = one ac is the only form consistent with
                        // "logs aligned modulo reveal-name visibility": the searcher sees real
                        // names here; the opponent's view stays masked via the dummy `Searched`
                        // reveal at the resolution ac (below).
                        if let Some(ref library_cards) = choice_request.library_search_cards {
                            let game_guard = game.lock().await;
                            log::debug!(
                                "Handler P{}: Sending SearchCandidates ({} cards) for library search (mtg-253/mtg-o99ow)",
                                conn.player_id, library_cards.len()
                            );
                            let cards: Vec<CardReveal> = library_cards
                                .iter()
                                .filter_map(|&card_id| {
                                    game_guard.cards.try_get(card_id).map(|card| CardReveal {
                                        card_id,
                                        name: card.name.to_string(),
                                        card_def: game_guard.card_definitions.get(&card.name).cloned(),
                                    })
                                })
                                .collect();
                            drop(game_guard);
                            if !cards.is_empty() {
                                conn.send(&ServerMessage::SearchCandidates {
                                    searcher: conn.player_id, // Player searching their own library
                                    cards,
                                    action_count: choice_request.action_count,
                                }).await?;
                            }
                        }

                        // Minimal lazy protocol (mtg-o99ow) Phase-1 dual-emit:
                        // assemble the single catch-up buffer carried by this
                        // ChoiceRequest. The eager messages above/below are still
                        // sent; a buffer-aware client consumes the buffer and
                        // ignores the eager copies. Built under the game lock
                        // (reveal/candidate identities need card lookups).
                        let buffer = {
                            let game_guard = game.lock().await;
                            assemble_choice_buffer(
                                &game_guard,
                                &choice_request,
                                conn.player_id,
                                &pending_reorder_facts,
                                &pending_choice_facts,
                            )
                        };

                        // Check if client already sent a choice (pending_choice)
                        if let Some(pending) = conn.pending_choice.take() {
                            log::debug!(
                                "Handler P{}: Processing pending choice {} (arrived before ChoiceRequest)",
                                conn.player_id, pending.choice_seq
                            );

                            // Send response to coordinator, including spell_ability for robust matching
                            let response = ChoiceResponse {
                                choice_seq: pending.choice_seq,
                                choice_indices: pending.choice_indices,
                                spell_ability: pending.spell_ability,
                                target_card_ids: pending.target_card_ids,
                            };
                            conn.game_tx.send(HandlerToGame::ChoiceResponse {
                                response,
                                client_action_count: pending.action_count,
                                client_state_hash: pending.client_state_hash,
                                client_debug_info: pending.client_debug_info,
                            }).await?;
                            // Buffer not delivered on this path (no ChoiceRequest
                            // sent); keep the accumulated facts for the next one.
                            let _ = buffer;
                        } else {
                            // Normal case: send ChoiceRequest to client and wait
                            conn.send(&ServerMessage::ChoiceRequest {
                                choice_seq: choice_request.choice_seq,
                                for_player: conn.player_id,
                                choice_type: choice_request.choice_type.clone(),
                                options: choice_request.options.clone(),
                                state_hash: choice_request.state_hash,
                                action_count: choice_request.action_count,
                                timestamp_ms: now_ms(),
                                context: None,
                                debug_info: choice_request.debug_info.clone(),
                                abilities: choice_request.abilities.clone(),
                                buffer,
                            }).await?;

                            // The accumulated facts are now delivered in the buffer.
                            pending_reorder_facts.clear();
                            pending_choice_facts.clear();

                            // Mark that we're waiting for this choice
                            waiting_for_choice = Some(*choice_request);
                        }
                    }

                    Some(GameToHandler::OpponentMadeChoice(info)) => {
                        log::debug!(
                            "Handler P{}: Accumulating opponent choice seq={} for next ChoiceRequest buffer",
                            conn.player_id, info.choice_seq
                        );

                        // Minimal lazy protocol (mtg-o99ow) TASK 2 — buffer is the
                        // SOLE source of the opponent decision. We ONLY accumulate
                        // it for the next ChoiceRequest's `buffer`
                        // (`assemble_choice_buffer` emits the `BufferedFact::Choice`
                        // AND, for a hidden tutor, the dummy `Searched` reveal; the
                        // opponent's cast-card reveal already rides in that
                        // recipient's `choice_request.reveals` at its own ac). The
                        // eager `OpponentChoice` + bundled `CardRevealed` sends were
                        // DELETED here: they were the dual-stamp source (the same
                        // card re-revealed at the choice ac vs its own ac) and are
                        // fully superseded by the buffer. Both native and WASM
                        // clients are buffer-driven; deleting the eager copy makes
                        // the buffer the sole mid-game opponent-choice source (the
                        // false-positive guard: no eager copy can secretly drive the
                        // shadow). Opening hands + initial library orders still flow
                        // via the game-setup path; ChoiceAccepted is still sent.
                        pending_choice_facts.push(info);
                    }

                    Some(GameToHandler::ChoiceAccepted { choice_seq, action_count, timestamp_ms, library_search_result }) => {
                        log::debug!(
                            "Handler P{}: Forwarding ChoiceAccepted seq={}",
                            conn.player_id, choice_seq
                        );

                        // mtg-o99ow Phase 2: do NOT eagerly re-reveal the found card here.
                        //
                        // The own-library search found card is ALREADY delivered to the
                        // recipient by `collect_reveals_since_last_choice` at its TRUE
                        // resolution `action_count` (the undo-log position of the
                        // RevealCard logged when `move_card` lifts it Library->hand),
                        // via the generic reveals flush above (server.rs ~2933) for the
                        // eager native path and via `assemble_choice_buffer` path (1) for
                        // the buffer path. Re-emitting it HERE stamped it at the
                        // search-CHOICE `action_count` instead — the SAME ac as the
                        // `SearchCandidates` delta — which is a two-distinct-deltas-at-one-ac
                        // protocol desync (WASM strict log panics; native synthetic log
                        // mis-applies -> found card never reaches hand -> off-by-one).
                        //
                        // Re-stamping it at the resolution ac HERE is impossible: at
                        // ChoiceAccepted-forward time the move has not executed yet, so the
                        // RevealCard does not exist in the undo log (the layer-4 re-stamp
                        // impossibility, proven for the opponent path; identical here).
                        // The CardId the searcher needs to move the right instance still
                        // travels in ChoiceAccepted.library_search_result (below); the card
                        // DATA is already known to the searcher via SearchCandidates.
                        let _ = library_search_result;

                        conn.send(&ServerMessage::ChoiceAccepted {
                            choice_seq,
                            action_count,
                            timestamp_ms,
                            library_search_result,
                        }).await?;
                    }

                    Some(GameToHandler::LibraryReordered { player, new_order, action_count }) => {
                        log::debug!(
                            "Handler P{}: Forwarding LibraryReordered for {:?} ({} cards) ac={}",
                            conn.player_id, player, new_order.len(), action_count
                        );

                        // Minimal lazy protocol (mtg-o99ow) Phase-1 dual-emit:
                        // accumulate this reorder for the next ChoiceRequest
                        // buffer (BLOCKER 2: the new order exists here, not in a
                        // backward undo-log scan). The eager LibraryReordered
                        // send below still happens.
                        pending_reorder_facts.push((player, new_order.clone(), action_count));

                        conn.send(&ServerMessage::LibraryReordered {
                            player,
                            new_order,
                            // mtg-o99ow L2: the reorder's own undo-log position
                            // (ReorderLibrary for scry/surveil), threaded from
                            // pending_library_reorders. Client keys on it in L3.
                            action_count,
                        }).await?;
                    }

                    Some(GameToHandler::GameEnded(info)) => {
                        log::info!(
                            "Handler P{}: Sending GameEnded winner={:?}",
                            conn.player_id, info.winner
                        );
                        conn.send(&ServerMessage::GameEnded {
                            winner: info.winner,
                            reason: info.reason,
                            final_state_hash: info.final_hash,
                            action_count: info.action_count,
                        }).await?;
                        break;
                    }

                    Some(GameToHandler::FatalError(msg)) => {
                        log::error!("Handler P{}: Fatal error: {}", conn.player_id, msg);
                        conn.send(&ServerMessage::Error {
                            message: msg,
                            fatal: true,
                        }).await?;
                        break;
                    }

                    None => {
                        // Coordinator channel closed
                        log::debug!("Handler P{}: Coordinator channel closed", conn.player_id);
                        break;
                    }
                }
            }

            // Messages from client (WebSocket)
            ws_msg = ws_rx.next() => {
                match ws_msg {
                    Some(Ok(Message::Text(text))) => {
                        if log::log_enabled!(log::Level::Debug) {
                            let truncated = if text.len() > 500 {
                                format!("{}... ({} bytes)", &text[..500], text.len())
                            } else {
                                text.to_string()
                            };
                            log::debug!("[P{}->SERVER] {}", conn.player_id, truncated);
                        }

                        match serde_json::from_str::<ClientMessage>(&text) {
                            Ok(ClientMessage::SubmitChoice {
                                choice_seq,
                                choice_indices,
                                action_count,
                                client_state_hash,
                                debug_info,
                                spell_ability,
                                target_card_ids,
                                ..
                            }) => {
                                if waiting_for_choice.take().is_some() {
                                    // Normal case: we sent ChoiceRequest and client is responding
                                    log::debug!(
                                        "Handler P{}: Received choice seq={} action_count={} spell_ability={:?} targets={:?}",
                                        conn.player_id, choice_seq, action_count, spell_ability.as_ref().map(|a| format!("{:?}", a)), target_card_ids
                                    );

                                    // Send response to coordinator, including spell_ability for robust matching
                                    let response = ChoiceResponse { choice_seq, choice_indices, spell_ability, target_card_ids };
                                    conn.game_tx.send(HandlerToGame::ChoiceResponse {
                                        response,
                                        client_action_count: action_count,
                                        client_state_hash,
                                        client_debug_info: debug_info,
                                    }).await?;
                                } else {
                                    // Client is ahead: queue for later
                                    log::debug!(
                                        "Handler P{}: Queueing early choice seq={} (waiting for ChoiceRequest)",
                                        conn.player_id, choice_seq
                                    );
                                    conn.pending_choice = Some(PendingChoice {
                                        choice_seq,
                                        choice_indices,
                                        action_count,
                                        client_state_hash,
                                        client_debug_info: debug_info,
                                        spell_ability,
                                        target_card_ids,
                                    });
                                }
                            }

                            Ok(ClientMessage::Ping { timestamp_ms }) => {
                                conn.send(&ServerMessage::Pong { timestamp_ms }).await?;
                            }

                            Ok(ClientMessage::BugReport {
                                description,
                                game_logs,
                                console_logs,
                                trusted_password,
                            }) => {
                                // Two-phase bug report (mtg-5ejgo): see the lobby
                                // call site above for why the phase-1 disk
                                // confirmation and phase-2 GitHub result are sent
                                // as two separate messages.
                                let report = BugReportRequest {
                                    description,
                                    game_logs,
                                    console_logs,
                                    trusted_password,
                                };
                                let reporter = Some(conn.player_id);
                                let stored = store_bug_report(&server_config, &report, reporter).await;
                                conn.send(&bug_report_stored_message(&stored, reporter)).await?;
                                if let Ok(stored_report) = &stored {
                                    let issue_msg = file_bug_report_issue(&report, stored_report, reporter).await;
                                    conn.send(&issue_msg).await?;
                                }
                            }

                            Ok(ClientMessage::Disconnect) => {
                                log::info!("Handler P{}: Client disconnected gracefully", conn.player_id);
                                conn.game_tx.send(HandlerToGame::ClientDisconnected).await?;
                                break;
                            }

                            Ok(ClientMessage::Authenticate { .. }
                            | ClientMessage::CreateGame { .. }
                            | ClientMessage::JoinGame { .. }
                            | ClientMessage::ListGames { .. }
                            | ClientMessage::Register { .. }
                            | ClientMessage::SetDeck { .. }
                            | ClientMessage::SetReady { .. }
                            | ClientMessage::Reconnect { .. }) => {
                                // Lobby/auth/waiting-room messages are not legal once a
                                // game has started for this connection. We surface a
                                // non-fatal Error so test clients can recover.
                                conn.send(&ServerMessage::Error {
                                    message: "Already authenticated / in a game".to_string(),
                                    fatal: false,
                                }).await?;
                            }

                            Err(e) => {
                                log::error!("Handler P{}: Failed to parse: {}", conn.player_id, e);
                                conn.send(&ServerMessage::Error {
                                    message: format!("Invalid message: {}", e),
                                    fatal: false,
                                }).await?;
                            }
                        }
                    }

                    Some(Ok(Message::Close(_))) => {
                        log::info!("Handler P{}: WebSocket closed", conn.player_id);
                        let _ = conn.game_tx.send(HandlerToGame::ClientDisconnected).await;
                        break;
                    }

                    Some(Ok(_)) => {
                        // Ignore binary/ping/pong
                    }

                    Some(Err(e)) => {
                        log::error!("Handler P{}: WebSocket error: {}", conn.player_id, e);
                        let _ = conn.game_tx.send(HandlerToGame::ClientError(e.to_string())).await;
                        break;
                    }

                    None => {
                        log::debug!("Handler P{}: WebSocket stream ended", conn.player_id);
                        let _ = conn.game_tx.send(HandlerToGame::ClientDisconnected).await;
                        break;
                    }
                }
            }
        }
    }

    log::debug!("Handler P{}: Exiting", conn.player_id);
    Ok(())
}

/// Run the game loop with NetworkControllers
fn run_game_loop(
    game: Arc<Mutex<GameState>>,
    mut p1_controller: NetworkController,
    mut p2_controller: NetworkController,
    tag_gamelogs: bool,
    verbosity: crate::game::VerbosityLevel,
    no_color_logs: bool,
) -> Result<GameResult> {
    // Take ownership of game for the game loop
    let mut game = {
        // We need to get the game out of the mutex for the game loop
        // This is safe because the WebSocket handlers only read game state
        let guard = game.blocking_lock();
        guard.clone()
    };

    // Configure the game logger with server settings
    game.logger.set_verbosity(verbosity);
    game.logger.set_tag_gamelogs(tag_gamelogs);
    // Disable colors if --no-color-logs flag or NO_COLOR env var is set
    let color_enabled = !no_color_logs && std::env::var("NO_COLOR").is_err();
    game.logger.set_color_enabled(color_enabled);

    log::debug!(
        "Server GameLoop: undo_log.len() = {} (should be 0 for synchronized mode)",
        game.undo_log.len()
    );

    // Create game loop with skip_opening_hands() to match client behavior.
    // Both server and client will draw opening hands during GameLoop::setup_game(),
    // ensuring identical undo_log entries and synchronized action_counts.
    //
    // All reveals are sent synchronously via ChoiceRequest messages,
    // ensuring strict ordering and preventing desync issues.
    let mut game_loop = GameLoop::new(&mut game).skip_opening_hands();

    // Run until game ends
    let result = game_loop.run_game(&mut p1_controller, &mut p2_controller);

    match result {
        Ok(game_result) => Ok(game_result),
        Err(e) => Err(anyhow!("Game loop error: {}", e)),
    }
}

/// Peek at opening hand for a player and return CardReveals WITHOUT drawing
///
/// This looks at the top 7 cards of the library without actually drawing them.
/// The drawing will happen later when the GameLoop runs with skip_opening_hands().
/// This ensures both server and client GameLoops start with identical empty undo_logs.
fn peek_opening_hand(game: &GameState, player_id: PlayerId) -> Result<Vec<CardReveal>> {
    let mut hand = Vec::new();

    // Get the library for this player
    let zones = game
        .get_player_zones(player_id)
        .ok_or_else(|| anyhow!("Player {:?} not found", player_id))?;

    // Peek at top 7 cards (library.cards stores bottom-to-top, so we take from the end)
    let lib_cards = &zones.library.cards;
    let start_idx = lib_cards.len().saturating_sub(7);

    for &card_id in lib_cards[start_idx..].iter().rev() {
        // Get card info for reveal
        if let Ok(card) = game.cards.get(card_id) {
            let card_def = game.card_definitions.get(&card.name).cloned();
            hand.push(CardReveal {
                card_id,
                name: card.name.to_string(),
                card_def,
            });
        }
    }

    Ok(hand)
}

/// Compute network-safe state hash
///
/// Delegates to `state_hash::compute_network_state_hash` which hashes all PUBLIC
/// game state (battlefield, stack, graveyard, exile, life totals, turn/step,
/// hand/library SIZES) while excluding hidden information (hand contents,
/// library order, RNG state). This produces identical hashes on server and client.
fn compute_network_hash(game: &GameState) -> u64 {
    compute_network_state_hash(game)
}

// ═══════════════════════════════════════════════════════════════════════════
// REVEAL HELPERS
// ═══════════════════════════════════════════════════════════════════════════

/// Build a CardReveal from a CardRevealInfo by looking up card details in game state
fn build_card_reveal(game: &GameState, info: &CardRevealInfo) -> Option<CardReveal> {
    let card = game.cards.try_get(info.card_id)?;

    // Look up CardDefinition from game.card_definitions
    let card_def = game.card_definitions.get(&card.name).cloned();

    Some(CardReveal {
        card_id: info.card_id,
        name: card.name.to_string(),
        card_def,
    })
}

/// Assemble the minimal-lazy-protocol `ChoiceRequest` buffer (mtg-o99ow).
///
/// Collects every reveal-class + opponent-choice fact the recipient needs to
/// replay its shadow forward to this choice point into ONE ascending-`ac`
/// vector, each fact at its TRUE game `action_count`. This is the single
/// catch-up payload that supersedes the eager `CardRevealed` /
/// `LibraryReordered` / `SearchCandidates` / `OpponentChoice` message zoo.
///
/// Sources (the critique's BLOCKER 1 / BLOCKER 2 corrections):
/// - **Reveals** come from `choice_request.reveals` (collected per-recipient by
///   `collect_reveals_since_last_choice`, each at its own undo position). This
///   INCLUDES an opponent's cast-card reveal at its own ac (K+1), so we do NOT
///   re-emit it at the choice ac (K) — that re-emission was the dual-stamp.
/// - **Library reorders** come from the coordinator-broadcast `reorders` (the
///   new order only exists live, never in a backward undo-log scan — BLOCKER 2).
/// - **Search candidates** come from `choice_request.library_search_cards`.
/// - **Choices** come from the coordinator-retained `choices` (the `choice_seq`
///   / `choice_indices` / `description` only exist there — BLOCKER 1). For an
///   opponent's hidden library search we also emit a dummy `Searched` reveal at
///   the resolution ac so the recipient's `searched_card_for` selection works.
///
/// `searcher` is the recipient (own-library search). The result is stably sorted
/// by `ac` so same-`ac` `Choice` facts keep their `choice_seq` order.
fn assemble_choice_buffer(
    game: &GameState,
    choice_request: &ChoiceRequest,
    searcher: PlayerId,
    reorders: &[(PlayerId, Vec<CardId>, u64)],
    choices: &[OpponentChoiceInfo],
) -> Vec<(u64, BufferedFact)> {
    let mut buffer: Vec<(u64, BufferedFact)> = Vec::new();

    // (1) Per-recipient reveals, each at its own undo-log position.
    for info in &choice_request.reveals {
        if let Some(card) = build_card_reveal(game, info) {
            let reason = zone_to_reveal_reason(info.to_zone);
            buffer.push((
                info.action_count,
                BufferedFact::Reveal {
                    owner: info.owner,
                    card,
                    reason,
                },
            ));
        }
    }

    // (2) Library-search candidates: one atomic-multi delta at the search ac.
    if let Some(ref library_cards) = choice_request.library_search_cards {
        let cards: Vec<CardReveal> = library_cards
            .iter()
            .filter_map(|&card_id| {
                game.cards.try_get(card_id).map(|card| CardReveal {
                    card_id,
                    name: card.name.to_string(),
                    card_def: game.card_definitions.get(&card.name).cloned(),
                })
            })
            .collect();
        if !cards.is_empty() {
            buffer.push((
                choice_request.action_count,
                BufferedFact::SearchCandidates { searcher, cards },
            ));
        }
    }

    // (3) Library reorders (new order from the live coordinator broadcast).
    for (player, new_order, ac) in reorders {
        buffer.push((
            *ac,
            BufferedFact::LibraryReorder {
                player: *player,
                new_order: new_order.clone(),
            },
        ));
    }

    // (4) Opponent choices (+ dummy Searched reveal for hidden tutor results).
    for info in choices {
        buffer.push((
            info.action_count,
            BufferedFact::Choice {
                choice_seq: info.choice_seq,
                choice_type: info.choice_type.clone(),
                choice_indices: info.choice_indices.clone(),
                description: info.description.clone(),
                spell_ability: info.spell_ability.clone(),
                library_search_result: info.library_search_result,
                target_card_ids: info.target_card_ids.clone(),
            },
        ));
        if let Some(card_id) = info.library_search_result {
            buffer.push((
                info.action_count,
                BufferedFact::Reveal {
                    owner: info.player,
                    card: CardReveal {
                        card_id,
                        name: String::new(), // hidden — opponent's own library
                        card_def: None,
                    },
                    reason: RevealReason::Searched,
                },
            ));
        }
    }

    // Stable sort: same-ac Choice facts keep their choice_seq emission order.
    buffer.sort_by_key(|(ac, _)| *ac);

    buffer
}

/// Convert a zone to the appropriate RevealReason
fn zone_to_reveal_reason(zone: Zone) -> RevealReason {
    match zone {
        Zone::Hand => RevealReason::Draw,
        Zone::Battlefield | Zone::Stack => RevealReason::Played,
        Zone::Graveyard => RevealReason::Effect, // Mill or other effects
        Zone::Exile => RevealReason::Effect,
        Zone::Library => RevealReason::Searched, // Returned to library (unusual)
        Zone::Command => RevealReason::Effect,
    }
}

/// Determine whether a submitted bug report should be flagged `trusted`.
///
/// **Infallible by design** (returns `bool`, not `Result<bool>`): a bug report
/// MUST always be stored regardless of the password outcome. The password only
/// decides the `trusted` metadata field:
///
/// | `expected_password` | `provided_password` | result |
/// |---|---|---|
/// | empty (not configured) | any / None | `false` (untrusted — no configured password to verify against) |
/// | non-empty | `None` | `false` (no password supplied) |
/// | non-empty | `Some(pw)` where `pw == expected` | `true` |
/// | non-empty | `Some(pw)` where `pw != expected` | `false` (wrong password → untrusted, NOT an error) |
///
/// The previous `Result<bool>` version would `Err` on a wrong password, which
/// caused the caller to propagate the error and REJECT the upload — violating
/// the invariant that bug reports are always stored. (mtg-obrx2)
fn validate_trusted_bug_report_password(expected_password: &str, provided_password: Option<&str>) -> bool {
    if expected_password.is_empty() {
        // No password configured — no way to grant trust; always untrusted.
        return false;
    }
    match provided_password {
        Some(pw) => pw == expected_password,
        None => false,
    }
}

async fn create_bug_report_dir(root: &std::path::Path, timestamp_ms: u64) -> Result<PathBuf> {
    fs::create_dir_all(root).await?;

    let mut attempt = 0usize;
    loop {
        let dir_name = if attempt == 0 {
            timestamp_ms.to_string()
        } else {
            format!("{timestamp_ms}-{attempt}")
        };
        let report_dir = root.join(dir_name);

        match fs::create_dir(&report_dir).await {
            Ok(()) => return Ok(report_dir),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                attempt += 1;
            }
            Err(error) => return Err(error.into()),
        }
    }
}

fn bug_report_repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("mtg-engine crate should live under repo root")
        .to_path_buf()
}

/// Resolve the GitHub repository (`OWNER/REPO` or full URL) that bug-report
/// issues and labels are filed against.
///
/// This is read from the `MTG_GH_REPO` environment variable (set via deploy
/// config) so a future org/repo move is a single configuration change rather
/// than a code edit. When unset, it falls back to the compiled-in
/// [`BUG_REPORT_GITHUB_REPO`] default so filing is ALWAYS scoped to an
/// explicit repo — never dependent on `gh`'s cwd / git-remote auto-detection
/// (mtg-587).
fn bug_report_gh_repo() -> String {
    std::env::var("MTG_GH_REPO")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| BUG_REPORT_GITHUB_REPO.to_string())
}

/// Append a `-R <OWNER/REPO>` selector to `gh` subcommand args, scoping the
/// command to the configured bug-report repository (`MTG_GH_REPO` override, or
/// the [`BUG_REPORT_GITHUB_REPO`] default). Keeps repo selection in ONE place
/// so call sites never hardcode the repo (mtg-602 config-drive + mtg-587
/// explicit-repo invariant).
fn with_configured_gh_repo(mut gh_args: Vec<String>) -> Vec<String> {
    gh_args.push("-R".to_string());
    gh_args.push(bug_report_gh_repo());
    gh_args
}

/// Build the optional egress-proxy command prefix from a raw value (the
/// `MTG_GH_PROXY` env var). Empty/whitespace → no prefix (invoke the tool
/// directly). Pure helper so it is unit-testable without mutating process env.
fn proxy_prefix_from(value: Option<&str>) -> Vec<String> {
    match value.map(str::trim).filter(|trimmed| !trimmed.is_empty()) {
        Some(proxy) => vec![proxy.to_string()],
        None => Vec::new(),
    }
}

/// The egress-proxy prefix for spawned bug-report CLIs (`gh`, `claude`).
///
/// Read from `MTG_GH_PROXY`. UNSET/empty — the default, including production VMs
/// that have direct internet egress — means invoke the tool directly. Set it to
/// a wrapper path (e.g. `/usr/bin/with-proxy`) ONLY on networks that require
/// routing outbound calls through a proxy (e.g. Meta devservers).
///
/// The previous code HARDCODED `/usr/bin/with-proxy`, which is absent on the
/// deploy VM, so every `gh` invocation failed with ENOENT before `gh` even ran
/// (mtg-zvlpk). Making the prefix config-driven + default-direct fixes filing on
/// the VM (where `gh` is installed, authed, and has direct egress).
fn command_proxy_prefix() -> Vec<String> {
    proxy_prefix_from(std::env::var("MTG_GH_PROXY").ok().as_deref())
}

/// Resolve `path` to an absolute path against `base` (the server's working
/// directory). An already-absolute `path` is returned unchanged — `Path::join`
/// replaces the base when its argument is absolute. Used so the `gh` file-path
/// args (`--body-file`, gist files) are absolute and therefore independent of
/// `gh`'s own working directory (mtg-zvlpk).
fn absolutize_under(path: &Path, base: &Path) -> PathBuf {
    base.join(path)
}

fn bug_report_issue_title(report: &BugReportRequest) -> String {
    let summary = report
        .description
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("Bug report")
        .chars()
        .take(72)
        .collect::<String>();
    format!("Bug report: {}", summary)
}

fn build_claude_autofix_prompt(
    report: &BugReportRequest,
    report_dir: &Path,
    issue_url: &str,
    reporter_player_id: Option<PlayerId>,
) -> String {
    format!(
        "Fix this bug report and file a PR.\n\n\
GitHub issue: {issue_url}\n\
Stored report directory: {report_dir}\n\
Reporter player id: {reporter}\n\n\
Requirements:\n\
- Reproduce and fix the bug described below.\n\
- File a PR for the fix.\n\
- Link the PR to the issue by mentioning {issue_url} in the PR body.\n\
- After fixing, ensure the PR is cross-referenced back to the issue.\n\n\
Bug description:\n\
```text\n\
{description}\n\
```\n\n\
Game logs:\n\
```text\n\
{game_logs}\n\
```\n\n\
Console logs:\n\
```text\n\
{console_logs}\n\
```",
        issue_url = issue_url,
        report_dir = report_dir.display(),
        reporter = reporter_player_id
            .map(|player_id| player_id.as_u32().to_string())
            .unwrap_or_else(|| "pre_auth".to_string()),
        description = report.description,
        game_logs = report.game_logs,
        console_logs = report.console_logs,
    )
}

fn parse_first_url(text: &str) -> Result<String> {
    text.split_whitespace()
        .find(|token| token.starts_with("http://") || token.starts_with("https://"))
        .map(ToOwned::to_owned)
        .ok_or_else(|| anyhow!("gh output did not contain a URL: {}", text.trim()))
}

fn format_command_for_error(args: &[String]) -> String {
    args.join(" ")
}

fn run_command(args: &[String], cwd: &Path) -> std::io::Result<CommandOutput> {
    let mut command = Command::new(&args[0]);
    command.args(&args[1..]).current_dir(cwd);
    let output = command.output()?;
    Ok(CommandOutput {
        success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

fn run_gh_command_with_runner(
    runner: &dyn Fn(&[String], &Path) -> std::io::Result<CommandOutput>,
    cwd: &Path,
    gh_args: &[String],
) -> Result<String> {
    let mut command_args = command_proxy_prefix();
    command_args.push("/usr/bin/gh".to_string());
    command_args.extend_from_slice(gh_args);
    let output = runner(&command_args, cwd)?;
    if output.success {
        Ok(output.stdout)
    } else {
        Err(anyhow!(
            "command failed: {}\nstdout: {}\nstderr: {}",
            format_command_for_error(&command_args),
            output.stdout.trim(),
            output.stderr.trim()
        ))
    }
}

fn spawn_claude_autofix_process(args: &[String], cwd: &Path) -> std::io::Result<Option<u32>> {
    let mut command = Command::new(&args[0]);
    command.args(&args[1..]).current_dir(cwd);
    let child = command.spawn()?;
    Ok(Some(child.id()))
}

#[allow(clippy::type_complexity)]
fn launch_claude_autofix_with_spawner(
    spawner: &dyn Fn(&[String], &Path) -> std::io::Result<Option<u32>>,
    repo_root: &Path,
    request: &AutoFixLaunchRequest,
) -> Result<Option<u32>> {
    let mut command_args = command_proxy_prefix();
    command_args.extend_from_slice(&[
        "claude".to_string(),
        "--dangerously-skip-permissions".to_string(),
        "-p".to_string(),
        request.prompt.clone(),
    ]);
    let pid = spawner(&command_args, repo_root)?;
    Ok(pid)
}

fn maybe_schedule_claude_autofix(
    report: &BugReportRequest,
    report_dir: &Path,
    reporter_player_id: Option<PlayerId>,
    stored_report: &StoredBugReport,
    issue_url: Option<&str>,
) {
    if !stored_report.trusted {
        log::debug!("Skipping Claude auto-fix launch because bug report was not trusted");
        return;
    }

    let Some(issue_url) = issue_url else {
        log::warn!("Skipping Claude auto-fix launch because no GitHub issue URL was created");
        return;
    };

    let repo_root = bug_report_repo_root();
    let request = AutoFixLaunchRequest {
        issue_url: issue_url.to_string(),
        prompt: build_claude_autofix_prompt(report, report_dir, issue_url, reporter_player_id),
    };

    log::info!(
        "Scheduling Claude auto-fix launch for trusted bug report {} linked to {}",
        report_dir.display(),
        issue_url
    );

    schedule_claude_autofix_with_spawner(Arc::new(spawn_claude_autofix_process), repo_root, request);
}

#[allow(clippy::type_complexity)]
fn schedule_claude_autofix_with_spawner(
    spawner: Arc<dyn Fn(&[String], &Path) -> std::io::Result<Option<u32>> + Send + Sync>,
    repo_root: PathBuf,
    request: AutoFixLaunchRequest,
) {
    tokio::spawn(async move {
        log::info!("Starting Claude auto-fix attempt for {}", request.issue_url);
        match launch_claude_autofix_with_spawner(spawner.as_ref(), &repo_root, &request) {
            Ok(pid) => {
                log::info!("Claude auto-fix launched for {} (pid={:?})", request.issue_url, pid);
            }
            Err(error) => {
                log::error!("Failed to launch Claude auto-fix for {}: {}", request.issue_url, error);
            }
        }
    });
}

/// Preflight `gh auth status` so a missing/expired token surfaces as a clear
/// warning in the issue-filing log path rather than an opaque failure on the
/// first repo-scoped command (mtg-587). Returns `Ok(())` when authenticated,
/// `Err` otherwise; callers treat the error as a non-fatal warning and fall
/// back to local storage.
fn check_gh_auth_with_runner(
    runner: &dyn Fn(&[String], &Path) -> std::io::Result<CommandOutput>,
    cwd: &Path,
) -> Result<()> {
    run_gh_command_with_runner(runner, cwd, &["auth".to_string(), "status".to_string()])?;
    Ok(())
}

fn available_bug_report_labels_with_runner(
    runner: &dyn Fn(&[String], &Path) -> std::io::Result<CommandOutput>,
    cwd: &Path,
) -> Result<HashSet<String>> {
    let stdout = run_gh_command_with_runner(
        runner,
        cwd,
        &with_configured_gh_repo(vec![
            "label".to_string(),
            "list".to_string(),
            "--json".to_string(),
            "name".to_string(),
            "--limit".to_string(),
            "200".to_string(),
        ]),
    )?;
    let labels: Vec<serde_json::Value> = serde_json::from_str(&stdout)?;
    Ok(labels
        .into_iter()
        .filter_map(|label| {
            label
                .get("name")
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned)
        })
        .collect())
}

fn upload_bug_report_logs_with_runner(
    runner: &dyn Fn(&[String], &Path) -> std::io::Result<CommandOutput>,
    cwd: &Path,
    report_dir: &Path,
) -> Result<String> {
    let stdout = run_gh_command_with_runner(
        runner,
        cwd,
        &[
            "gist".to_string(),
            "create".to_string(),
            report_dir.join("game_logs.txt").display().to_string(),
            report_dir.join("console_logs.txt").display().to_string(),
            "-d".to_string(),
            format!("MTG Forge bug report logs {}", report_dir.display()),
        ],
    )?;
    parse_first_url(&stdout)
}

fn build_bug_report_issue_body(
    report: &BugReportRequest,
    report_dir: &Path,
    reporter_player_id: Option<PlayerId>,
    log_artifact_url: Option<&str>,
    log_artifact_warning: Option<&str>,
) -> String {
    let mut body = String::new();
    body.push_str("## User Report\n\n");
    body.push_str(&report.description);
    body.push_str("\n\n## Server Metadata\n\n");
    body.push_str(&format!("- Stored report directory: `{}`\n", report_dir.display()));
    body.push_str(&format!(
        "- Reporter player id: {}\n",
        reporter_player_id
            .map(|player_id| player_id.as_u32().to_string())
            .unwrap_or_else(|| "pre_auth".to_string())
    ));
    body.push_str(&format!(
        "- Trusted password supplied: {}\n",
        if report.trusted_password.is_some() { "yes" } else { "no" }
    ));

    body.push_str("\n## Logs\n\n");
    match log_artifact_url {
        Some(url) => {
            body.push_str(&format!(
                "Uploaded `game_logs.txt` and `console_logs.txt` via `gh gist create`: {url}\n"
            ));
        }
        None => body.push_str(
            "Automated GitHub log artifact upload was not available. The logs remain stored on the server in the report directory above.\n",
        ),
    }

    if let Some(warning) = log_artifact_warning {
        body.push_str("\nUpload warning:\n\n");
        body.push_str("```text\n");
        body.push_str(warning);
        body.push_str("\n```\n");
    }

    body
}

fn create_github_issue_with_runner(
    runner: &dyn Fn(&[String], &Path) -> std::io::Result<CommandOutput>,
    report: &BugReportRequest,
    report_dir: &Path,
    reporter_player_id: Option<PlayerId>,
) -> Result<GitHubIssueOutcome> {
    // `report_dir` (and thus the issue-body + gist file-path ARGS derived from
    // it) may be RELATIVE to the server's working directory — the default
    // `bug_reports_dir` is relative ("bug_reports/<ts>"). `gh` resolves any
    // relative file-path arg (and its own cwd) against ITS OWN working dir, so
    // mixing a relative cwd with relative `--body-file` paths is a footgun:
    //   - old code: cwd = bug_report_repo_root() = COMPILE-TIME crate path,
    //     absent on the deploy VM → Command::spawn ENOENT (mtg-zvlpk);
    //   - first fix: cwd = report_dir (relative) → gh resolved the relative
    //     `--body-file bug_reports/<ts>/…` UNDER report_dir → "no such file".
    // Make the whole class impossible: ABSOLUTIZE report_dir against the server's
    // current working dir up front, then derive every file arg (and the gh cwd)
    // from the absolute path. Absolute args make gh's cwd irrelevant to file
    // resolution, and an absolute existing cwd can never ENOENT.
    let report_dir_abs = std::env::current_dir()
        .map(|cwd| absolutize_under(report_dir, &cwd))
        .unwrap_or_else(|_| report_dir.to_path_buf());
    let report_dir = report_dir_abs.as_path();
    let gh_cwd = report_dir;

    // Preflight: a clear, early signal if the VM's `gh` is not authenticated.
    // Non-fatal — we still attempt to file (and fall back to local storage on
    // failure), but the warning makes auth problems easy to diagnose.
    if let Err(error) = check_gh_auth_with_runner(runner, gh_cwd) {
        log::warn!(
            "gh auth status preflight failed before filing bug-report issue to {}: {}",
            BUG_REPORT_GITHUB_REPO,
            error
        );
    }

    let available_labels = match available_bug_report_labels_with_runner(runner, gh_cwd) {
        Ok(labels) => labels,
        Err(error) => {
            log::warn!("Failed to fetch GitHub labels for bug report issue: {}", error);
            HashSet::new()
        }
    };
    let chosen_labels: Vec<&str> = ["bug", "bug-report", "triage"]
        .into_iter()
        .filter(|label| available_labels.contains(*label))
        .collect();

    let (log_artifact_url, log_artifact_warning) = match upload_bug_report_logs_with_runner(runner, gh_cwd, report_dir)
    {
        Ok(url) => (Some(url), None),
        Err(error) => {
            log::warn!("Failed to upload bug report logs with gh gist create: {}", error);
            (None, Some(error.to_string()))
        }
    };

    let issue_body = build_bug_report_issue_body(
        report,
        report_dir,
        reporter_player_id,
        log_artifact_url.as_deref(),
        log_artifact_warning.as_deref(),
    );
    let issue_body_path = report_dir.join("github_issue_body.md");
    std::fs::write(&issue_body_path, issue_body)?;

    let mut issue_args = vec![
        "issue".to_string(),
        "create".to_string(),
        "--title".to_string(),
        bug_report_issue_title(report),
        "--body-file".to_string(),
        issue_body_path.display().to_string(),
    ];
    for label in &chosen_labels {
        issue_args.push("--label".to_string());
        issue_args.push((*label).to_string());
    }
    let issue_args = with_configured_gh_repo(issue_args);

    let issue_stdout = run_gh_command_with_runner(runner, gh_cwd, &issue_args)?;
    let issue_url = parse_first_url(&issue_stdout)?;
    std::fs::write(report_dir.join("github_issue_url.txt"), format!("{issue_url}\n"))?;

    Ok(GitHubIssueOutcome {
        issue_url,
        warning: log_artifact_warning,
    })
}

/// Maximum time the GitHub issue-filing step (phase 2) may run before we give
/// up and report it as failed. The bug report is already persisted to disk by
/// this point, so a slow or hung `gh` invocation must NEVER block the user — we
/// time out and report the GitHub step as failed while the disk copy stays safe
/// (mtg-5ejgo).
const GITHUB_ISSUE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// Build the phase-1 (`BugReportStored`) message from the disk-write result.
///
/// Logging lives here so both WebSocket call sites (lobby + in-game) stay DRY:
/// each only has to send the returned message.
fn bug_report_stored_message(stored: &Result<StoredBugReport>, reporter_player_id: Option<PlayerId>) -> ServerMessage {
    match stored {
        Ok(stored_report) => {
            log::info!(
                "Stored bug report from {:?} in {}",
                reporter_player_id,
                stored_report.report_dir.display()
            );
            ServerMessage::BugReportStored {
                success: true,
                report_dir: Some(stored_report.report_dir.display().to_string()),
                error: None,
            }
        }
        Err(error) => {
            log::error!("Bug report disk write failed: {}", error);
            ServerMessage::BugReportStored {
                success: false,
                report_dir: None,
                error: Some(error.to_string()),
            }
        }
    }
}

/// Collapse the nested `timeout(spawn_blocking(github))` result into a single
/// `Result<GitHubIssueOutcome>`, mapping the timeout and task-join failure modes
/// to descriptive errors.
fn flatten_github_outcome(
    outcome: std::result::Result<
        std::result::Result<Result<GitHubIssueOutcome>, tokio::task::JoinError>,
        tokio::time::error::Elapsed,
    >,
) -> Result<GitHubIssueOutcome> {
    match outcome {
        Ok(Ok(result)) => result,
        Ok(Err(join_error)) => Err(anyhow!("GitHub issue task failed: {join_error}")),
        Err(_elapsed) => Err(anyhow!(
            "GitHub issue filing timed out after {} seconds",
            GITHUB_ISSUE_TIMEOUT.as_secs()
        )),
    }
}

/// Build the phase-2 (`BugReportIssueResult`) message from the GitHub outcome.
fn bug_report_issue_message(github_result: Result<GitHubIssueOutcome>) -> ServerMessage {
    match github_result {
        Ok(issue) => ServerMessage::BugReportIssueResult {
            issue_url: Some(issue.issue_url),
            error: issue.warning,
        },
        Err(error) => {
            log::warn!("Bug report stored locally, but GitHub issue creation failed: {}", error);
            ServerMessage::BugReportIssueResult {
                issue_url: None,
                error: Some(format!("GitHub issue creation failed: {}", error)),
            }
        }
    }
}

/// Phase 2 of a bug report submission: attempt to file the GitHub issue, bounded
/// by [`GITHUB_ISSUE_TIMEOUT`], and return the resulting `BugReportIssueResult`
/// message. The report is already persisted by the caller, so every failure mode
/// (gh missing, network hang, timeout) resolves to a non-fatal message rather
/// than blocking. Parameterized over the command `runner` and `timeout` so tests
/// can simulate a slow or failing `gh` without invoking the real binary.
async fn file_bug_report_issue_with_runner_and_timeout<R>(
    runner: R,
    timeout: std::time::Duration,
    report: &BugReportRequest,
    stored_report: &StoredBugReport,
    reporter_player_id: Option<PlayerId>,
) -> ServerMessage
where
    R: Fn(&[String], &Path) -> std::io::Result<CommandOutput> + Send + 'static,
{
    let report_clone = report.clone();
    let report_dir_clone = stored_report.report_dir.clone();
    let outcome = tokio::time::timeout(
        timeout,
        tokio::task::spawn_blocking(move || {
            create_github_issue_with_runner(&runner, &report_clone, &report_dir_clone, reporter_player_id)
        }),
    )
    .await;

    let github_result = flatten_github_outcome(outcome);
    let issue_url = github_result.as_ref().ok().map(|issue| issue.issue_url.clone());
    maybe_schedule_claude_autofix(
        report,
        &stored_report.report_dir,
        reporter_player_id,
        stored_report,
        issue_url.as_deref(),
    );
    bug_report_issue_message(github_result)
}

/// Phase 2 with the production command runner and timeout.
async fn file_bug_report_issue(
    report: &BugReportRequest,
    stored_report: &StoredBugReport,
    reporter_player_id: Option<PlayerId>,
) -> ServerMessage {
    file_bug_report_issue_with_runner_and_timeout(
        run_command,
        GITHUB_ISSUE_TIMEOUT,
        report,
        stored_report,
        reporter_player_id,
    )
    .await
}

async fn store_bug_report(
    config: &ServerConfig,
    report: &BugReportRequest,
    reporter_player_id: Option<PlayerId>,
) -> Result<StoredBugReport> {
    let trusted =
        validate_trusted_bug_report_password(&config.trusted_bug_report_password, report.trusted_password.as_deref());
    let timestamp_ms = now_ms();
    let report_dir = create_bug_report_dir(&config.bug_reports_dir, timestamp_ms).await?;

    fs::write(report_dir.join("user_report.txt"), &report.description).await?;
    fs::write(report_dir.join("game_logs.txt"), &report.game_logs).await?;
    fs::write(report_dir.join("console_logs.txt"), &report.console_logs).await?;

    let metadata = BugReportMetadata {
        timestamp_ms,
        reporter_player_id: reporter_player_id.map(|player_id| player_id.as_u32()),
        trusted,
        trusted_password_supplied: report.trusted_password.is_some(),
    };
    fs::write(report_dir.join("metadata.json"), serde_json::to_vec_pretty(&metadata)?).await?;

    Ok(StoredBugReport { report_dir, trusted })
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
    use std::fs as stdfs;
    use std::sync::{Arc, Mutex as StdMutex};
    use tempfile::tempdir;
    use tokio::sync::oneshot as tokio_oneshot;
    use tokio::time::{sleep, timeout, Duration};

    fn make_output(stdout: &str, stderr: &str) -> CommandOutput {
        CommandOutput {
            success: true,
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
        }
    }

    #[test]
    fn test_with_configured_gh_repo() {
        // Snapshot any externally-set value so the test is self-contained.
        let previous = std::env::var("MTG_GH_REPO").ok();

        // Unset: falls back to the compiled-in default repo so filing is
        // ALWAYS scoped to an explicit `-R <repo>` (never cwd-dependent).
        std::env::remove_var("MTG_GH_REPO");
        let args = with_configured_gh_repo(vec!["issue".to_string(), "create".to_string()]);
        assert_eq!(
            args,
            vec![
                "issue".to_string(),
                "create".to_string(),
                "-R".to_string(),
                BUG_REPORT_GITHUB_REPO.to_string(),
            ]
        );

        // Configured: `-R <OWNER/REPO>` overrides the default so a future
        // org/repo move is a single config change rather than a code edit.
        std::env::set_var("MTG_GH_REPO", "exampleorg/example-repo");
        let args = with_configured_gh_repo(vec!["issue".to_string(), "create".to_string()]);
        assert_eq!(
            args,
            vec![
                "issue".to_string(),
                "create".to_string(),
                "-R".to_string(),
                "exampleorg/example-repo".to_string(),
            ]
        );

        // Whitespace-only / blank values are treated as unset (default repo).
        std::env::set_var("MTG_GH_REPO", "   ");
        let args = with_configured_gh_repo(vec!["issue".to_string()]);
        assert_eq!(
            args,
            vec![
                "issue".to_string(),
                "-R".to_string(),
                BUG_REPORT_GITHUB_REPO.to_string(),
            ]
        );

        // Restore the prior environment.
        match previous {
            Some(value) => std::env::set_var("MTG_GH_REPO", value),
            None => std::env::remove_var("MTG_GH_REPO"),
        }
    }

    #[test]
    fn test_server_config_default() {
        let config = ServerConfig::default();
        assert_eq!(config.port, DEFAULT_PORT);
        assert_eq!(config.starting_life, 20);
        assert!(!config.deck_visibility);
        assert!(config.trusted_bug_report_password.is_empty());
        assert_eq!(config.bug_reports_dir, PathBuf::from("bug_reports"));
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

    #[test]
    fn test_validate_trusted_bug_report_password() {
        // Empty expected password → always untrusted (cannot configure trust).
        assert!(!validate_trusted_bug_report_password("", None));
        assert!(!validate_trusted_bug_report_password("", Some("anything")));

        // Configured password, correct → trusted.
        assert!(validate_trusted_bug_report_password("trusted", Some("trusted")));

        // Configured password, wrong → UNTRUSTED (NOT an error — report is still stored).
        // This is the key fix: the old code returned Err and rejected the upload.
        assert!(!validate_trusted_bug_report_password("trusted", Some("wrong")));

        // Configured password, none supplied → untrusted.
        assert!(!validate_trusted_bug_report_password("trusted", None));
    }

    #[tokio::test]
    async fn test_store_bug_report_writes_expected_files() {
        let temp = tempdir().expect("tempdir");
        let config = ServerConfig {
            trusted_bug_report_password: "trusted".to_string(),
            bug_reports_dir: temp.path().join("bug_reports"),
            ..Default::default()
        };
        let report = BugReportRequest {
            description: "The client froze after combat damage.".to_string(),
            game_logs: "[GAMELOG] Combat damage step".to_string(),
            console_logs: "TypeError: Cannot read properties of undefined".to_string(),
            trusted_password: Some("trusted".to_string()),
        };

        let stored_report = store_bug_report(&config, &report, Some(PlayerId::new(1)))
            .await
            .expect("store report");
        let report_dir = stored_report.report_dir;

        assert!(report_dir.starts_with(&config.bug_reports_dir));
        assert!(stored_report.trusted);
        assert_eq!(
            stdfs::read_to_string(report_dir.join("user_report.txt")).expect("user report"),
            report.description
        );
        assert_eq!(
            stdfs::read_to_string(report_dir.join("game_logs.txt")).expect("game logs"),
            report.game_logs
        );
        assert_eq!(
            stdfs::read_to_string(report_dir.join("console_logs.txt")).expect("console logs"),
            report.console_logs
        );

        let metadata: serde_json::Value =
            serde_json::from_str(&stdfs::read_to_string(report_dir.join("metadata.json")).expect("metadata"))
                .expect("parse metadata");
        assert_eq!(metadata["reporter_player_id"], 1);
        assert_eq!(metadata["trusted"], true);
        assert_eq!(metadata["trusted_password_supplied"], true);
        assert!(metadata["timestamp_ms"].as_u64().is_some());
    }

    /// A wrong password must NOT reject the upload — the report is stored as
    /// UNTRUSTED. This test was the inverse under the old `Result<bool>`
    /// implementation; it is now the key regression guard for mtg-obrx2.
    #[tokio::test]
    async fn test_store_bug_report_stores_with_wrong_password_as_untrusted() {
        let temp = tempdir().expect("tempdir");
        let config = ServerConfig {
            trusted_bug_report_password: "trusted".to_string(),
            bug_reports_dir: temp.path().join("bug_reports"),
            ..Default::default()
        };
        let report = BugReportRequest {
            description: "desc".to_string(),
            game_logs: "game".to_string(),
            console_logs: "console".to_string(),
            trusted_password: Some("wrong".to_string()),
        };

        // The report MUST be stored (not rejected) even with the wrong password.
        let stored = store_bug_report(&config, &report, None)
            .await
            .expect("wrong password must not reject — always store");
        // trusted=false because the password was wrong.
        assert!(!stored.trusted, "wrong password should yield trusted=false");
        // Files must exist.
        assert!(stored.report_dir.join("user_report.txt").exists());
        let metadata: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(stored.report_dir.join("metadata.json")).unwrap()).unwrap();
        assert_eq!(metadata["trusted"], false);
        assert_eq!(metadata["trusted_password_supplied"], true);
    }

    /// No password supplied → untrusted, still stored.
    #[tokio::test]
    async fn test_store_bug_report_stores_without_password_as_untrusted() {
        let temp = tempdir().expect("tempdir");
        let config = ServerConfig {
            trusted_bug_report_password: "trusted".to_string(),
            bug_reports_dir: temp.path().join("bug_reports"),
            ..Default::default()
        };
        let report = BugReportRequest {
            description: "no pw".to_string(),
            game_logs: "g".to_string(),
            console_logs: "c".to_string(),
            trusted_password: None,
        };
        let stored = store_bug_report(&config, &report, None).await.expect("store");
        assert!(!stored.trusted);
    }

    #[test]
    fn test_create_github_issue_with_runner_builds_expected_commands() {
        let temp = tempdir().expect("tempdir");
        let report_dir = temp.path().join("bug_report");
        stdfs::create_dir_all(&report_dir).expect("report dir");
        stdfs::write(report_dir.join("game_logs.txt"), "game log").expect("game logs");
        stdfs::write(report_dir.join("console_logs.txt"), "console log").expect("console logs");

        let report = BugReportRequest {
            description: "Priority pass caused a client hang.\nSecond line.".to_string(),
            game_logs: "game log".to_string(),
            console_logs: "console log".to_string(),
            trusted_password: None,
        };

        let calls = Arc::new(StdMutex::new(Vec::<Vec<String>>::new()));
        let calls_clone = Arc::clone(&calls);
        // No MTG_GH_PROXY in the test env → commands are ["/usr/bin/gh", <sub>, ...],
        // so the gh subcommand is at index 1.
        let runner = move |args: &[String], _cwd: &Path| -> std::io::Result<CommandOutput> {
            calls_clone.lock().expect("lock calls").push(args.to_vec());
            if args.get(1).map(String::as_str) == Some("auth") {
                Ok(make_output("Logged in to github.com as rrnewton\n", ""))
            } else if args.get(1).map(String::as_str) == Some("label") {
                Ok(make_output(r#"[{"name":"bug"},{"name":"triage"}]"#, ""))
            } else if args.get(1).map(String::as_str) == Some("gist") {
                Ok(make_output("https://gist.github.com/example/logs\n", ""))
            } else if args.get(1).map(String::as_str) == Some("issue") {
                Ok(make_output("https://github.com/rrnewton/DeepScry/issues/123\n", ""))
            } else {
                panic!("unexpected command: {args:?}");
            }
        };

        let outcome =
            create_github_issue_with_runner(&runner, &report, &report_dir, Some(PlayerId::new(1))).expect("issue");

        assert_eq!(
            outcome,
            GitHubIssueOutcome {
                issue_url: "https://github.com/rrnewton/DeepScry/issues/123".to_string(),
                warning: None,
            }
        );

        let calls = calls.lock().expect("lock calls");
        assert_eq!(calls.len(), 4);
        // Default (no MTG_GH_PROXY): each command is ["/usr/bin/gh", <sub>, ...]
        // — invoked directly, no proxy wrapper (mtg-zvlpk).
        // 0: gh auth status preflight
        assert_eq!(calls[0][0], "/usr/bin/gh");
        assert_eq!(calls[0][1], "auth");
        assert_eq!(calls[0][2], "status");
        // 1: label list -- repo-scoped via explicit -R
        assert_eq!(calls[1][1], "label");
        assert!(calls[1].windows(2).any(|w| w[0] == "-R" && w[1] == "rrnewton/DeepScry"));
        // 2: gist create -- NOT repo-scoped (gists are user-scoped). File-path
        // args MUST be ABSOLUTE so gh can open them regardless of its cwd
        // (mtg-zvlpk: a relative path resolved under the wrong cwd → "no such file").
        assert_eq!(calls[2][1], "gist");
        assert!(calls[2]
            .iter()
            .any(|arg| arg.ends_with("game_logs.txt") && Path::new(arg).is_absolute()));
        assert!(calls[2]
            .iter()
            .any(|arg| arg.ends_with("console_logs.txt") && Path::new(arg).is_absolute()));
        assert!(!calls[2].iter().any(|arg| arg == "-R"));
        // 3: issue create -- repo-scoped via explicit -R
        assert_eq!(calls[3][1], "issue");
        assert!(calls[3].windows(2).any(|w| w[0] == "-R" && w[1] == "rrnewton/DeepScry"));
        assert!(calls[3].windows(2).any(|w| w[0] == "--label" && w[1] == "bug"));
        assert!(calls[3].windows(2).any(|w| w[0] == "--label" && w[1] == "triage"));
        // --body-file MUST be an ABSOLUTE path (mtg-zvlpk).
        assert!(calls[3].windows(2).any(|w| w[0] == "--body-file"
            && w[1].ends_with("github_issue_body.md")
            && Path::new(&w[1]).is_absolute()));

        let issue_body = stdfs::read_to_string(report_dir.join("github_issue_body.md")).expect("issue body");
        assert!(issue_body.contains("Priority pass caused a client hang."));
        assert!(issue_body.contains("https://gist.github.com/example/logs"));
        assert!(stdfs::read_to_string(report_dir.join("github_issue_url.txt"))
            .expect("issue url")
            .contains("/issues/123"));
    }

    #[test]
    fn test_check_gh_auth_with_runner_reports_unauthenticated() {
        // Authenticated: gh auth status exits 0.
        let ok_runner = |args: &[String], _cwd: &Path| -> std::io::Result<CommandOutput> {
            // Default (no MTG_GH_PROXY): command is ["/usr/bin/gh", "auth", "status"].
            assert_eq!(args.first().map(String::as_str), Some("/usr/bin/gh"));
            assert_eq!(args.get(1).map(String::as_str), Some("auth"));
            assert_eq!(args.get(2).map(String::as_str), Some("status"));
            Ok(make_output("Logged in to github.com\n", ""))
        };
        assert!(check_gh_auth_with_runner(&ok_runner, Path::new("/tmp")).is_ok());

        // Unauthenticated: gh auth status exits non-zero -> Err (treated as a
        // non-fatal warning by the caller, which falls back to local storage).
        let fail_runner = |_args: &[String], _cwd: &Path| -> std::io::Result<CommandOutput> {
            Ok(CommandOutput {
                success: false,
                stdout: String::new(),
                stderr: "You are not logged into any GitHub hosts.".to_string(),
            })
        };
        let error = check_gh_auth_with_runner(&fail_runner, Path::new("/tmp")).expect_err("unauthenticated");
        assert!(error.to_string().contains("not logged into"));
    }

    #[test]
    fn test_absolutize_under() {
        // A relative path is resolved against the base (server working dir) →
        // absolute. This is what makes the gh `--body-file` arg independent of
        // gh's own cwd (mtg-zvlpk: a relative arg + relative cwd double-nested).
        assert_eq!(
            absolutize_under(Path::new("bug_reports/123/github_issue_body.md"), Path::new("/srv/app")),
            PathBuf::from("/srv/app/bug_reports/123/github_issue_body.md")
        );
        // An already-absolute path is returned unchanged (base is ignored).
        assert_eq!(
            absolutize_under(Path::new("/var/reports/123/x.md"), Path::new("/srv/app")),
            PathBuf::from("/var/reports/123/x.md")
        );
    }

    #[test]
    fn test_proxy_prefix_from() {
        // Unset / empty / whitespace → no proxy wrapper (invoke directly). This is
        // the default + production-VM path: the old hardcoded /usr/bin/with-proxy
        // is absent on the VM and made gh ENOENT (mtg-zvlpk).
        assert_eq!(proxy_prefix_from(None), Vec::<String>::new());
        assert_eq!(proxy_prefix_from(Some("")), Vec::<String>::new());
        assert_eq!(proxy_prefix_from(Some("   ")), Vec::<String>::new());
        // A non-empty value (trimmed) becomes the single-element command prefix,
        // for networks that genuinely require an egress proxy (Meta devservers).
        assert_eq!(
            proxy_prefix_from(Some("/usr/bin/with-proxy")),
            vec!["/usr/bin/with-proxy".to_string()]
        );
        assert_eq!(
            proxy_prefix_from(Some("  /usr/bin/with-proxy  ")),
            vec!["/usr/bin/with-proxy".to_string()]
        );
    }

    #[test]
    fn test_create_github_issue_with_runner_handles_missing_gh() {
        let temp = tempdir().expect("tempdir");
        let report_dir = temp.path().join("bug_report");
        stdfs::create_dir_all(&report_dir).expect("report dir");
        stdfs::write(report_dir.join("game_logs.txt"), "game log").expect("game logs");
        stdfs::write(report_dir.join("console_logs.txt"), "console log").expect("console logs");

        let report = BugReportRequest {
            description: "Desync after combat".to_string(),
            game_logs: "game log".to_string(),
            console_logs: "console log".to_string(),
            trusted_password: None,
        };

        let runner = |_args: &[String], _cwd: &Path| -> std::io::Result<CommandOutput> {
            Err(std::io::Error::new(ErrorKind::NotFound, "gh not found"))
        };

        let error =
            create_github_issue_with_runner(&runner, &report, &report_dir, None).expect_err("missing gh should fail");
        assert!(error.to_string().contains("gh not found"));
    }

    #[test]
    #[allow(clippy::wildcard_enum_match_arm)]
    fn test_bug_report_issue_message_reports_failure_without_url() {
        let response = bug_report_issue_message(Err(anyhow!("gh not found")));
        match response {
            ServerMessage::BugReportIssueResult { issue_url, error } => {
                assert_eq!(issue_url, None);
                assert!(error.expect("error message").contains("GitHub issue creation failed"));
            }
            other => panic!("unexpected response: {other:?}"),
        }
    }

    /// The genuinely-bad case: a failed disk write must report `success: false`
    /// with the error, so the client's "saved to disk" box shows a failure and
    /// the user can retry (mtg-5ejgo).
    #[test]
    #[allow(clippy::wildcard_enum_match_arm)]
    fn test_bug_report_stored_message_reports_disk_failure() {
        let response = bug_report_stored_message(&Err(anyhow!("disk full")), Some(PlayerId::new(2)));
        match response {
            ServerMessage::BugReportStored {
                success,
                report_dir,
                error,
            } => {
                assert!(!success);
                assert_eq!(report_dir, None);
                assert!(error.expect("error message").contains("disk full"));
            }
            other => panic!("unexpected response: {other:?}"),
        }
    }

    /// A `gh` invocation that returns ENOENT instantly must resolve phase 2 to a
    /// failure message (no URL) WITHOUT hitting the timeout (mtg-5ejgo).
    #[tokio::test]
    #[allow(clippy::wildcard_enum_match_arm)]
    async fn test_file_bug_report_issue_reports_github_failure() {
        let temp = tempdir().expect("tempdir");
        let report_dir = temp.path().join("bug_report");
        stdfs::create_dir_all(&report_dir).expect("report dir");
        let report = BugReportRequest {
            description: "Desync after combat".to_string(),
            game_logs: "game log".to_string(),
            console_logs: "console log".to_string(),
            trusted_password: None,
        };
        let stored = StoredBugReport {
            report_dir: report_dir.clone(),
            trusted: false,
        };
        let failing_runner = |_args: &[String], _cwd: &Path| -> std::io::Result<CommandOutput> {
            Err(std::io::Error::new(ErrorKind::NotFound, "gh not found"))
        };

        let response = file_bug_report_issue_with_runner_and_timeout(
            failing_runner,
            Duration::from_secs(15),
            &report,
            &stored,
            None,
        )
        .await;
        match response {
            ServerMessage::BugReportIssueResult { issue_url, error } => {
                assert_eq!(issue_url, None);
                assert!(error.expect("error").contains("GitHub issue creation failed"));
            }
            other => panic!("unexpected response: {other:?}"),
        }
    }

    /// A hung `gh` invocation must be bounded by the timeout: phase 2 resolves to
    /// a "timed out" failure message rather than blocking the user forever
    /// (mtg-5ejgo). Uses a short timeout + a runner that sleeps past it.
    #[tokio::test]
    #[allow(clippy::wildcard_enum_match_arm)]
    async fn test_file_bug_report_issue_times_out_on_slow_gh() {
        let temp = tempdir().expect("tempdir");
        let report_dir = temp.path().join("bug_report");
        stdfs::create_dir_all(&report_dir).expect("report dir");
        let report = BugReportRequest {
            description: "Hang repro".to_string(),
            game_logs: "game log".to_string(),
            console_logs: "console log".to_string(),
            trusted_password: None,
        };
        let stored = StoredBugReport {
            report_dir: report_dir.clone(),
            trusted: false,
        };
        // Runner blocks well past the (tiny) timeout to simulate a hung gh.
        let slow_runner = |_args: &[String], _cwd: &Path| -> std::io::Result<CommandOutput> {
            std::thread::sleep(Duration::from_millis(400));
            Ok(CommandOutput {
                success: true,
                stdout: "https://github.com/example/repo/issues/1\n".to_string(),
                stderr: String::new(),
            })
        };

        let response = file_bug_report_issue_with_runner_and_timeout(
            slow_runner,
            Duration::from_millis(50),
            &report,
            &stored,
            None,
        )
        .await;
        match response {
            ServerMessage::BugReportIssueResult { issue_url, error } => {
                assert_eq!(issue_url, None);
                assert!(error.expect("error").contains("timed out"));
            }
            other => panic!("unexpected response: {other:?}"),
        }
    }

    #[test]
    fn test_launch_claude_autofix_with_spawner_builds_expected_command() {
        let repo_root = PathBuf::from("/tmp/mtg-repo");
        let request = AutoFixLaunchRequest {
            issue_url: "https://github.com/rrnewton/DeepScry/issues/123".to_string(),
            prompt: "Fix the bug".to_string(),
        };
        let seen_args = Arc::new(StdMutex::new(Vec::<String>::new()));
        let seen_args_clone = Arc::clone(&seen_args);
        let seen_cwd = Arc::new(StdMutex::new(PathBuf::new()));
        let seen_cwd_clone = Arc::clone(&seen_cwd);
        let spawner = move |args: &[String], cwd: &Path| -> std::io::Result<Option<u32>> {
            *seen_args_clone.lock().expect("lock args") = args.to_vec();
            *seen_cwd_clone.lock().expect("lock cwd") = cwd.to_path_buf();
            Ok(Some(4242))
        };

        let pid = launch_claude_autofix_with_spawner(&spawner, &repo_root, &request).expect("launch");

        assert_eq!(pid, Some(4242));
        let args = seen_args.lock().expect("lock args");
        // Default (no MTG_GH_PROXY): claude is spawned directly, no proxy wrapper.
        assert_eq!(
            args.as_slice(),
            &[
                "claude".to_string(),
                "--dangerously-skip-permissions".to_string(),
                "-p".to_string(),
                "Fix the bug".to_string(),
            ]
        );
        assert_eq!(*seen_cwd.lock().expect("lock cwd"), repo_root);
    }

    #[tokio::test]
    async fn test_schedule_claude_autofix_with_spawner_is_fire_and_forget() {
        let (started_tx, started_rx) = tokio_oneshot::channel::<()>();
        let started_tx = Arc::new(StdMutex::new(Some(started_tx)));
        let spawner = {
            let started_tx = Arc::clone(&started_tx);
            move |_args: &[String], _cwd: &Path| -> std::io::Result<Option<u32>> {
                if let Some(tx) = started_tx.lock().expect("lock sender").take() {
                    let _ = tx.send(());
                }
                std::thread::sleep(std::time::Duration::from_millis(200));
                Ok(Some(777))
            }
        };

        let before = std::time::Instant::now();
        schedule_claude_autofix_with_spawner(
            Arc::new(spawner),
            PathBuf::from("/tmp/mtg-repo"),
            AutoFixLaunchRequest {
                issue_url: "https://github.com/rrnewton/DeepScry/issues/9".to_string(),
                prompt: "Prompt".to_string(),
            },
        );
        let elapsed = before.elapsed();

        assert!(elapsed < Duration::from_millis(50));
        timeout(Duration::from_secs(1), started_rx)
            .await
            .expect("spawned task should run")
            .expect("sender should send");
        sleep(Duration::from_millis(250)).await;
    }

    #[tokio::test]
    async fn test_maybe_schedule_claude_autofix_skips_untrusted_or_missing_issue() {
        let report = BugReportRequest {
            description: "desc".to_string(),
            game_logs: "game".to_string(),
            console_logs: "console".to_string(),
            trusted_password: None,
        };
        maybe_schedule_claude_autofix(
            &report,
            Path::new("/tmp/report"),
            None,
            &StoredBugReport {
                report_dir: PathBuf::from("/tmp/report"),
                trusted: false,
            },
            Some("https://github.com/rrnewton/DeepScry/issues/1"),
        );
        maybe_schedule_claude_autofix(
            &report,
            Path::new("/tmp/report"),
            None,
            &StoredBugReport {
                report_dir: PathBuf::from("/tmp/report"),
                trusted: true,
            },
            None,
        );
    }
}
