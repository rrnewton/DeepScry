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
use crate::network::{ActionLog, ChoiceEntry, StateSyncEntry};
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
        /// Server-assigned sequence number (the `ActionLog<ChoiceEntry>` key
        /// — strictly unique/monotonic per choice, unlike `action_count`).
        choice_seq: u32,
        choice_indices: Vec<usize>,
        description: String,
        spell_ability: Option<crate::core::SpellAbility>,
        /// The CardId chosen for library search operations
        library_search_result: Option<CardId>,
        /// Actual target CardIds (if this was a target choice)
        target_card_ids: Option<Vec<CardId>>,
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
            ServerMessage::CardRevealed {
                owner,
                card,
                reason,
                // mtg-610: the effective-action_count stamp is consumed only by
                // the WASM shadow's reveal-history rewind path. The native client
                // uses a blocking-thread model with no client-side rewind, so it
                // ignores the stamp (eager frontier consume is unchanged).
                action_count: _,
            } => Some(NetworkMessage::CardRevealed { owner, card, reason }),
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
                choice_seq,
                choice_indices,
                description,
                spell_ability,
                library_search_result,
                target_card_ids,
                action_count,
                ..
            } => Some(NetworkMessage::OpponentChoice {
                action_count,
                choice_seq,
                choice_indices,
                description,
                spell_ability,
                library_search_result,
                target_card_ids,
            }),
            ServerMessage::GameEnded {
                winner, action_count, ..
            } => Some(NetworkMessage::GameEnded { winner, action_count }),
            ServerMessage::Error { message, fatal } => Some(NetworkMessage::Error { message, fatal }),
            ServerMessage::LibraryReordered {
                player, new_order, ..
            } => Some(NetworkMessage::LibraryReordered { player, new_order }),
            // Ignore connection/setup messages - handled during connection setup, not gameplay.
            // Lobby messages (GameList/GameCreated/ServerFull/JoinFailed/WaitingRoomUpdate/
            // RegisterResult/ReconnectResult) are also pre-gameplay: the lobby flow consumes
            // them before the in-game NetworkMessage stream begins, so they are never
            // expected here.
            ServerMessage::AuthResult { .. }
            | ServerMessage::RegisterResult { .. }
            | ServerMessage::BugReportStored { .. }
            | ServerMessage::BugReportIssueResult { .. }
            | ServerMessage::WaitingForOpponent
            | ServerMessage::WaitingRoomUpdate { .. }
            | ServerMessage::WaitingRoomReady { .. }
            | ServerMessage::ReconnectResult { .. }
            | ServerMessage::GameStarted { .. }
            | ServerMessage::GameList { .. }
            | ServerMessage::GameCreated { .. }
            | ServerMessage::ServerFull { .. }
            | ServerMessage::JoinFailed { .. }
            | ServerMessage::SyncError { .. }
            // L1 (mtg-o99ow): SearchCandidates is not emitted yet (library-search
            // candidates still arrive as N CardRevealed). When L2 emits it, the
            // native shadow will need to consume it like the WASM client does.
            | ServerMessage::SearchCandidates { .. }
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

// Phase 2 step 3 (mtg-i2x3r / netarch): the legacy `RemoteChoiceInfo`,
// `ChoiceAcceptedInfo`, and the deprecated `ChoiceInfo` enums, plus the
// `PendingReveal` struct, are GONE. Opponent choices and ChoiceAccepted
// acks now flow through the shared `ActionLog<ChoiceEntry>` buffers, and
// reveals through `ActionLog<StateSyncEntry>`. Only `LocalChoiceInfo`
// (still on the local-choice MVar) and `PendingLibraryReorder` (the
// pre-game-loop initial-reorder carrier) remain.

/// Pending library reorder for sync callback processing
///
/// Contains a LibraryReordered message. Library reorders are processed
/// BEFORE reveals to ensure libraries are in sync before cards are drawn.
#[derive(Debug, Clone)]
pub struct PendingLibraryReorder {
    pub player: PlayerId,
    /// New library order, top-to-bottom (protocol format)
    pub new_order: Vec<CardId>,
}

/// Shadow state-sync log + apply cursor (Phase 2 step 3a).
///
/// Wraps the SHARED `ActionLog<StateSyncEntry>` primitive plus the two
/// pieces of consumer-owned bookkeeping the sync callback needs: the
/// monotonic `action_count` allocator used when appending, and the
/// `last_applied_ac` cursor that records how far the shadow `GameState`
/// has been advanced. The whole struct lives behind one `Mutex` in
/// `SharedNetworkState` (the lock wraps the owner, not the log —
/// docs/NETWORK_ACTION_LOG.md § 3.3).
#[derive(Default)]
struct StateSyncBuffer {
    /// Append-only, `action_count`-indexed shadow state-sync log. Shared
    /// primitive (`crate::network::ActionLog<StateSyncEntry>`), identical
    /// to the WASM client's `state_sync` field.
    log: ActionLog<StateSyncEntry>,
    /// Synthetic `action_count` allocator for pushes. The wire messages
    /// (`CardRevealed` / `LibraryReordered`) do not currently carry a
    /// server `action_count`, so the appender bumps this counter to keep
    /// the log strictly monotonic. Mirrors the WASM `next_state_sync_ac`.
    next_ac: u64,
    /// Cursor: highest `action_count` whose entry has been applied to the
    /// shadow `GameState`. `apply_up_to_frontier` walks entries with
    /// `last_applied_ac < ac <= frontier()` and bumps this.
    last_applied_ac: u64,
}

/// Opponent-choice buffer + FIFO read cursor (Phase 2 step 3b).
///
/// Wraps the SHARED `ActionLog<ChoiceEntry>` primitive keyed by the
/// server `choice_seq`, plus the engine-side read cursor that hands out
/// choices in `choice_seq` order (the native equivalent of the WASM
/// `next_opponent_choice_cursor`). Reads are non-destructive; only the
/// cursor advances, so a rewind/replay re-hands the same choices.
#[derive(Default)]
struct OpponentChoiceBuffer {
    log: ActionLog<ChoiceEntry>,
    /// Highest `choice_seq` already handed to the controller. The next
    /// read returns the first entry with `choice_seq > cursor`.
    cursor: u64,
}

/// Local ChoiceAccepted buffer + read cursor (Phase 2 step 3c).
///
/// Wraps a SHARED `ActionLog<ChoiceEntry>` keyed by the server
/// `choice_seq`. Only the `choice_seq` + `library_search_result` fields
/// of `ChoiceEntry` are meaningful here (the rest are unused for accepted
/// acks); reusing `ChoiceEntry` keeps native + WASM on one payload set
/// rather than introducing a parallel native-only struct. Read
/// non-destructively by `choice_seq`.
#[derive(Default)]
struct ChoiceAcceptedBuffer {
    log: ActionLog<ChoiceEntry>,
    /// Highest `choice_seq` already consumed by the local controller.
    cursor: u64,
}

/// Shared network state for synchronization between network loop and game loop
///
/// This structure implements choice + state synchronization using the
/// shared `ActionLog` primitive (Phase 2 step 3) plus an MVar for local
/// ChoiceRequests:
/// - `state_sync`: `ActionLog<StateSyncEntry>` of CardRevealed / LibraryReordered
/// - `local_choice_mvar`: MVar for ChoiceRequest messages (local player)
/// - `opponent_choices`: `ActionLog<ChoiceEntry>` of OpponentChoice (remote player)
/// - `choice_accepted`: `ActionLog<ChoiceEntry>` of ChoiceAccepted (local acks)
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
    /// Shadow state-sync log + apply cursor, behind one lock.
    ///
    /// Phase 2 step 3a (mtg-i2x3r / netarch): this REPLACES the legacy
    /// `pending_reveals` / `pending_library_reorders` VecDeques, the
    /// `library_reorder_condvar`, and the `choice_pending` race-fixer flag.
    /// `CardRevealed` and `LibraryReordered` are now appended to a single
    /// `ActionLog<StateSyncEntry>` keyed by a monotonic `action_count`, and
    /// the sync callback walks unapplied entries up to the frontier with
    /// reorder-before-reveal ordering (mtg-589). Reads are non-destructive
    /// (the log is append-only; only the cursor moves), which is what gives
    /// native the same rewind/replay property the WASM client already has —
    /// and it reuses the SAME shared `ActionLog<StateSyncEntry>` primitive
    /// (invariant #10: one primitive, native + WASM).
    state_sync: std::sync::Mutex<StateSyncBuffer>,

    /// Condvar notified on every `state_sync` push. The blocking game-loop
    /// thread waits on this (NO timeout) until the state-sync frontier
    /// reaches the count it needs — the native trampoline-equivalent of the
    /// WASM "return `NeedsInput` and unwind" path
    /// (docs/NETWORK_ACTION_LOG.md § 4). A timeout here would mean "proceed
    /// with stale data" = silent desync, which is exactly the bug the old
    /// `wait_for_library_reorders` timeout introduced; data arrival or a
    /// fatal disconnect are the only things that release the wait.
    state_sync_notify: std::sync::Condvar,

    /// MVar for local player choice requests (ChoiceRequest messages)
    local_choice_mvar: super::mvar::MVar<LocalChoiceInfo>,

    /// Opponent-choice buffer (`ActionLog<ChoiceEntry>`) + FIFO read cursor,
    /// behind one lock. Phase 2 step 3b: REPLACES the legacy
    /// `remote_choice_mvar`. The WS reader appends each `OpponentChoice`
    /// keyed by its server-assigned `choice_seq` (strictly unique/monotonic
    /// per choice — `action_count` is NOT unique, mtg-sfihb); the
    /// `RemoteController` reads in `choice_seq` order via a non-destructive
    /// cursor, mirroring the WASM `opponent_choices` log + cursor shim.
    opponent_choices: std::sync::Mutex<OpponentChoiceBuffer>,

    /// Condvar notified on every `opponent_choices` push, plus on
    /// exit/end/error signalling. The blocking `RemoteController` waits on
    /// this (NO timeout) until an unconsumed opponent choice is available,
    /// or a terminal (exit/error) state is set.
    opponent_choices_notify: std::sync::Condvar,

    /// Local ChoiceAccepted buffer (`ActionLog<ChoiceEntry>`) + read cursor,
    /// behind one lock. Phase 2 step 3c: REPLACES the legacy
    /// `choice_accepted_mvar` + `take_choice_accepted_for_seq`. Keyed by the
    /// server `choice_seq` (the value `NetworkLocalController` already waits
    /// for), read non-destructively so a rewind/replay re-reads the same
    /// authoritative library-search result.
    choice_accepted: std::sync::Mutex<ChoiceAcceptedBuffer>,

    /// Condvar notified on every `choice_accepted` push, plus on
    /// exit/end/error signalling. `NetworkLocalController::choose_from_library`
    /// waits on this (NO timeout) for the ChoiceAccepted matching its
    /// `choice_seq`.
    choice_accepted_notify: std::sync::Condvar,

    /// Latest action count from server (updated on ChoiceRequest/OpponentChoice)
    /// Used as sync target to ensure client processes all reveals before choices
    server_action_count: std::sync::atomic::AtomicU64,

    /// Terminal flag: set once a `GameEnded`/fatal `Error`/socket close has
    /// been observed by the WS reader. Releases the no-timeout Condvar waits
    /// (state-sync frontier, opponent-choice, choice-accepted) so they
    /// return a terminal result instead of blocking forever. This is the
    /// "fatal disconnect" half of the "data arrival OR fatal disconnect"
    /// wait-release condition (docs/NETWORK_ACTION_LOG.md § 4).
    terminal: std::sync::atomic::AtomicBool,

    /// Server-authoritative game winner, captured from the `GameEnded` message.
    ///
    /// The server is the single source of truth about who won (see
    /// `docs/NETWORK_ARCHITECTURE.md`: "Server has the golden copy"). Both
    /// clients receive the SAME `GameEnded { winner }` over their socket, so
    /// reading the winner from here guarantees both clients agree.
    ///
    /// Without this, `run_game` returned either its locally-derived
    /// `GameResult::winner` (for the client whose GameLoop finished naturally)
    /// or `None` (for the client whose controller hit `ExitGame` first when the
    /// game-end channel closed). Those two paths disagree — producing the flaky
    /// "Clients disagree on winner: Some(1) vs None" failure.
    ///
    /// `None` outer = no `GameEnded` seen yet; `Some(inner)` = server reported
    /// the (possibly-`None`, i.e. draw) winner.
    server_winner: std::sync::Mutex<Option<Option<PlayerId>>>,

    /// Notifies any `run_game` waiter that the `GameEnded` message has been
    /// processed (and `server_winner` populated). The client whose GameLoop
    /// returns first (e.g. its controller hit `ExitGame` from a closed channel)
    /// may race ahead of the reader thread that records the server's verdict.
    /// Awaiting this `Notify` before reading `server_winner` closes that race so
    /// both clients report the SAME server-authoritative winner.
    game_ended_notify: tokio::sync::Notify,
}

impl SharedNetworkState {
    /// Create a new shared network state
    pub fn new() -> Self {
        Self {
            state_sync: std::sync::Mutex::new(StateSyncBuffer::default()),
            state_sync_notify: std::sync::Condvar::new(),
            local_choice_mvar: super::mvar::MVar::new(),
            opponent_choices: std::sync::Mutex::new(OpponentChoiceBuffer::default()),
            opponent_choices_notify: std::sync::Condvar::new(),
            choice_accepted: std::sync::Mutex::new(ChoiceAcceptedBuffer::default()),
            choice_accepted_notify: std::sync::Condvar::new(),
            server_action_count: std::sync::atomic::AtomicU64::new(0),
            terminal: std::sync::atomic::AtomicBool::new(false),
            server_winner: std::sync::Mutex::new(None),
            game_ended_notify: tokio::sync::Notify::new(),
        }
    }

    /// Record the server-authoritative winner from a `GameEnded` message.
    /// First writer wins; subsequent `GameEnded`/close events do not clobber it.
    /// Wakes any `run_game` task awaiting the server verdict via `wait_for_server_winner`.
    pub fn set_server_winner(&self, winner: Option<PlayerId>) {
        if let Ok(mut slot) = self.server_winner.lock() {
            if slot.is_none() {
                *slot = Some(winner);
            }
        }
        // Wake waiters even if the slot was already set (idempotent notify).
        self.game_ended_notify.notify_waiters();
    }

    /// Await the server-authoritative winner (the `GameEnded` message), up to a
    /// bounded timeout. Returns the server's verdict if `GameEnded` arrives in
    /// time, or `None` if it does not (caller then falls back to its locally
    /// derived result). This is NOT a protocol poll loop — it waits once on a
    /// terminal shutdown notification, with a timeout guard so a stuck/aborted
    /// peer cannot hang the client forever.
    pub async fn wait_for_server_winner(&self, timeout: std::time::Duration) -> Option<Option<PlayerId>> {
        // Fast path: already recorded.
        if let Some(w) = self.server_winner() {
            return Some(w);
        }
        // Register for notification BEFORE re-checking to avoid a lost wakeup.
        let notified = self.game_ended_notify.notified();
        if let Some(w) = self.server_winner() {
            return Some(w);
        }
        match tokio::time::timeout(timeout, notified).await {
            Ok(()) => self.server_winner(),
            Err(_) => self.server_winner(),
        }
    }

    /// Get the server-authoritative winner, if a `GameEnded` was received.
    /// Outer `None` => no `GameEnded` observed; `Some(inner)` => server's verdict
    /// (`inner` may itself be `None` for a draw).
    pub fn server_winner(&self) -> Option<Option<PlayerId>> {
        self.server_winner.lock().ok().and_then(|slot| *slot)
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

    // ─────────────────────────────────────────────────────────────────────
    // STATE-SYNC LOG (Phase 2 step 3a — reveal/reorder via ActionLog<StateSyncEntry>)
    // ─────────────────────────────────────────────────────────────────────

    /// Append a `StateSyncEntry` to the shadow state-sync log at the next
    /// synthetic `action_count`, then notify the frontier-wait Condvar.
    ///
    /// Sole appenders: the WS reader (`run_ws_reader_shared`) for live
    /// reveals/reorders, and `run_game` for the initial reorders captured
    /// during `wait_for_game_start`. Append-only / strictly monotonic by
    /// construction (invariant #2 of docs/NETWORK_ACTION_LOG.md § 8).
    fn push_state_sync(&self, entry: StateSyncEntry) {
        let mut buf = self.state_sync.lock().unwrap();
        buf.next_ac += 1;
        let ac = buf.next_ac;
        buf.log.push(ac, entry);
        drop(buf);
        self.state_sync_notify.notify_all();
    }

    /// Append a `CardRevealed` to the shadow state-sync log (WS reader).
    pub fn push_reveal(&self, owner: PlayerId, card: CardReveal, reason: RevealReason) {
        self.push_state_sync(StateSyncEntry::RevealCard {
            owner,
            card: Box::new(card),
            reason,
        });
    }

    /// Append a `LibraryReordered` to the shadow state-sync log (WS reader
    /// for in-game shuffles, and `run_game` for initial reorders).
    pub fn push_library_reorder(&self, player: PlayerId, new_order: Vec<CardId>) {
        self.push_state_sync(StateSyncEntry::LibraryReorder { player, new_order });
    }

    /// Apply every state-sync entry received but not yet applied to the
    /// shadow `game`, up to the current frontier. **Non-destructive read**
    /// of the log — only the per-consumer cursor advances; the log itself is
    /// untouched, so a rewind/replay can re-apply from `reset_state_sync_cursor`.
    ///
    /// CRITICAL ORDERING (mtg-589): within each apply batch, `LibraryReorder`
    /// entries are applied BEFORE `RevealCard` entries (even when a reveal
    /// arrived earlier on the wire), because the server guarantees the
    /// library order before any draw. The legacy two-step
    /// `drain_all_library_reorders → drain_all_reveals` preserved this; the
    /// log preserves it via a two-pass apply over the same cursor window.
    /// The cursor advances over BOTH passes' entries so re-runs after a
    /// rewind replay them in the same per-pass order (bit-identical shadow).
    ///
    /// Returns the number of entries applied (diagnostics).
    pub fn apply_state_sync_up_to_frontier(
        &self,
        game: &mut GameState,
        card_db: &AsyncCardDatabase,
        local_player: PlayerId,
    ) -> usize {
        let mut buf = self.state_sync.lock().unwrap();
        let frontier = match buf.log.frontier() {
            Some(f) => f,
            None => return 0,
        };
        if frontier <= buf.last_applied_ac {
            return 0;
        }

        // Snapshot the cursor window once (apply-by-index would still need a
        // clone to release the lock before touching `game`; the window is
        // tiny — one reveal/reorder per sync point).
        let last_applied = buf.last_applied_ac;
        let to_apply: Vec<(u64, StateSyncEntry)> = buf
            .log
            .iter()
            .filter(|(ac, _)| *ac > last_applied && *ac <= frontier)
            .map(|(ac, entry)| (ac, entry.clone()))
            .collect();
        // Advance the cursor and release the lock before mutating `game`
        // (the WS reader must stay able to append while we apply).
        buf.last_applied_ac = frontier;
        drop(buf);

        // Pass 1: library reorders. Protocol sends top-to-bottom; the shadow
        // library Vec is bottom-to-top (draw pops the last element).
        for (ac, entry) in &to_apply {
            if let StateSyncEntry::LibraryReorder { player, new_order } = entry {
                log::debug!(
                    "apply_state_sync: library reorder ac={} player={:?} ({} cards)",
                    ac,
                    player,
                    new_order.len()
                );
                if let Some(zones) = game.get_player_zones_mut(*player) {
                    zones.library.cards = new_order.iter().rev().copied().collect();
                }
            }
        }

        // Pass 2: card reveals (library order is now server-authoritative).
        let mut applied = 0;
        for (ac, entry) in to_apply {
            if let StateSyncEntry::RevealCard { owner, card, reason } = entry {
                log::debug!(
                    "apply_state_sync: reveal ac={} owner={:?} card={}",
                    ac,
                    owner,
                    card.name
                );
                process_card_reveal(game, card_db, owner, *card, reason, local_player);
            }
            applied += 1;
        }
        applied
    }

    /// Reset the state-sync apply cursor so the next
    /// `apply_state_sync_up_to_frontier` re-applies every entry. Used by
    /// snapshot-resume / rewind: when the engine rewinds `action_count`, the
    /// shadow mutations rewind too and must be re-applied on the forward
    /// replay. The log itself stays intact (non-destructive reads).
    pub fn reset_state_sync_cursor(&self) {
        self.state_sync.lock().unwrap().last_applied_ac = 0;
    }

    /// Block (NO timeout) until the state-sync frontier reaches at least
    /// `count` entries, i.e. until `count` reveals/reorders have arrived.
    ///
    /// This is the native frontier-wait — the blocking-thread equivalent of
    /// the WASM "return `NeedsInput` and unwind" path
    /// (docs/NETWORK_ACTION_LOG.md § 4). It releases ONLY on data arrival
    /// (frontier ≥ `count`) or a terminal disconnect (`terminal` set);
    /// there is deliberately NO timeout, because a timeout would mean
    /// "proceed with stale data" = silent desync (the exact bug the old
    /// `wait_for_library_reorders` timeout caused).
    ///
    /// Returns `true` if the frontier reached `count`, `false` if the wait
    /// was released by a terminal disconnect first.
    pub fn wait_for_state_sync_frontier(&self, count: u64) -> bool {
        let mut buf = self.state_sync.lock().unwrap();
        loop {
            if buf.log.frontier().unwrap_or(0) >= count {
                return true;
            }
            if self.terminal.load(std::sync::atomic::Ordering::Acquire) {
                return buf.log.frontier().unwrap_or(0) >= count;
            }
            buf = self.state_sync_notify.wait(buf).unwrap();
        }
    }

    // ─────────────────────────────────────────────────────────────────────
    // LOCAL CHOICE MVar (ChoiceRequest stays on the MVar)
    // ─────────────────────────────────────────────────────────────────────

    /// Push a local choice request (ChoiceRequest from server)
    pub fn push_local_choice(&self, choice: LocalChoiceInfo) {
        self.local_choice_mvar.put(choice);
    }

    /// Take the next local choice from MVar (called by NetworkLocalController)
    ///
    /// Blocks until a choice is available, then consumes it.
    /// Returns None only if exit has been signaled and MVar is empty.
    pub fn take_local_choice(&self) -> Option<LocalChoiceInfo> {
        self.local_choice_mvar.take()
    }

    // ─────────────────────────────────────────────────────────────────────
    // OPPONENT-CHOICE BUFFER (Phase 2 step 3b — ActionLog<ChoiceEntry>)
    // ─────────────────────────────────────────────────────────────────────

    /// Append an `OpponentChoice` to the per-controller choice buffer, keyed
    /// by the server `choice_seq` (strictly unique/monotonic per choice;
    /// `action_count` is NOT unique — mtg-sfihb). Notifies the Condvar.
    ///
    /// Sole appender: the WS reader. Append-only / strictly monotonic; a
    /// stale or out-of-order `choice_seq` makes `ActionLog::push` panic,
    /// which is the correct fatal response (NETWORK_ARCHITECTURE.md
    /// § "Desync is ALWAYS a Fatal Error").
    pub fn push_opponent_choice(&self, entry: ChoiceEntry) {
        let mut buf = self.opponent_choices.lock().unwrap();
        buf.log.push(u64::from(entry.choice_seq), entry);
        drop(buf);
        self.opponent_choices_notify.notify_all();
    }

    /// Block (NO timeout) for the next unconsumed opponent choice in
    /// `choice_seq` order, advancing the read cursor. Non-destructive: the
    /// entry stays in the log for replay. Releases on data arrival or a
    /// terminal disconnect (the native frontier-wait — § 4).
    ///
    /// Returns `Some(entry)` on a buffered choice, or `None` if a terminal
    /// disconnect was signalled before one arrived (caller treats as exit).
    pub fn take_opponent_choice(&self) -> Option<ChoiceEntry> {
        let mut buf = self.opponent_choices.lock().unwrap();
        loop {
            let next = buf
                .log
                .iter()
                .find(|(seq, _)| *seq > buf.cursor)
                .map(|(seq, e)| (seq, e.clone()));
            if let Some((seq, entry)) = next {
                buf.cursor = seq;
                return Some(entry);
            }
            if self.terminal.load(std::sync::atomic::Ordering::Acquire) {
                return None;
            }
            buf = self.opponent_choices_notify.wait(buf).unwrap();
        }
    }

    /// Reset the opponent-choice read cursor to 0 so a replay re-hands out
    /// every buffered choice (rewind/snapshot-resume support).
    pub fn reset_opponent_choice_cursor(&self) {
        self.opponent_choices.lock().unwrap().cursor = 0;
    }

    // ─────────────────────────────────────────────────────────────────────
    // LOCAL CHOICE-ACCEPTED BUFFER (Phase 2 step 3c — ActionLog<ChoiceEntry>)
    // ─────────────────────────────────────────────────────────────────────

    /// Append a `ChoiceAccepted` ack to the local choice-accepted buffer,
    /// keyed by `choice_seq`. Only `choice_seq` + `library_search_result`
    /// carry meaning here; the other `ChoiceEntry` fields are unused. Reuses
    /// `ChoiceEntry` so native + WASM share one payload set. Notifies.
    ///
    /// Sole appender: the WS reader.
    pub fn push_choice_accepted(&self, choice_seq: u32, library_search_result: Option<CardId>) {
        let mut buf = self.choice_accepted.lock().unwrap();
        buf.log.push(
            u64::from(choice_seq),
            ChoiceEntry {
                choice_seq,
                action_count: 0,
                choice_indices: Vec::new(),
                description: String::new(),
                spell_ability: None,
                library_search_result,
                target_card_ids: None,
            },
        );
        drop(buf);
        self.choice_accepted_notify.notify_all();
    }

    /// Block (NO timeout) for the ChoiceAccepted matching `expected_seq`,
    /// reading it non-destructively (cursor advances to it). The
    /// library-search-result for `expected_seq` is returned. Releases on the
    /// matching entry arriving or a terminal disconnect.
    ///
    /// Returns `Some(library_search_result)` (inner may be `None` for a
    /// non-search accept) when matched, or `None` on terminal disconnect.
    pub fn wait_for_choice_accepted(&self, expected_seq: u32) -> Option<Option<CardId>> {
        let key = u64::from(expected_seq);
        let mut buf = self.choice_accepted.lock().unwrap();
        loop {
            if let Some(entry) = buf.log.get(key) {
                let result = entry.library_search_result;
                if buf.cursor < key {
                    buf.cursor = key;
                }
                return Some(result);
            }
            if self.terminal.load(std::sync::atomic::Ordering::Acquire) {
                return None;
            }
            buf = self.choice_accepted_notify.wait(buf).unwrap();
        }
    }

    /// Reset the choice-accepted read cursor to 0 (rewind/replay support).
    pub fn reset_choice_accepted_cursor(&self) {
        self.choice_accepted.lock().unwrap().cursor = 0;
    }

    // ─────────────────────────────────────────────────────────────────────
    // TERMINAL / EXIT SIGNALLING
    // ─────────────────────────────────────────────────────────────────────

    /// Signal that the game should exit. Sets the terminal flag (releasing
    /// every no-timeout Condvar wait), signals the local-choice MVar, and
    /// notifies the state-sync / opponent-choice / choice-accepted Condvars
    /// so any blocked waiter wakes and observes the terminal state.
    pub fn signal_exit(&self) {
        self.terminal.store(true, std::sync::atomic::Ordering::Release);
        self.local_choice_mvar.signal_exit();
        self.state_sync_notify.notify_all();
        self.opponent_choices_notify.notify_all();
        self.choice_accepted_notify.notify_all();
    }

    /// Check if exit has been signaled
    pub fn should_exit(&self) -> bool {
        self.terminal.load(std::sync::atomic::Ordering::Acquire)
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
    /// Accept invalid / unknown-issuer TLS certificates when connecting to
    /// `wss://`. Required to talk to a server fronted by a Cloudflare
    /// Origin Cert (or any private CA) WITHOUT going through CF's edge.
    /// **DO NOT enable for production play** — disables MITM protection.
    pub accept_invalid_certs: bool,
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
            accept_invalid_certs: false,
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
/// TODO(mtg-218 Phase 3): Complete late-binding architecture migration
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
        // TODO(mtg-218 Phase 3): Use CardZone::new_library_with_cards() once server sends DeckCardIdRanges
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

            // mtg-yulth: also record the definition in the shadow game's public
            // `card_definitions` map (keyed by CardName). The server reveals the
            // CardDefinition of every publicly-known card (drawn, played, and -- via
            // the mtg-253 fix -- every searchable library card) BEFORE any choice
            // that depends on it, so this map ends up containing exactly the cards a
            // controller may legally reason about by name. Info-independent AI
            // decisions (e.g. heuristic library search via
            // choose_from_library_by_names) can then look up CardDefinitions by name
            // and get the SAME data the full-info server has, instead of an empty map.
            {
                let key = crate::core::CardName::from(card.name.as_str());
                let defs = std::sync::Arc::make_mut(&mut self.game.card_definitions);
                defs.entry(key).or_insert_with(|| card_def.clone());
            }

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
    /// Library reorders received during wait_for_game_start (before run_game task spawns)
    /// These are transferred to the run_game spawn_blocking closure directly
    pending_library_reorders: Vec<PendingLibraryReorder>,
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
            pending_library_reorders: Vec::new(),
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

        // Build WebSocket URL. Accept a bare `host:port` (legacy default
        // — gets `ws://` prepended) OR a fully-qualified WebSocket URL
        // starting with `ws://` / `wss://`. The TLS variant is needed
        // to talk to the deployed `mtg server-web` behind HTTPS at
        // `wss://deepscry.net:8080/lobby`. `tokio_tungstenite` already
        // handles TLS via its `rustls-tls-webpki-roots` feature; we
        // just need to pass the full URL through, plus install the
        // rustls CryptoProvider (rustls 0.23 requires explicit selection;
        // `install_default` errors harmlessly if already installed).
        let server = &self.config.server;
        let url = if server.starts_with("ws://") || server.starts_with("wss://") {
            if server.starts_with("wss://") {
                let _ = rustls::crypto::ring::default_provider().install_default();
            }
            server.clone()
        } else {
            format!("ws://{server}")
        };
        log::info!("Connecting to {}...", url);

        // Connect. For `wss://` with `accept_invalid_certs=true` we
        // build a custom rustls connector that trusts any cert (used
        // by the deploy probes and dev tools that hit a server fronted
        // by a Cloudflare Origin Cert directly, where the CA is
        // private). All other code paths take the default connector
        // (proper webpki-roots verification).
        let (ws, _response) = if url.starts_with("wss://") && self.config.accept_invalid_certs {
            use tokio_tungstenite::Connector;
            log::warn!("[client] accepting invalid TLS certificates (insecure — for dev probes only)");
            let tls = build_insecure_rustls_config();
            tokio_tungstenite::connect_async_tls_with_config(
                &url,
                None,
                false,
                Some(Connector::Rustls(std::sync::Arc::new(tls))),
            )
            .await?
        } else {
            connect_async(&url).await?
        };
        self.ws = Some(ws);

        // Send authentication
        let auth_msg = ClientMessage::Authenticate {
            password: self.config.password.clone(),
            player_name: self.config.player_name.clone(),
            deck: deck_to_submission(&deck),
        };
        self.send_message(&auth_msg).await?;

        // Wait for auth result.
        //
        // Post-server-lobby (PR #9): the legacy `Authenticate` request now
        // routes through the lobby. The CREATOR (first authenticator for a
        // given default-lobby slot) gets `GameCreated`; the JOINER (second
        // authenticator) gets the legacy `AuthResult { success: true }`.
        // We accept either as a successful authentication.
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
            ServerMessage::GameCreated {
                game_name,
                your_player_id,
                your_name,
            } => {
                // Lobby creator path. Treat as a successful auth: update our
                // player name from the server assignment if applicable.
                if let Some(assigned_name) = your_name {
                    self.config.player_name = Some(assigned_name.clone());
                    log::info!(
                        "Authenticated as '{}' (player {:?}) — created lobby game '{}'",
                        assigned_name,
                        your_player_id,
                        game_name
                    );
                } else {
                    log::info!(
                        "Authenticated as player {:?} — created lobby game '{}'",
                        your_player_id,
                        game_name
                    );
                }
            }
            _ => {
                return Err(anyhow!("Unexpected response: expected AuthResult or GameCreated"));
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
                    reconnect_token: _, // stored by the client UI layer, not the game loop
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
            let mut game = initializer.init_game_reserve_only(p1_name, p2_name, starting_life, ranges);
            // mtg-yulth: populate the shadow game's public card_definitions map from
            // BOTH (public) deck lists, identically to the server's
            // init_game_with_positional_ids. Card *definitions* are public data;
            // without this the shadow map is empty and name-based controller
            // decisions (heuristic library search) diverge from the full-info
            // server, breaking the information-independence invariant
            // (docs/NETWORK_ARCHITECTURE.md).
            initializer
                .populate_card_definitions(&mut game, p1_deck, p2_deck)
                .await?;
            game
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
                ServerMessage::CardRevealed {
                    owner,
                    card,
                    reason,
                    action_count: _, // mtg-610: native opening-hand path ignores the stamp
                } => {
                    // HIDDEN INFO ARCHITECTURE (mtg-218):
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
                ServerMessage::LibraryReordered { player, new_order, .. } => {
                    // CRITICAL: Server sends LibraryReordered during opening hands phase.
                    // We must capture these and apply them in run_game before GameLoop starts.
                    // Without this, the client has wrong library order and draws wrong cards.
                    log::info!(
                        "Captured LibraryReordered for {:?} ({} cards) during wait_for_game_start",
                        player,
                        new_order.len()
                    );
                    self.pending_library_reorders
                        .push(PendingLibraryReorder { player, new_order });
                }
                _ => {
                    log::debug!("Unexpected message while waiting for opening reveals: {:?}", msg);
                }
            }
        }

        log::info!(
            "Received {} opening hand reveals, {} library reorders queued, shadow state ready",
            reveals_received,
            self.pending_library_reorders.len()
        );
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

        // Take pending library reorders captured during wait_for_game_start
        // These MUST be applied before the game loop starts since they were received
        // before the WS reader task was spawned.
        let pending_library_reorders = std::mem::take(&mut self.pending_library_reorders);

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

        // Clone for sync callback, controllers, and initial-reorder seeding
        let sync_state = shared_state.clone();
        let controller_state = shared_state.clone();
        let seed_state = shared_state.clone();
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
                // Phase 2 step 3a: non-destructively apply every unapplied
                // state-sync entry (reorders BEFORE reveals, mtg-589) up to
                // the current frontier. Replaces the legacy
                // drain_all_library_reorders + drain_all_reveals two-step;
                // the log is keyed by action_count so arrival races no longer
                // hand the wrong subset to a given sync point, and reads are
                // non-destructive (enabling rewind/replay).
                let applied = sync_state.apply_state_sync_up_to_frontier(game, &card_db_for_sync, our_player_id);
                if applied > 0 {
                    log::debug!(
                        "sync_callback: applied {} state-sync entries (game_action={})",
                        applied,
                        game.undo_log.len()
                    );
                }
            };

            // IMMEDIATE LIBRARY REORDER: Fold the initial library reorders
            // (captured during wait_for_game_start, BEFORE the WS reader task
            // spawned) into the SAME state-sync log at their action_count, so
            // the first sync_callback applies them before any draw. This
            // unifies the initial-reorder path with the in-game path on one
            // log (no separate pre-loop application).
            if !pending_library_reorders.is_empty() {
                log::info!(
                    "run_game: seeding {} initial library reorders into state-sync log BEFORE GameLoop starts",
                    pending_library_reorders.len()
                );
                for reorder in pending_library_reorders {
                    log::debug!(
                        "run_game: seeding library reorder for {:?}, {} cards",
                        reorder.player,
                        reorder.new_order.len()
                    );
                    seed_state.push_library_reorder(reorder.player, reorder.new_order);
                }
            } else {
                log::warn!("run_game: no pending library reorders - library order may be wrong!");
            }

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

        // The server is authoritative about the winner (see
        // docs/NETWORK_ARCHITECTURE.md). Both clients receive the SAME
        // `GameEnded { winner }`, so prefer that value over any locally-derived
        // result. This guarantees both clients agree even though their
        // GameLoops exit via different paths (natural game-end on one client vs
        // `ExitGame`-from-channel-close on the other).
        //
        // IMPORTANT: await the GameEnded message (set by the still-running
        // reader task) BEFORE aborting the reader. The client whose GameLoop
        // returns first can otherwise read `server_winner` as `None` and report
        // a different winner than its peer — the flaky "Clients disagree on
        // winner: Some(1) vs None" failure. The timeout is a safety guard only;
        // in a correct game both clients receive GameEnded promptly.
        let server_winner = shared_state
            .wait_for_server_winner(std::time::Duration::from_secs(10))
            .await;

        // Clean up tasks
        reader_handle.abort();
        writer_handle.abort();

        // Return result - handle both JoinError and game error
        match game_result {
            Ok(Ok(result)) => {
                log::info!(
                    "Client GameLoop finished: local_winner={:?} server_winner={:?}, action_count={}",
                    result.winner,
                    server_winner,
                    result.action_count
                );
                // Prefer the server's authoritative verdict; fall back to the
                // locally-derived winner only if no GameEnded was observed.
                Ok(server_winner.unwrap_or(result.winner))
            }
            Ok(Err(e)) => {
                let error_msg = e.to_string();
                if error_msg.contains("Game exit requested") {
                    // Game ended normally via controller ExitGame. The winner
                    // comes from the server's GameEnded message.
                    log::info!("Game ended via ExitGame, server_winner={:?}", server_winner);
                    Ok(server_winner.flatten())
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
                                // Append to the shadow state-sync log (Phase 2 step 3a).
                                log::debug!(
                                    "WsReaderShared: buffering reveal {} (id={}) for {:?}",
                                    card.name,
                                    card.card_id.as_u32(),
                                    owner
                                );
                                shared_state.push_reveal(owner, card, reason);
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
                                // Append to the local choice-accepted buffer
                                // (Phase 2 step 3c), keyed by choice_seq.
                                shared_state.push_choice_accepted(choice_seq, library_search_result);
                            }
                            NetworkMessage::OpponentChoice {
                                action_count,
                                choice_seq,
                                choice_indices,
                                description,
                                spell_ability,
                                library_search_result,
                                target_card_ids,
                            } => {
                                // Update tracked action count (for sync targeting)
                                shared_state.update_server_action_count(action_count);
                                // Append to the per-controller opponent-choice
                                // buffer (Phase 2 step 3b), keyed by choice_seq.
                                log::debug!(
                                    "WsReaderShared: OpponentChoice seq={} indices={:?} action={} lib_search={:?} targets={:?} -> opponent_choices",
                                    choice_seq,
                                    choice_indices,
                                    action_count,
                                    library_search_result,
                                    target_card_ids
                                );
                                shared_state.push_opponent_choice(ChoiceEntry {
                                    choice_seq,
                                    action_count,
                                    choice_indices,
                                    description,
                                    spell_ability,
                                    library_search_result,
                                    target_card_ids,
                                });
                            }
                            NetworkMessage::GameEnded { winner, action_count } => {
                                log::info!(
                                    "WsReaderShared: Game ended, winner={:?}, action={}",
                                    winner,
                                    action_count
                                );
                                // Record server-authoritative winner BEFORE
                                // signalling exit, so run_game can report the
                                // same verdict on both clients regardless of
                                // which controller exit path fires first.
                                shared_state.set_server_winner(winner);
                                // signal_exit() sets the terminal flag and
                                // notifies the state-sync / opponent-choice /
                                // choice-accepted Condvars so any blocked
                                // waiter wakes and observes the terminal state.
                                // The local ChoiceRequest path is still on the
                                // MVar, so push its Exit explicitly.
                                shared_state.signal_exit();
                                shared_state.push_local_choice(LocalChoiceInfo::Exit { winner });
                                return;
                            }
                            NetworkMessage::Error { message, fatal } => {
                                if fatal {
                                    log::error!("WsReaderShared: Fatal error: {}", message);
                                    // Terminal: release all waiters (terminal
                                    // flag + Condvar notify) and push the
                                    // local-choice MVar's Error explicitly.
                                    shared_state.signal_exit();
                                    shared_state.push_local_choice(LocalChoiceInfo::Error { message });
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
                                // Queue for sync_callback to apply
                                shared_state.push_library_reorder(player, new_order);
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
                return;
            }
            Ok(_) => {
                // Ignore binary/ping/pong
            }
            Err(e) => {
                log::error!("WsReaderShared: WebSocket error: {}", e);
                shared_state.signal_exit();
                shared_state.push_local_choice(LocalChoiceInfo::Error { message: e.to_string() });
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

/// Build a rustls `ClientConfig` that accepts ANY certificate without
/// verification. Used by `mtg connect --accept-invalid-certs wss://...`
/// against a server using a Cloudflare Origin Cert (issued by a CF
/// private CA that webpki-roots doesn't trust) when the server is
/// reached directly (not via CF's edge).
///
/// **Security:** this disables MITM protection completely. The flag
/// guarding the code path is intentionally verbose (`accept_invalid_certs`)
/// and a `log::warn!` fires whenever it's used. Never enable for
/// production game traffic.
fn build_insecure_rustls_config() -> rustls::ClientConfig {
    use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
    use rustls::crypto::{verify_tls12_signature, verify_tls13_signature};
    use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
    use rustls::{DigitallySignedStruct, Error as RustlsError, SignatureScheme};

    #[derive(Debug)]
    struct NoVerifier(rustls::crypto::CryptoProvider);

    impl ServerCertVerifier for NoVerifier {
        fn verify_server_cert(
            &self,
            _end_entity: &CertificateDer<'_>,
            _intermediates: &[CertificateDer<'_>],
            _server_name: &ServerName<'_>,
            _ocsp_response: &[u8],
            _now: UnixTime,
        ) -> Result<ServerCertVerified, RustlsError> {
            Ok(ServerCertVerified::assertion())
        }
        fn verify_tls12_signature(
            &self,
            message: &[u8],
            cert: &CertificateDer<'_>,
            dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, RustlsError> {
            verify_tls12_signature(message, cert, dss, &self.0.signature_verification_algorithms)
        }
        fn verify_tls13_signature(
            &self,
            message: &[u8],
            cert: &CertificateDer<'_>,
            dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, RustlsError> {
            verify_tls13_signature(message, cert, dss, &self.0.signature_verification_algorithms)
        }
        fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
            self.0.signature_verification_algorithms.supported_schemes()
        }
    }

    // Make sure a CryptoProvider is installed (idempotent).
    let _ = rustls::crypto::ring::default_provider().install_default();
    let provider = rustls::crypto::CryptoProvider::get_default()
        .expect("CryptoProvider must be installed")
        .clone();

    rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .expect("default protocol versions")
        .dangerous()
        .with_custom_certificate_verifier(std::sync::Arc::new(NoVerifier((*provider).clone())))
        .with_no_client_auth()
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
            commanders: Vec::new(),
        };

        let submission = deck_to_submission(&deck);
        assert_eq!(submission.main_deck_size(), 24);
        assert_eq!(submission.sideboard_size(), 2);
    }
}
