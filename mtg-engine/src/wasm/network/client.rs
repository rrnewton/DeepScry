//! WASM Network Client State Machine
//!
//! Manages connection state and message queues for browser-based network gameplay.
//! Unlike the native client which blocks on channels, this uses queues that
//! JavaScript can fill from WebSocket callbacks.

use crate::core::{PlayerId, SpellAbility};
use crate::network::{
    state_sync_entries_equivalent, ActionLog, BufferedFact, ChoiceEntry, ChoiceType, ClientMessage, DeckSubmission,
    ServerMessage, StateSyncEntry,
};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

/// Connection state for the WASM network client
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkState {
    /// Not connected to any server
    Disconnected,
    /// WebSocket connection established, awaiting authentication
    Connecting,
    /// Authenticated, waiting for opponent to join
    WaitingForOpponent,
    /// Game is in progress
    InGame,
    /// Game has ended
    GameEnded,
    /// Error state
    Error,
}

impl NetworkState {
    /// Get a string representation for JavaScript
    pub fn as_str(&self) -> &'static str {
        match self {
            NetworkState::Disconnected => "disconnected",
            NetworkState::Connecting => "connecting",
            NetworkState::WaitingForOpponent => "waiting_for_opponent",
            NetworkState::InGame => "in_game",
            NetworkState::GameEnded => "game_ended",
            NetworkState::Error => "error",
        }
    }
}

/// Data from an OpponentChoice message.
///
/// Carry-on view returned by [`WasmNetworkClient::pop_opponent_choice`] and
/// [`WasmNetworkClient::peek_opponent_choice`]. The underlying storage in
/// Phase 2 step 2 is now an `ActionLog<ChoiceEntry>` keyed by
/// `action_count`; this struct materialises the index + payload back into
/// the single shape `WasmRemoteController` and the SMART damage-assignment
/// overrides already consume. Keeping the shape stable across the refactor
/// localises the change to the storage layer.
#[derive(Debug, Clone)]
pub struct OpponentChoiceData {
    pub choice_seq: u32,
    pub choice_indices: Vec<usize>,
    pub description: String,
    pub spell_ability: Option<SpellAbility>,
    /// Server-reported `action_count` (= the `ActionLog<ChoiceEntry>` key
    /// in the new storage). Preserved for callers that still want to log
    /// or diff it.
    pub action_count: u64,
    /// For LibrarySearchByName choices: the specific CardId that was chosen.
    /// Allows the shadow game's WasmRemoteController to know which card moved to hand.
    pub library_search_result: Option<crate::core::CardId>,
    /// Authoritative target CardIds for choices that need them (e.g., damage
    /// assignment among multiple blockers). Used by `WasmRemoteController` to
    /// pick the correct blocker even when the index would point to a different
    /// CardId in the client's view than in the server's view.
    pub target_card_ids: Option<Vec<crate::core::CardId>>,
}

impl OpponentChoiceData {
    /// Materialise the `ActionLog`-stored `(choice_seq, ChoiceEntry)`
    /// pair back into the legacy `OpponentChoiceData` shape that
    /// `WasmRemoteController` (and the SMART damage-assignment overrides)
    /// consume. Cheap clone of the structured payload — the same allocation
    /// the old `VecDeque` consumed when it `push_back`ed.
    ///
    /// The log key is `choice_seq` (mtg-sfihb), but `action_count` is still
    /// surfaced for diagnostics from the payload field.
    fn from_log_entry(entry: &ChoiceEntry) -> Self {
        Self {
            choice_seq: entry.choice_seq,
            choice_indices: entry.choice_indices.clone(),
            description: entry.description.clone(),
            spell_ability: entry.spell_ability.clone(),
            action_count: entry.action_count,
            library_search_result: entry.library_search_result,
            target_card_ids: entry.target_card_ids.clone(),
        }
    }
}

/// Data from a ChoiceRequest message
#[derive(Debug, Clone)]
pub struct ChoiceRequestData {
    pub choice_seq: u32,
    pub choice_type: ChoiceType,
    pub options: Vec<String>,
    pub state_hash: u64,
    pub action_count: u64,
    /// Server's authoritative list of available abilities for Priority choices.
    /// Index 0 is "Pass priority" (None), indices 1+ are the actual abilities.
    /// Used for DESYNC DETECTION: local abilities are validated against these.
    /// Any mismatch is a FATAL error (per NETWORK_ARCHITECTURE.md).
    pub abilities: Option<Vec<Option<SpellAbility>>>,
}

/// WASM Network Client
///
/// Manages connection state and provides non-blocking access to server messages.
/// JavaScript fills the incoming queues via WebSocket callbacks, and polls
/// the outgoing queue to send messages.
pub struct WasmNetworkClient {
    /// Current connection state
    state: NetworkState,

    /// Our player ID (set after GameStarted)
    our_player_id: Option<PlayerId>,

    /// Opponent's player ID
    opponent_id: Option<PlayerId>,

    /// Opponent's name
    opponent_name: Option<String>,

    /// Whether network debug mode is enabled
    network_debug: bool,

    /// Append-only, **game-`action_count`-indexed** shadow LIBRARY-REORDER log
    /// (mtg-o99ow L3 + the reorder/reveal SPLIT).
    ///
    /// **SPLIT from the reveal log (mtg-o99ow, mirrors the native client).** A
    /// `LibraryReorder` and a `RevealCard` can legitimately carry the SAME game
    /// `action_count`: the server stamps a shuffle's reorder at the post-shuffle
    /// undo position (`state.rs` shuffle path, `undo_log.len()` after the
    /// `ShuffleLibrary`) while reveals are stamped at their OWN undo index
    /// (`forward_idx` in `collect_reveals_since_last_choice`), two numbering
    /// schemes that COINCIDE on a shuffle-then-draw resolution (Timetwister /
    /// Wheel of Fortune / Windfall). They are INDEPENDENT deltas (different
    /// player / zone) that both must apply (reorder first, then reveal). Keeping
    /// them in ONE ac-keyed log made the second arrival look like "two distinct
    /// deltas share an ac" and (correctly, for SAME-class deltas) PANIC as fatal
    /// — which would crash the WASM shadow on any shuffle-then-draw card. Two
    /// logs restore the real invariant: distinct ac per SAME-class delta; a
    /// reorder and a reveal MAY share an ac. Each log keeps `insert_sorted`
    /// (out-of-order tolerant, dedups idempotent same-delta re-sends, fatal on a
    /// DIFFERING same-class delta at one ac).
    ///
    /// Backs the reveal-as-choice unification described in
    /// `docs/NETWORK_ACTION_LOG.md` § 3.2 and
    /// `ai_docs/REVEAL_ACTIONLOG_UNIFICATION_DESIGN_20260603.md`. The WS
    /// receive handler / `apply_choice_buffer` pushes each server-authoritative
    /// delta at the **game `action_count` the server stamped on the wire** — the
    /// undo-log position of the delta's own action. The key IS the game position,
    /// so the shadow consumes each delta at the SAME game `action_count` on the
    /// forward pass and on every rewind/replay — no synthetic arrival counter, no
    /// effective-ac side map.
    ///
    /// Replaces the legacy `pending_reveals` / `pending_library_reorders`
    /// VecDeques + `drain_*` helpers and the interim synthetic-key +
    /// effective-ac machinery (mtg-610).
    reorder_log: ActionLog<StateSyncEntry>,

    /// Append-only, **game-`action_count`-indexed** shadow REVEAL log
    /// (`RevealCard` + `SearchCandidates`). See `reorder_log` for why the two are
    /// split. The N library-search candidates revealed by a single search are
    /// folded into one [`StateSyncEntry::SearchCandidates`] carrying
    /// `Vec<CardReveal>` at the one search ac (N separate `RevealCard` at one ac
    /// would be a same-class collision). Mirrors the native client's `reveal_log`.
    reveal_log: ActionLog<StateSyncEntry>,

    /// Cursor for **LibraryReorder** entries — POSITIONAL: a reorder at game ac
    /// K is applied only once the shadow's own `action_count` has reached K
    /// (bounded by `target_action`). Load-bearing: a shuffle's order must NOT
    /// overwrite the shadow library before the shadow has replayed earlier-ac
    /// actions that read it (e.g. an opponent's cycling fetch that pulls a card
    /// OUT of the pre-shuffle library — apply the post-shuffle order early and the
    /// fetched card vanishes → the search finds nothing → desync). Mirrors the
    /// native `last_applied_reorder_ac`.
    last_applied_reorder_ac: u64,

    /// Cursor for **RevealCard / SearchCandidates** entries — EAGER: applied up
    /// to the reveal-history-complete watermark (`max_received_choice_ac`), AHEAD
    /// of the shadow's own `action_count`. The shadow replays the OPPONENT's
    /// actions (via `WasmRemoteController`) at acs beyond its own position, and
    /// those plays need their card identities revealed BEFORE the shadow can
    /// replay them (else "Entity not found"). Reveals are identity injections
    /// (library-order independent), so applying them early is safe.
    ///
    /// **This eager bound FIXES the apply-frontier stall (mtg-o99ow B2):** the
    /// old single cursor bounded reveals by `target_action` too, so the shadow's
    /// forward replay stalled at the last reveal (e.g. server=55 / local=50,
    /// diff=5) instead of advancing to the `ChoiceRequest`'s `action_count` —
    /// because a buffered opponent choice at a higher ac could not be consumed
    /// until its bundled reveals were applied. Mirrors the native
    /// `last_applied_reveal_ac`.
    last_applied_reveal_ac: u64,

    /// **Game-start library orders (mtg-o99ow L3).** The server syncs each
    /// player's initial (pre-game) shuffled library order via
    /// `ServerMessage::LibraryReordered { action_count: 0, .. }` — two per
    /// client (own + opponent). These predate the empty undo log, so they have
    /// no real action position and BOTH would collide at ac 0 in the
    /// strictly-increasing `state_sync` log. They are not in-game deltas; they
    /// establish the shadow's starting library state. We therefore hold them
    /// here keyed by player (latest wins) and apply them at the very first
    /// sync point (`target_action == 0`, before the `skip_opening_hands`
    /// GameLoop draws) and re-apply on every rewind (the undo rewind restores
    /// public state but not the hidden library order). Mid-game reorders
    /// (`action_count > 0`) go into the ac-keyed `state_sync` log instead.
    initial_library_orders: std::collections::BTreeMap<PlayerId, Vec<crate::core::CardId>>,

    /// One-shot guard: have the [`initial_library_orders`] been written to the
    /// shadow yet? Applied exactly once, at the first sync point before the
    /// `skip_opening_hands` draws. NOT cleared by a cursor reset (a rewind
    /// restores the library order through the undo log + re-applied in-game
    /// reorders, not by re-wiping to the pre-game order); cleared only on a full
    /// `reset` for a new game.
    initial_library_applied: bool,

    /// Highest game `action_count` carried by any `ChoiceRequest` /
    /// `OpponentChoice` the shadow has RECEIVED (mtg-610).
    ///
    /// The server sends a choice's bundled `CardRevealed` messages BEFORE the
    /// choice itself, in order on the same connection. So once a choice at
    /// action_count A has been received, EVERY reveal with effective_ac ≤ A has
    /// also arrived. The rewind/replay verifier uses this as its
    /// "reveal-history is complete up to R" signal: it only caches/compares a
    /// turn-start hash for a rewind to R once `max_received_choice_ac ≥ R`,
    /// otherwise the shadow has raced ahead of the in-flight reveal stream (it
    /// advances by replaying its own recorded choices) and the materialised set
    /// would be incomplete — the source of the `cards[id]` PRIOR-null /
    /// CURRENT-present turn-start drift.
    max_received_choice_ac: u64,

    /// Append-only, `choice_seq`-indexed per-controller choice buffer for
    /// opponent (remote) choices.
    ///
    /// Backs the Phase 2 step 2 migration described in
    /// `docs/NETWORK_ACTION_LOG.md` § 3.1 / `NETWORK_ACTION_LOG_MIGRATION.md`
    /// § 2.1. The WS receive handler pushes `ServerMessage::OpponentChoice`
    /// here keyed by the server-reported **`choice_seq`** (NOT `action_count`).
    ///
    /// `choice_seq` is the only strictly-unique, strictly-monotonic per-choice
    /// key: the server bumps it exactly once per `ChoiceRequest`. `action_count`
    /// (= `undo_log.len()`) is NOT unique per choice — during multi-step combat
    /// damage assignment the server emits two choices
    /// (`choose_blocker_for_lethal_damage` then
    /// `choose_blocker_for_remaining_damage` for the same attacker) with no
    /// undoable action between them, so both carry the same `action_count`.
    /// Keying by `action_count` made the second `ActionLog::push` panic with
    /// "action_count must be strictly increasing" (mtg-sfihb). Keying by
    /// `choice_seq` is correct by construction.
    ///
    /// Reads are non-destructive: a per-client cursor
    /// (`next_opponent_choice_cursor`) records how far the controller has
    /// consumed, and a snapshot rewind resets the cursor without touching
    /// the log itself (invariant #3 of `docs/NETWORK_ACTION_LOG.md` § 8).
    ///
    /// Replaces the legacy `VecDeque<OpponentChoiceData>` queue and its
    /// destructive `pop_front` semantics. The structured payload is
    /// `ChoiceEntry`. See the "STATE-SYNC LOG" parallel for the same shape
    /// applied to reveals / reorders in step 1.
    pub opponent_choices: ActionLog<ChoiceEntry>,

    /// Read cursor into `opponent_choices`. The next `pop_opponent_choice`
    /// returns the first entry whose `choice_seq > next_opponent_choice_cursor`;
    /// the cursor then advances to that entry's `choice_seq`.
    ///
    /// This preserves the existing FIFO consume semantics that
    /// `WasmRemoteController` relies on while the underlying storage moves
    /// to append-only / non-destructive. A future step (the per-controller
    /// `choose_at(view, ac)` trait method described in
    /// `docs/NETWORK_ACTION_LOG.md` § 3.1) lets the engine ask for the
    /// entry at a specific `action_count`, bypassing this cursor entirely.
    next_opponent_choice_cursor: u64,

    /// **Minimal lazy protocol (mtg-o99ow), Phase 1.** Set true the first time a
    /// `ChoiceRequest` arrives. Before the first choice the server's eager
    /// `CardRevealed` / `LibraryReordered` messages carry the opening hand +
    /// initial library orders, which are NOT part of any choice buffer — so we
    /// still process them. AFTER the first `ChoiceRequest`, the per-choice
    /// `buffer` is the AUTHORITATIVE source of every mid-game reveal / reorder /
    /// search / opponent-choice fact, and the still-sent (dual-emit) eager
    /// copies are ignored — this is what makes the buffer alone drive the WASM
    /// shadow and eliminates the eager opponent-cast dual-stamp (the same reveal
    /// arriving at the choice ac AND its own ac).
    buffer_is_authoritative: bool,

    /// Current ChoiceRequest from server (if any)
    current_choice_request: Option<ChoiceRequestData>,

    /// Whether the last SubmitChoice was acknowledged
    choice_acknowledged: bool,

    /// Last submitted choice sequence number (for duplicate detection)
    /// This is stored in the client so it persists across controller instances
    last_submitted_choice_seq: Option<u32>,

    /// Outbound message queue (JavaScript polls and sends)
    outbound_queue: VecDeque<String>,

    /// Last error message
    last_error: Option<String>,

    /// Game winner (if game ended)
    winner: Option<Option<PlayerId>>,

    // Game initialization data (from GameStarted message)
    /// Starting life total
    starting_life: i32,
    /// Initial state hash for verification
    initial_state_hash: u64,
    /// Our library size after drawing
    library_size: usize,
    /// Opponent's library size
    opponent_library_size: usize,
    /// Opponent's hand count
    opponent_hand_count: usize,
    /// CardID ranges for late-binding architecture (mtg-254)
    /// CRITICAL: WASM must use these ranges to initialize game state
    /// with the same CardIDs as the server for behavioral identity.
    deck_card_ids: Option<crate::network::DeckCardIdRanges>,
    /// Serialized RNG state for deterministic shuffles
    /// Must be deserialized and used to seed the game RNG for identical behavior.
    rng_state: Vec<u8>,
    /// Token definitions that may be created during the game
    token_definitions: std::collections::HashMap<String, crate::loader::CardDefinition>,

    // Connection parameters (set before connecting)
    /// Server URL
    server_url: Option<String>,
    /// Password for authentication
    password: Option<String>,
    /// Player name
    player_name: Option<String>,
    /// Deck JSON for submission
    deck_json: Option<String>,
    /// Lobby action to dispatch on WebSocket open (mtg-474).
    ///
    /// `None` (default) → preserve legacy behaviour: auto-send `Authenticate`
    /// against the well-known `DEFAULT_LOBBY_GAME` slot. The first authenticator
    /// becomes its creator, the second joins.
    ///
    /// `Some(LobbyAction::Create { .. })` → on open, send `CreateGame` for the
    /// named slot. Used by the landing-page lobby (web/index.html) when the
    /// user clicks "Create" and the page redirects to tui_game.html with
    /// `?lobby_create=...` query params.
    ///
    /// `Some(LobbyAction::Join { .. })` → on open, send `JoinGame` for the
    /// named slot. Used by the redirect from the lobby's "Join" button.
    lobby_action: Option<LobbyAction>,
}

/// Per-connection lobby action queued for the WebSocket-open callback.
///
/// Encapsulates the choice between `CreateGame` and `JoinGame` so the WASM
/// client can be driven from a `?lobby_create=` / `?lobby_join=` redirect
/// without growing a new wasm-bindgen flag per field.
#[derive(Debug, Clone)]
pub enum LobbyAction {
    /// Send `ClientMessage::CreateGame` on WS open.
    Create {
        /// Game slot name (must be unique among waiting games).
        game_name: String,
        /// Optional per-game password.
        game_password: Option<String>,
    },
    /// Send `ClientMessage::JoinGame` on WS open.
    Join {
        /// Existing slot to join (case-insensitive on the server).
        game_name: String,
        /// Per-game password if the creator set one.
        game_password: Option<String>,
    },
}

impl WasmNetworkClient {
    /// Create a new WASM network client
    pub fn new() -> Self {
        Self {
            state: NetworkState::Disconnected,
            our_player_id: None,
            opponent_id: None,
            opponent_name: None,
            network_debug: false,
            reorder_log: ActionLog::new(),
            reveal_log: ActionLog::new(),
            last_applied_reorder_ac: 0,
            last_applied_reveal_ac: 0,
            initial_library_orders: std::collections::BTreeMap::new(),
            initial_library_applied: false,
            max_received_choice_ac: 0,
            opponent_choices: ActionLog::new(),
            next_opponent_choice_cursor: 0,
            buffer_is_authoritative: false,
            current_choice_request: None,
            choice_acknowledged: true, // Start acknowledged (no pending)
            last_submitted_choice_seq: None,
            outbound_queue: VecDeque::new(),
            last_error: None,
            winner: None,
            starting_life: 20, // Default, updated by GameStarted
            initial_state_hash: 0,
            library_size: 0,
            opponent_library_size: 0,
            opponent_hand_count: 0,
            deck_card_ids: None,
            rng_state: Vec::new(),
            token_definitions: std::collections::HashMap::new(),
            server_url: None,
            password: None,
            player_name: None,
            deck_json: None,
            lobby_action: None,
        }
    }

    /// Configure the lobby action to dispatch on the next WebSocket `on_open`.
    ///
    /// Call this BEFORE the WebSocket opens. `None` reverts to the legacy
    /// `Authenticate` behaviour. See [`LobbyAction`] for variants.
    pub fn set_lobby_action(&mut self, action: Option<LobbyAction>) {
        self.lobby_action = action;
    }

    /// Get current connection state
    pub fn state(&self) -> NetworkState {
        self.state
    }

    /// Get our player ID (if assigned)
    pub fn our_player_id(&self) -> Option<PlayerId> {
        self.our_player_id
    }

    /// Get opponent's player ID (if known)
    pub fn opponent_id(&self) -> Option<PlayerId> {
        self.opponent_id
    }

    /// Get opponent's name (if known)
    pub fn opponent_name(&self) -> Option<&str> {
        self.opponent_name.as_deref()
    }

    /// Get our player name (as sent to server during authentication)
    pub fn our_name(&self) -> Option<&str> {
        self.player_name.as_deref()
    }

    /// Get the last error message
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    /// Get the game winner (if game ended)
    pub fn winner(&self) -> Option<Option<PlayerId>> {
        self.winner
    }

    /// Check if network debug mode is enabled
    pub fn is_network_debug(&self) -> bool {
        self.network_debug
    }

    /// Get the starting life total from GameStarted
    pub fn starting_life(&self) -> i32 {
        self.starting_life
    }

    /// Get the initial state hash from GameStarted
    pub fn initial_state_hash(&self) -> u64 {
        self.initial_state_hash
    }

    /// Get our library size after drawing (from GameStarted)
    pub fn library_size(&self) -> usize {
        self.library_size
    }

    /// Get opponent's library size (from GameStarted)
    pub fn opponent_library_size(&self) -> usize {
        self.opponent_library_size
    }

    /// Get opponent's hand count (from GameStarted)
    pub fn opponent_hand_count(&self) -> usize {
        self.opponent_hand_count
    }

    /// Get CardID ranges for late-binding architecture (mtg-254)
    ///
    /// CRITICAL: WASM must use these ranges to initialize game state
    /// with `init_game_reserve_only()` for behavioral identity with native.
    pub fn deck_card_ids(&self) -> Option<&crate::network::DeckCardIdRanges> {
        self.deck_card_ids.as_ref()
    }

    /// Get serialized RNG state for deterministic shuffles
    ///
    /// Must be deserialized and used to seed the game RNG for identical
    /// shuffle behavior between server and client.
    pub fn rng_state(&self) -> &[u8] {
        &self.rng_state
    }

    /// Get token definitions for token creation
    pub fn token_definitions(&self) -> &std::collections::HashMap<String, crate::loader::CardDefinition> {
        &self.token_definitions
    }

    /// Set connection parameters before connecting
    pub fn set_connection_params(&mut self, server_url: &str, password: &str, player_name: &str, deck_json: &str) {
        self.server_url = Some(server_url.to_string());
        self.password = Some(password.to_string());
        self.player_name = Some(player_name.to_string());
        self.deck_json = Some(deck_json.to_string());
        log::info!("WasmNetworkClient: Connection params set for {}", player_name);
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // CONNECTION LIFECYCLE
    // ═══════════════════════════════════════════════════════════════════════════

    /// Called when WebSocket connection opens
    ///
    /// Automatically sends authentication if connection params are set.
    pub fn on_open(&mut self) {
        log::info!("WasmNetworkClient: WebSocket connected");
        self.state = NetworkState::Connecting;

        // Auto-dispatch the appropriate first message if we have stored params.
        // `lobby_action` (mtg-474) selects between legacy Authenticate vs the
        // explicit CreateGame / JoinGame paths used by the post-lobby redirect.
        let (Some(password), Some(player_name), Some(deck_json)) =
            (self.password.clone(), self.player_name.clone(), self.deck_json.clone())
        else {
            return;
        };
        let deck = match serde_json::from_str::<DeckSubmission>(&deck_json) {
            Ok(d) => d,
            Err(e) => {
                log::error!("WasmNetworkClient: Failed to parse deck JSON: {}", e);
                self.last_error = Some(format!("Invalid deck JSON: {}", e));
                self.state = NetworkState::Error;
                return;
            }
        };
        match self.lobby_action.clone() {
            None => {
                self.authenticate(&password, &player_name, deck);
                log::info!("WasmNetworkClient: Auto-authenticated as {}", player_name);
            }
            Some(LobbyAction::Create {
                game_name,
                game_password,
            }) => {
                self.create_game(&password, &game_name, game_password, &player_name, deck);
                log::info!("WasmNetworkClient: Sent CreateGame '{}' as {}", game_name, player_name);
            }
            Some(LobbyAction::Join {
                game_name,
                game_password,
            }) => {
                self.join_game(&password, &game_name, game_password, &player_name, deck);
                log::info!("WasmNetworkClient: Sent JoinGame '{}' as {}", game_name, player_name);
            }
        }
    }

    /// Called when WebSocket connection closes
    pub fn on_close(&mut self) {
        log::info!("WasmNetworkClient: WebSocket closed");
        if self.state != NetworkState::GameEnded {
            self.state = NetworkState::Disconnected;
            self.last_error = Some("Connection closed".to_string());
        }
    }

    /// Called when WebSocket encounters an error
    pub fn on_error(&mut self, error: &str) {
        log::error!("WasmNetworkClient: WebSocket error: {}", error);
        self.state = NetworkState::Error;
        self.last_error = Some(error.to_string());
    }

    /// Queue authentication message
    ///
    /// If player_name is empty, server will assign a default name with suffix.
    pub fn authenticate(&mut self, password: &str, player_name: &str, deck: DeckSubmission) {
        let msg = ClientMessage::Authenticate {
            password: password.to_string(),
            player_name: if player_name.is_empty() {
                None
            } else {
                Some(player_name.to_string())
            },
            deck,
        };
        self.queue_outbound(msg);
    }

    /// Queue a `CreateGame` message — register a named pre-game slot on the
    /// server and wait for an opponent to `JoinGame` it.
    ///
    /// Used by the post-lobby redirect path (mtg-474): the landing-page
    /// lobby (web/index.html) closes its own browsing WS and redirects to
    /// `tui_game.html?lobby_create=NAME&...`. The TUI page then opens a fresh
    /// WS, calls `network_create_game`, and runs the per-game flow from there.
    /// This avoids re-using a lobby-mode WS for in-game traffic, which is
    /// what triggered the "Already authenticated / in a game" Error reply
    /// from `handle_player_websocket` in the previous design.
    pub fn create_game(
        &mut self,
        password: &str,
        game_name: &str,
        game_password: Option<String>,
        player_name: &str,
        deck: DeckSubmission,
    ) {
        let msg = ClientMessage::CreateGame {
            password: password.to_string(),
            game_name: if game_name.is_empty() {
                None
            } else {
                Some(game_name.to_string())
            },
            game_password,
            player_name: if player_name.is_empty() {
                None
            } else {
                Some(player_name.to_string())
            },
            deck,
            // The game page plays the game with legacy immediate-start; the
            // launcher's lobby socket already ran the Variant-1 waiting-room
            // rendezvous (mtg-682) and freed the slot before navigating here.
            waiting_room: false,
        };
        self.queue_outbound(msg);
    }

    /// Queue a `JoinGame` message — attach to an existing pre-game slot the
    /// lobby already advertised via `GameList`.
    pub fn join_game(
        &mut self,
        password: &str,
        game_name: &str,
        game_password: Option<String>,
        player_name: &str,
        deck: DeckSubmission,
    ) {
        let msg = ClientMessage::JoinGame {
            password: password.to_string(),
            game_name: game_name.to_string(),
            game_password,
            player_name: if player_name.is_empty() {
                None
            } else {
                Some(player_name.to_string())
            },
            deck,
            // Game-page join uses legacy immediate-start; the launcher already
            // did the Variant-1 waiting-room rendezvous (mtg-682).
            waiting_room: false,
        };
        self.queue_outbound(msg);
    }

    /// Queue disconnect message
    pub fn disconnect(&mut self) {
        let msg = ClientMessage::Disconnect;
        self.queue_outbound(msg);
        self.state = NetworkState::Disconnected;
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // MESSAGE HANDLING
    // ═══════════════════════════════════════════════════════════════════════════

    /// Process a server message (called from JavaScript after receiving)
    ///
    /// Returns true if the message was processed successfully.
    pub fn on_message(&mut self, json: &str) -> bool {
        match serde_json::from_str::<ServerMessage>(json) {
            Ok(msg) => {
                self.handle_server_message(msg);
                true
            }
            Err(e) => {
                log::error!("WasmNetworkClient: Failed to parse message: {}", e);
                self.last_error = Some(format!("Parse error: {}", e));
                false
            }
        }
    }

    /// Handle a parsed server message
    fn handle_server_message(&mut self, msg: ServerMessage) {
        match msg {
            ServerMessage::AuthResult {
                success,
                error,
                your_player_id,
                your_name,
            } => {
                if success {
                    log::info!(
                        "WasmNetworkClient: Authenticated as '{}' ({:?})",
                        your_name.as_deref().unwrap_or("<pending>"),
                        your_player_id
                    );
                    self.our_player_id = your_player_id;
                    // Store the assigned name if provided
                    // (it will be None for first player until P2 connects)
                    self.state = NetworkState::WaitingForOpponent;
                } else {
                    log::error!("WasmNetworkClient: Auth failed: {:?}", error);
                    self.state = NetworkState::Error;
                    self.last_error = error;
                }
            }

            ServerMessage::BugReportStored { success, error, .. } => {
                if !success {
                    log::error!("WasmNetworkClient: Bug report disk write failed: {:?}", error);
                    self.last_error = error;
                }
            }

            // Phase-2 GitHub issue result is surfaced to the user entirely in the
            // JS bug-report widget (web/bug_report.js); the WASM client has no
            // game-state stake in it, so there is nothing to record here.
            ServerMessage::BugReportIssueResult { .. } => {}

            ServerMessage::WaitingForOpponent => {
                log::info!("WasmNetworkClient: Waiting for opponent");
                self.state = NetworkState::WaitingForOpponent;
            }

            ServerMessage::GameStarted {
                your_player_id,
                opponent_name,
                opening_hand,
                opponent_hand_count,
                library_size,
                opponent_library_size,
                opponent_decklist: _, // Not used in WASM client
                starting_life,
                initial_state_hash,
                network_debug,
                deck_card_ids,
                token_definitions,
                rng_state,
                reconnect_token: _, // Stored by the JS layer, not the WASM game client
            } => {
                log::info!(
                    "WasmNetworkClient: Game started! We are {:?}, opponent: {}, life: {}",
                    your_player_id,
                    opponent_name,
                    starting_life
                );

                // Log critical sync data (mtg-254: behavioral identity)
                if let Some(ref ranges) = deck_card_ids {
                    log::info!(
                        "WasmNetworkClient: DeckCardIdRanges received - P1:[{}..{}), P2:[{}..{})",
                        ranges.p1_start,
                        ranges.p1_end,
                        ranges.p2_start,
                        ranges.p2_end
                    );
                } else {
                    log::warn!("WasmNetworkClient: No DeckCardIdRanges - late-binding CardIDs unavailable!");
                }
                if !rng_state.is_empty() {
                    log::info!("WasmNetworkClient: RNG state received ({} bytes)", rng_state.len());
                } else {
                    log::warn!("WasmNetworkClient: No RNG state - shuffles may diverge!");
                }
                if !token_definitions.is_empty() {
                    log::info!(
                        "WasmNetworkClient: {} token definitions received",
                        token_definitions.len()
                    );
                }

                self.our_player_id = Some(your_player_id);
                self.opponent_id = Some(if your_player_id.as_u32() == 0 {
                    PlayerId::new(1)
                } else {
                    PlayerId::new(0)
                });
                self.opponent_name = Some(opponent_name);
                self.network_debug = network_debug;
                self.starting_life = starting_life;
                self.initial_state_hash = initial_state_hash;
                self.library_size = library_size;
                self.opponent_library_size = opponent_library_size;
                self.opponent_hand_count = opponent_hand_count;
                // CRITICAL: Store late-binding architecture data (mtg-254)
                self.deck_card_ids = deck_card_ids;
                self.rng_state = rng_state;
                self.token_definitions = token_definitions;
                self.state = NetworkState::InGame;

                // NOTE: Do NOT queue opening_hand reveals here!
                // The server sends individual CardRevealed messages for opening hand cards
                // immediately after GameStarted. If we queue opening_hand here AND process
                // the CardRevealed messages, we'd double-process the same cards.
                //
                // The native client also does NOT queue opening_hand as reveals - it just
                // registers the card definitions for later reference.
                //
                // The opening_hand field is informational only (hand count, which cards we got).
                let _ = opening_hand; // Acknowledge we intentionally ignore
            }

            ServerMessage::CardRevealed {
                owner,
                card,
                reason,
                action_count,
            } => {
                // Minimal lazy protocol (mtg-o99ow): once the buffer is
                // authoritative, mid-game reveals arrive via the ChoiceRequest
                // buffer at their TRUE ac. Ignore the eager (dual-emit) copy —
                // this is what removes the eager opponent-cast dual-stamp (the
                // same card revealed at the choice ac AND its own ac). Before the
                // first choice, eager reveals carry the opening hand, so process.
                if self.buffer_is_authoritative {
                    return;
                }
                log::debug!(
                    "WasmNetworkClient: Card revealed - {} ({:?}) for {:?} ac={:?}",
                    card.name,
                    reason,
                    owner,
                    action_count
                );
                // mtg-o99ow L3: key the reveal DIRECTLY by the game `action_count`
                // the server stamped (the undo-log position of this RevealCard).
                // Every reveal now carries a real ac; a missing stamp is a server
                // desync (NETWORK_ARCHITECTURE.md "Desync is ALWAYS Fatal").
                match action_count {
                    Some(ac) => self.push_state_sync(
                        ac,
                        StateSyncEntry::RevealCard {
                            owner,
                            card: Box::new(card),
                            reason,
                        },
                    ),
                    None => {
                        log::error!(
                            "WasmNetworkClient: FATAL — CardRevealed for {:?} ({}) carried no action_count (mtg-o99ow L3 requires server-stamped game ac)",
                            owner, card.name
                        );
                        self.state = NetworkState::Error;
                        self.last_error = Some("CardRevealed without action_count (desync)".to_string());
                    }
                }
            }

            ServerMessage::ChoiceRequest {
                choice_seq,
                choice_type,
                options,
                state_hash,
                action_count,
                abilities,
                buffer,
                ..
            } => {
                log::debug!(
                    "WasmNetworkClient: ChoiceRequest seq={} type={:?} action_count={} abilities={} buffer={}",
                    choice_seq,
                    choice_type,
                    action_count,
                    abilities.as_ref().map(|a| a.len()).unwrap_or(0),
                    buffer.len()
                );
                // Minimal lazy protocol (mtg-o99ow) Phase 1: route the single
                // catch-up buffer into the state-sync + opponent-choice logs.
                // From this point on the buffer is the AUTHORITATIVE source of
                // mid-game facts; the still-sent eager copies are ignored (see
                // `buffer_is_authoritative`).
                self.buffer_is_authoritative = true;
                self.apply_choice_buffer(buffer);
                // mtg-o99ow L3: reveals/reorders are keyed by their OWN game ac,
                // so there is nothing to "stamp" at a choice. We still record the
                // highest received choice ac as the reveal-history-complete
                // watermark used by the rewind gate (all deltas with ac ≤ this
                // have arrived in this buffer, by construction).
                self.note_received_choice_ac(action_count);
                self.current_choice_request = Some(ChoiceRequestData {
                    choice_seq,
                    choice_type,
                    options,
                    state_hash,
                    action_count,
                    abilities,
                });
            }

            ServerMessage::OpponentChoice {
                choice_seq,
                choice_indices,
                description,
                spell_ability,
                action_count,
                library_search_result,
                target_card_ids,
                ..
            } => {
                // Minimal lazy protocol (mtg-o99ow): the opponent's decision now
                // rides in our next ChoiceRequest buffer as BufferedFact::Choice.
                // Ignore the eager (dual-emit) copy — routing it too would
                // double-push the same choice_seq into opponent_choices and panic
                // ActionLog's strict-monotonic key.
                if self.buffer_is_authoritative {
                    return;
                }
                log::debug!(
                    "WasmNetworkClient: OpponentChoice seq={} indices={:?} action_count={} desc={}",
                    choice_seq,
                    choice_indices,
                    action_count,
                    description
                );
                // mtg-o99ow L3: see ChoiceRequest — reveals are self-keyed by
                // game ac now; just advance the reveal-history watermark.
                self.note_received_choice_ac(action_count);
                // Phase 1-2 dual-emit: record into the per-controller choice
                // buffer keyed by `choice_seq`, deduping the eager-vs-buffer
                // duplicate (see `record_opponent_choice`). `choice_seq` (NOT
                // `action_count`) is the log key: the server bumps it once per
                // ChoiceRequest, so it is unique/monotonic per choice, whereas
                // `action_count` is NOT unique (multi-step combat damage emits
                // two choices at one action_count, mtg-sfihb).
                self.record_opponent_choice(ChoiceEntry {
                    choice_seq,
                    action_count,
                    choice_indices,
                    description,
                    spell_ability,
                    library_search_result,
                    target_card_ids,
                });
            }

            ServerMessage::ChoiceAccepted { choice_seq, .. } => {
                log::debug!("WasmNetworkClient: ChoiceAccepted seq={}", choice_seq);
                self.choice_acknowledged = true;
            }

            ServerMessage::GameEnded { winner, reason, .. } => {
                log::info!(
                    "WasmNetworkClient: Game ended - winner: {:?}, reason: {:?}",
                    winner,
                    reason
                );
                self.state = NetworkState::GameEnded;
                self.winner = Some(winner);
            }

            ServerMessage::SyncError { details, fatal } => {
                log::error!("WasmNetworkClient: Sync error (fatal={}): {:?}", fatal, details);
                if fatal {
                    self.state = NetworkState::Error;
                }
                self.last_error = Some(format!("Sync error: {:?}", details));
            }

            ServerMessage::Error { message, fatal } => {
                log::error!("WasmNetworkClient: Server error (fatal={}): {}", fatal, message);
                if fatal {
                    self.state = NetworkState::Error;
                }
                self.last_error = Some(message);
            }

            ServerMessage::Pong { .. } => {
                // Keepalive response, ignore
            }

            ServerMessage::LibraryReordered {
                player,
                new_order,
                action_count,
            } => {
                // mtg-o99ow L3: key the reorder DIRECTLY by its game `action_count`
                // (the shuffle's `ShuffleLibrary` or scry/surveil's `ReorderLibrary`
                // undo position), so the shadow adopts the new order BEFORE the
                // post-reorder draws at the SAME position on every replay (fixes
                // residual-#1, mtg-yexvc seed-2). `action_count == 0` is the
                // pre-game initial-order sync (two per client, would collide at ac 0
                // in the strict-monotonic log) — held separately and applied at the
                // first sync point before the GameLoop's opening-hand draws.
                // Minimal lazy protocol (mtg-o99ow): mid-game reorders arrive via
                // the ChoiceRequest buffer (BufferedFact::LibraryReorder) once the
                // buffer is authoritative. Ignore the eager copy. (ac==0 initial
                // orders always precede the first choice, so they are processed.)
                if self.buffer_is_authoritative {
                    return;
                }
                log::debug!(
                    "WasmNetworkClient: Library reordered for {:?} ({} cards) ac={} - logged",
                    player,
                    new_order.len(),
                    action_count
                );
                if action_count == 0 {
                    self.initial_library_orders.insert(player, new_order);
                } else {
                    self.push_state_sync(action_count, StateSyncEntry::LibraryReorder { player, new_order });
                }
            }

            ServerMessage::SearchCandidates {
                searcher,
                cards,
                action_count,
            } => {
                // mtg-o99ow L2d/L3: the N library-search candidates a searcher sees
                // are ONE atomic-multi-delta at the single search-resolution ac.
                // Keyed directly by that game ac as a single Vec-carrying entry
                // (N separate CardRevealed at one ac would collide on
                // ActionLog::push). Applied by replaying process_card_reveal over
                // each candidate so the shadow library learns the candidate
                // identities (the searcher's controller filters by name).
                // Minimal lazy protocol (mtg-o99ow): search candidates arrive via
                // the ChoiceRequest buffer (BufferedFact::SearchCandidates) once
                // the buffer is authoritative. Ignore the eager copy.
                if self.buffer_is_authoritative {
                    return;
                }
                log::debug!(
                    "WasmNetworkClient: SearchCandidates for {:?} ({} cards) ac={}",
                    searcher,
                    cards.len(),
                    action_count
                );
                self.push_state_sync(action_count, StateSyncEntry::SearchCandidates { searcher, cards });
            }

            #[cfg(debug_assertions)]
            ServerMessage::DebugStateDump { .. } => {
                // Debug info, just log
                log::debug!("WasmNetworkClient: Received debug state dump");
            }

            // ─── Lobby messages ────────────────────────────────────────────
            // The WASM client doesn't yet drive the multi-game lobby UI
            // (it joins a single game at a time), so for now we just log
            // these so the match stays exhaustive and so a developer can see
            // them in the browser console while the lobby UI is wired up.
            // TODO(server-lobby): plumb GameList / GameCreated through to
            // the browser UI once the lobby front-end lands.
            ServerMessage::GameList {
                games,
                total_count,
                system_memory_used_percent,
                max_memory_percent,
            } => {
                log::info!(
                    "WasmNetworkClient: GameList ({}/{} waiting games, host_mem={:?}%, ceiling={}%)",
                    games.len(),
                    total_count,
                    system_memory_used_percent,
                    max_memory_percent
                );
            }

            ServerMessage::GameCreated {
                game_name,
                your_player_id,
                your_name,
            } => {
                log::info!(
                    "WasmNetworkClient: GameCreated name={:?} player={:?} display_name={:?}",
                    game_name,
                    your_player_id,
                    your_name
                );
                self.our_player_id = Some(your_player_id);
                self.state = NetworkState::WaitingForOpponent;
            }

            ServerMessage::ServerFull { reason } => {
                // Server-side admission denial. Treat as fatal for this
                // socket — the server will close it. The wire `reason` is
                // intentionally generic (no host metrics); see
                // `protocol::ServerMessage::ServerFull` docs.
                log::warn!("WasmNetworkClient: ServerFull: {}", reason);
                self.state = NetworkState::Error;
                self.last_error = Some(reason);
            }

            ServerMessage::JoinFailed { game_name, reason } => {
                log::warn!("WasmNetworkClient: JoinFailed for {:?}: {:?}", game_name, reason);
                self.last_error = Some(format!("Join failed for {game_name}: {reason:?}"));
            }

            // New lobby protocol messages — handled at the JS layer or
            // safely ignored in the WASM game client.
            ServerMessage::RegisterResult {
                success,
                player_name,
                error,
            } => {
                log::info!(
                    "WasmNetworkClient: RegisterResult name={:?} success={} error={:?}",
                    player_name,
                    success,
                    error
                );
            }

            ServerMessage::WaitingRoomUpdate { .. } => {
                // Forwarded to JS via callback in the future; currently a no-op
                // in the WASM game client (the lobby page handles it).
                log::debug!("WasmNetworkClient: WaitingRoomUpdate (lobby layer)");
            }

            ServerMessage::WaitingRoomReady { .. } => {
                // Pre-game rendezvous "go" signal. The launcher's plain-JS lobby
                // socket consumes this, not the WASM game client — by the time
                // the game page (which owns this WASM client) connects, the
                // waiting-room handshake is already done. No-op here.
                log::debug!("WasmNetworkClient: WaitingRoomReady (launcher lobby layer)");
            }

            ServerMessage::ReconnectResult { success, game_name, .. } => {
                log::info!(
                    "WasmNetworkClient: ReconnectResult success={} game={:?}",
                    success,
                    game_name
                );
            }
        }
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // MESSAGE QUEUE ACCESS (for controllers)
    // ═══════════════════════════════════════════════════════════════════════════

    /// Check if a ChoiceRequest is pending
    pub fn has_choice_request(&self) -> bool {
        self.current_choice_request.is_some()
    }

    /// Get the current ChoiceRequest (without consuming)
    pub fn peek_choice_request(&self) -> Option<&ChoiceRequestData> {
        self.current_choice_request.as_ref()
    }

    /// Consume the current ChoiceRequest
    pub fn take_choice_request(&mut self) -> Option<ChoiceRequestData> {
        self.current_choice_request.take()
    }

    /// Check if choice has been acknowledged
    pub fn is_choice_acknowledged(&self) -> bool {
        self.choice_acknowledged
    }

    /// Check if at least one unconsumed `OpponentChoice` is buffered.
    ///
    /// Phase 2 step 2: this is the cursor-vs-frontier test on
    /// `opponent_choices` — equivalent to the legacy `!is_empty()` check
    /// against the old `VecDeque`, but it derives the answer from the
    /// non-destructive `ActionLog` + cursor pair. See the field-level
    /// docs on `opponent_choices` / `next_opponent_choice_cursor`.
    pub fn has_opponent_choice(&self) -> bool {
        self.opponent_choices
            .frontier()
            .is_some_and(|f| f > self.next_opponent_choice_cursor)
    }

    /// Consume the next buffered `OpponentChoice` (the smallest
    /// `choice_seq` strictly greater than the read cursor).
    ///
    /// **Non-destructive read.** The underlying `ActionLog` is
    /// untouched; only the per-controller cursor advances. A subsequent
    /// `reset_opponent_choice_cursor()` re-exposes the entries to a
    /// replay pass without re-fetching from the server (invariant #3 of
    /// `docs/NETWORK_ACTION_LOG.md` § 8).
    pub fn pop_opponent_choice(&mut self) -> Option<OpponentChoiceData> {
        let (key, entry) = self.next_opponent_choice_unconsumed()?;
        // Clone before bumping the cursor — entry borrows `&self`, so we
        // materialise it BEFORE the `&mut self` cursor mutation below.
        let materialised = OpponentChoiceData::from_log_entry(entry);
        self.next_opponent_choice_cursor = key;
        Some(materialised)
    }

    /// Peek at the next buffered `OpponentChoice` without consuming it.
    ///
    /// Used by SMART damage assignment overrides in `WasmRemoteController`
    /// so the override can extract `target_card_ids` BEFORE calling the
    /// pop path. The peek-then-pop pattern is needed because the trait
    /// method receives no protocol context, only blocker lists.
    ///
    /// Returns the entry as an owned `OpponentChoiceData` (cheap clone of
    /// the structured fields). The previous API returned `Option<&_>`; the
    /// new API returns owned `Option<_>` because the `ActionLog`-backed
    /// storage materialises the carry-on `action_count` alongside the
    /// payload (the wire `OpponentChoiceData` shape persists across the
    /// refactor).
    pub fn peek_opponent_choice(&self) -> Option<OpponentChoiceData> {
        let (_key, entry) = self.next_opponent_choice_unconsumed()?;
        Some(OpponentChoiceData::from_log_entry(entry))
    }

    /// Internal helper: find the first `(choice_seq, &ChoiceEntry)` in
    /// `opponent_choices` with `choice_seq > next_opponent_choice_cursor`.
    /// Returns `None` if no unconsumed entry is available.
    ///
    /// Linear scan over the unconsumed suffix. The `ActionLog::iter` impl
    /// yields in ascending key (`choice_seq`) order, so the first match is the
    /// correct next entry. Several entries can be buffered at once: during
    /// multi-step combat damage assignment the server emits several
    /// OpponentChoices in a row (each its own `choice_seq`), so the cursor
    /// walks them one per consume.
    fn next_opponent_choice_unconsumed(&self) -> Option<(u64, &ChoiceEntry)> {
        self.opponent_choices
            .iter()
            .find(|(ac, _)| *ac > self.next_opponent_choice_cursor)
    }

    /// Reset the opponent-choice read cursor so the next `pop_opponent_choice`
    /// re-reads every buffered entry from scratch.
    ///
    /// Used by snapshot-resume / rewind code paths: when the engine
    /// rewinds `game.action_count`, the controller's choice consumption
    /// also rewinds, so the log's entries must be re-readable during the
    /// forward replay pass. The log itself stays intact — non-destructive
    /// reads are precisely what makes "rewind + replay" free
    /// (invariant #3 of `docs/NETWORK_ACTION_LOG.md` § 8).
    pub fn reset_opponent_choice_cursor(&mut self) {
        self.next_opponent_choice_cursor = 0;
    }

    /// Current opponent-choice read cursor (highest consumed `choice_seq`).
    ///
    /// Paired with [`set_opponent_choice_cursor`] to support the multi-step
    /// combat damage assignment checkpoint/restore (mtg-sfihb): the engine
    /// snapshots this value before the synchronous first pass and restores it
    /// if a mid-pass sub-choice has not yet arrived, so the re-run re-consumes
    /// the buffered sub-choices from the same starting point.
    pub fn opponent_choice_cursor(&self) -> u64 {
        self.next_opponent_choice_cursor
    }

    /// Restore the opponent-choice read cursor to a value previously obtained
    /// from [`opponent_choice_cursor`]. See that method (mtg-sfihb).
    pub fn set_opponent_choice_cursor(&mut self, cursor: u64) {
        self.next_opponent_choice_cursor = cursor;
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // STATE-SYNC LOG (Phase 2 step 1 — reveal/reorder via ActionLog<StateSyncEntry>)
    // ═══════════════════════════════════════════════════════════════════════════
    //
    // See `docs/NETWORK_ACTION_LOG.md` § 3.2 for design and
    // `docs/NETWORK_ACTION_LOG_MIGRATION.md` § 1 for the deletion list this
    // replaces. The legacy `drain_reveals` / `drain_library_reorders` /
    // `pop_reveal` / `has_pending_reveals` helpers and the
    // `pending_reveals` / `pending_library_reorders` VecDeques are gone.

    /// Append a `StateSyncEntry` to the shadow state-sync log at its server-
    /// stamped game `action_count` (mtg-o99ow L3), keyed and consumed by game ac.
    ///
    /// Sole appender: the WS receive handler (`on_message`). Two wire realities
    /// make this more than a plain append (mtg-o99ow WASM bug #2):
    ///
    /// 1. **Out-of-order arrival.** The server emits a choice window's deltas via
    ///    two uncoordinated paths — the coordinator's `LibraryReordered` broadcast
    ///    (at the reorder's own, often LARGER, ac) precedes the handler's
    ///    `choice.reveals` loop (at each reveal's SMALLER ac) — so deltas ARRIVE
    ///    out of game-ac order. We use [`ActionLog::insert_sorted`] to restore
    ///    canonical game-position order; arrival order is a wire artifact because
    ///    the log is consumed by ac.
    ///
    /// 2. **Idempotent re-sends.** The server's immediate reveal pusher AND
    ///    `collect_reveals_since_last_choice` can both emit the same reveal
    ///    (intentional double-send the client dedups; see `controller.rs`
    ///    `shared_reveal_index`). Because `action_count == undo_log.len()` is
    ///    globally unique per logged action, two deltas sharing an ac can ONLY be
    ///    the SAME logical delta re-sent (the same-ac collision audit proved
    ///    distinct deltas never share an ac). So a duplicate-ac arrival — whether
    ///    the prior copy is still ahead of the cursor or was already applied
    ///    (the log is non-destructive, so it remains) — is benign: we VERIFY it
    ///    is the same delta via [`state_sync_entries_equivalent`] and DROP the
    ///    re-send. A *different* delta at the same ac would be a genuine desync
    ///    and is fatal.
    ///
    /// Only when no entry yet occupies this ac do we insert. At that point a
    /// brand-new delta arriving behind its CLASS apply cursor (the reveal cursor
    /// for reveals/search-candidates, the reorder cursor for reorders) means a
    /// delta needed at its game position never arrived in time and the cursor has
    /// moved on — a lost delta = desync, fatal (NETWORK_ARCHITECTURE.md: Desync is
    /// ALWAYS Fatal). The bound is `>=` (not `>`): the cursor starts at 0 and the
    /// first opening-hand reveal is legitimately stamped at game ac 0
    /// (`opening_reveal_ac(0) == 0`).
    /// Route a minimal-lazy-protocol `ChoiceRequest` buffer (mtg-o99ow) into the
    /// shadow's existing consumer logs. The buffer is ascending-`ac` and carries
    /// every reveal-class + opponent-choice fact since this client's last choice:
    /// reveal-class facts go into the game-`ac`-keyed `state_sync` log (applied
    /// lazily by `apply_state_sync_at`), and `Choice` facts go into the
    /// `choice_seq`-keyed `opponent_choices` log consumed by `WasmRemoteController`.
    ///
    /// This is the single routing point that replaces the eager
    /// `CardRevealed` / `LibraryReordered` / `SearchCandidates` / `OpponentChoice`
    /// message arms (now ignored once `buffer_is_authoritative`).
    fn apply_choice_buffer(&mut self, buffer: Vec<(u64, BufferedFact)>) {
        for (ac, fact) in buffer {
            match fact {
                BufferedFact::Reveal { owner, card, reason } => {
                    self.push_state_sync(
                        ac,
                        StateSyncEntry::RevealCard {
                            owner,
                            card: Box::new(card),
                            reason,
                        },
                    );
                }
                BufferedFact::LibraryReorder { player, new_order } => {
                    // ac==0 would be a pre-game initial order (held separately to
                    // avoid colliding at ac 0); the buffer only ever carries
                    // mid-game reorders, but mirror the eager arm defensively.
                    if ac == 0 {
                        self.initial_library_orders.insert(player, new_order);
                    } else {
                        self.push_state_sync(ac, StateSyncEntry::LibraryReorder { player, new_order });
                    }
                }
                BufferedFact::SearchCandidates { searcher, cards } => {
                    self.push_state_sync(ac, StateSyncEntry::SearchCandidates { searcher, cards });
                }
                BufferedFact::Choice {
                    choice_seq,
                    choice_indices,
                    description,
                    spell_ability,
                    library_search_result,
                    target_card_ids,
                    .. // choice_type is wire-envelope only; the controller re-derives it
                } => {
                    // Keyed by choice_seq (strictly unique/monotonic per choice),
                    // NOT ac — same-ac combat-damage choices (mtg-sfihb) share an
                    // ac but have distinct choice_seq. See ChoiceEntry doc.
                    // record_opponent_choice dedups against an eager copy that may
                    // have arrived before the buffer became authoritative.
                    self.record_opponent_choice(ChoiceEntry {
                        choice_seq,
                        action_count: ac,
                        choice_indices,
                        description,
                        spell_ability,
                        library_search_result,
                        target_card_ids,
                    });
                }
            }
        }
    }

    /// Record an opponent choice into the `choice_seq`-keyed `opponent_choices`
    /// log, deduping the Phase 1-2 dual-emit duplicate.
    ///
    /// During the additive-buffer window the SAME opponent choice can arrive
    /// BOTH eagerly (`ServerMessage::OpponentChoice`, processed only while the
    /// buffer is not yet authoritative) AND in our next `ChoiceRequest` buffer
    /// (`BufferedFact::Choice`). Which lands first varies by timing: the
    /// opponent's first choice can precede our first `ChoiceRequest` (eager
    /// first) — that race is exactly the `choice_seq=1` double-push that panicked
    /// `ActionLog::push`'s strict-monotonic key. We dedup by `choice_seq`, the
    /// same idempotent-resend discipline `push_state_sync` applies to reveals
    /// keyed by `action_count`.
    ///
    /// Keep-FIRST is safe because both copies derive from the coordinator's
    /// single `OpponentChoiceInfo`, so the decision payload is content-identical
    /// (verified: choice_indices / description / spell_ability /
    /// library_search_result / target_card_ids / action_count all flow from the
    /// same source). A `debug_assert` on `choice_indices` turns any genuine
    /// divergence into a fatal desync rather than a silent drop.
    ///
    /// Uses `push` (NOT `insert_sorted`): `opponent_choices` is the
    /// per-controller choice buffer (owner #1), which `action_log.rs` requires
    /// to stay strictly append-ordered. Opponent `choice_seq`s arrive
    /// monotonically; the only out-of-order arrival is the dual-emit duplicate,
    /// which we drop here before it reaches `push`.
    ///
    /// TRANSITIONAL: in Phase 3 (eager `OpponentChoice` deleted) only the buffer
    /// feeds this, the dedup becomes a no-op, and this helper collapses back to a
    /// bare `push`.
    fn record_opponent_choice(&mut self, entry: ChoiceEntry) {
        let key = u64::from(entry.choice_seq);
        if let Some(existing) = self.opponent_choices.get(key) {
            debug_assert_eq!(
                existing.choice_indices, entry.choice_indices,
                "record_opponent_choice: two DIFFERENT choices share choice_seq={} \
                 (existing indices={:?}, new={:?}) — protocol desync \
                 (NETWORK_ARCHITECTURE.md: Desync is ALWAYS Fatal).",
                key, existing.choice_indices, entry.choice_indices,
            );
            // Idempotent dual-emit duplicate — already logged. Drop it.
            return;
        }
        self.opponent_choices.push(key, entry);
    }

    fn push_state_sync(&mut self, action_count: u64, entry: StateSyncEntry) {
        // Route by delta CLASS into its own log (reorders vs reveals may share an
        // ac — see the `reorder_log` doc). Same-class collision rules are
        // unchanged: idempotent same-delta re-send = drop; DIFFERING same-class
        // delta @ one ac = fatal; new delta behind its class cursor = lost =
        // fatal. A cross-class reorder + reveal sharing one ac is now legal: they
        // land in different logs, never collide, and apply in separate passes
        // (reorder-first).
        let is_reorder = matches!(entry, StateSyncEntry::LibraryReorder { .. });
        let class_cursor = if is_reorder {
            self.last_applied_reorder_ac
        } else {
            self.last_applied_reveal_ac
        };
        let log = if is_reorder {
            &mut self.reorder_log
        } else {
            &mut self.reveal_log
        };
        if let Some(existing) = log.get(action_count) {
            assert!(
                state_sync_entries_equivalent(existing, &entry),
                "push_state_sync: two DIFFERENT same-class state-sync deltas share game ac={} \
                 (existing={:?}, new={:?}). Two same-class deltas cannot share an undo-log \
                 position — this is a protocol desync (NETWORK_ARCHITECTURE.md: Desync is ALWAYS Fatal).",
                action_count,
                existing,
                entry,
            );
            // Benign idempotent re-send (shared_reveal_index double-send); the
            // monotone info is already logged at this ac. Drop it.
            return;
        }
        assert!(
            action_count >= class_cursor,
            "push_state_sync: a NEW state-sync delta arrived at ac={} but the apply \
             cursor for its class has already advanced past it (cursor={}) and no prior \
             copy was logged — a delta needed at its game position never arrived in time \
             = lost delta = desync (NETWORK_ARCHITECTURE.md: Desync is ALWAYS Fatal).",
            action_count,
            class_cursor,
        );
        log.insert_sorted(action_count, entry);
    }

    /// Advance the reveal-history-complete watermark to a received choice's game
    /// `action_count`. By wire ordering, the server sends all of a choice's
    /// bundled reveals/reorders BEFORE the choice itself, so once a choice at ac
    /// A has arrived every delta with ac ≤ A has also arrived. The rewind
    /// verifier uses this as the "reveal history is complete up to R" signal.
    fn note_received_choice_ac(&mut self, action_count: u64) {
        if action_count > self.max_received_choice_ac {
            self.max_received_choice_ac = action_count;
        }
    }

    /// Highest game `action_count` of any choice the shadow has received. The
    /// rewind verifier treats the reveal-history as complete up to this value
    /// (see [`max_received_choice_ac`]).
    pub fn max_received_choice_ac(&self) -> u64 {
        self.max_received_choice_ac
    }

    /// Authoritative library-search result for an OPPONENT `searcher` resolving
    /// at game position `target_action` (mtg-mb668).
    ///
    /// On P_viewer's shadow, when the OPPONENT (`searcher`) tutors a card, the
    /// server can't reveal WHICH card (hidden information), so it sends a single
    /// **dummy `Searched` reveal**: empty `name`, but carrying the **authoritative
    /// fetched `card_id`** (server.rs ~2933, `RevealReason::Searched`,
    /// `owner = searcher`), stamped with the search choice's `action_count`. That
    /// CardId is exactly what the shadow needs to record into the
    /// `LibrarySearch(Some(id))` ChoicePoint so the `move_card(id, Library, Hand)`
    /// decrements the opponent's library count — the identity stays hidden, only
    /// the count/zone-move is tracked.
    ///
    /// This buffer is **rewind-surviving and action_count-keyed** (append-only),
    /// unlike the raced `OpponentChoice.library_search_result` (absent at the
    /// FIRST resolution → `None` recorded and replayed forever = mtg-mb668 sig-1).
    /// We pick the dummy `Searched` reveal owned by `searcher` with the GREATEST
    /// game `action_count` that is `<= target_action` (mtg-o99ow L3: the log key
    /// IS the game ac now — the dummy is stamped at the search-RESOLUTION ac and
    /// MUST stay there; re-stamping it at an earlier RevealCard position would
    /// break this "greatest ac ≤ target" selection and reintroduce the mtg-mb668
    /// desync). The shadow resolves that search at the same position, so the
    /// closest `<= target_action` reveal is THIS search's result. Repeated
    /// searches each carry a distinct (strictly larger) ac, so each resolution
    /// selects its own reveal — identical on the forward pass and every replay.
    ///
    /// We match ONLY **empty-name** `Searched` reveals (the opponent-fetch dummy).
    /// Our OWN search instead receives MULTIPLE *named* `Searched` reveals (the
    /// candidate library, server.rs ~2830) which are NOT a single fetched result —
    /// excluding them prevents the lookup misfiring on our own searches (which in
    /// any case resolve from a non-empty `valid_cards`, so the lookup is not even
    /// consulted for them).
    ///
    /// `None` when there is no such reveal — the search genuinely failed to find
    /// (CR 701.19c, no `Searched` reveal sent), or the reveal is not yet bound to
    /// a choice (still past the frontier).
    pub fn searched_card_for(&self, searcher: PlayerId, target_action: u64) -> Option<crate::core::CardId> {
        let mut best: Option<(u64, crate::core::CardId)> = None;
        for (ac, entry) in self.reveal_log.iter() {
            let StateSyncEntry::RevealCard { owner, card, reason } = entry else {
                continue;
            };
            // Opponent-fetch dummy: empty name (identity hidden), authoritative
            // card_id, owned by the searcher, reason == Searched.
            if *owner != searcher || !matches!(reason, crate::network::RevealReason::Searched) || !card.name.is_empty()
            {
                continue;
            }
            if ac <= target_action && best.is_none_or(|(best_ac, _)| ac >= best_ac) {
                best = Some((ac, card.card_id));
            }
        }
        best.map(|(_, card_id)| card_id)
    }

    /// Apply the game-start library orders to `shadow` exactly once, before any
    /// `skip_opening_hands` draw. See [`initial_library_orders`].
    fn apply_initial_library_orders(&mut self, shadow: &mut crate::game::GameState) {
        if self.initial_library_applied {
            return;
        }
        for (player, new_order) in &self.initial_library_orders {
            // Protocol order is top-to-bottom; shadow library Vec is bottom-to-top
            // (draw pops the last element).
            if let Some(zones) = shadow.get_player_zones_mut(*player) {
                zones.library.cards = new_order.iter().rev().copied().collect();
            }
        }
        self.initial_library_applied = true;
    }

    /// Apply every state-sync entry whose game `action_count` is `<= target_action`
    /// (and has arrived, i.e. `<= frontier()`) and is not yet applied, to `shadow`
    /// (mtg-o99ow L3). **Non-destructive read** of `state_sync` — only the
    /// per-client cursor advances; the log itself is untouched.
    ///
    /// Consumption is keyed by GAME POSITION: a delta stamped at game ac K is
    /// applied exactly when the shadow's own `action_count` reaches K, identically
    /// on the forward pass and on every rewind/replay. This is the reveal-as-choice
    /// alignment contract — it replaces the interim synthetic-key greedy
    /// "up_to_frontier" apply.
    ///
    /// `local_player` is forwarded to `process_card_reveal` so reveals targeting
    /// our hand resolve correctly. Returns the number of entries applied.
    pub fn apply_state_sync_at(
        &mut self,
        shadow: &mut crate::game::GameState,
        local_player: Option<PlayerId>,
        target_action: u64,
    ) -> usize {
        // Game-start library orders precede every in-game delta; establish them
        // before the first draw (and they are not in the ac-keyed logs).
        self.apply_initial_library_orders(shadow);

        let watermark = self.max_received_choice_ac;

        // Two independently-bounded passes over SEPARATE logs (see the cursor +
        // reorder_log field docs):
        //
        // Pass 1 — REORDERS, POSITIONAL: bound by `target_action` (the shadow's
        //   own action_count) ∧ reorder frontier. A shuffle's new library order
        //   must NOT be adopted before the shadow has replayed the actions at
        //   earlier acs (e.g. an opponent's cycling fetch that pulls a card OUT of
        //   the pre-shuffle library). Applying it early makes the fetched card
        //   vanish from the shadow's library → the search finds nothing → desync.
        // Pass 2 — REVEALS, EAGER: bound by the reveal-history-complete watermark
        //   (`max_received_choice_ac`) ∧ reveal frontier — applied AHEAD of the
        //   shadow's position so the shadow can replay opponent plays whose cards
        //   must be instantiated first (else "Entity not found"). Reveals are
        //   identity injections (library-order independent), so applying them
        //   early is safe.
        //
        // The eager reveal bound is the apply-frontier STALL fix (mtg-o99ow B2):
        // the prior single cursor bounded reveals by `target_action` too, so the
        // shadow forward-replay stalled at the last reveal (server=55/local=50)
        // instead of advancing to the choice's ac and consuming a buffered
        // opponent choice that sat above the last reveal.
        //
        // The watermark is the server's per-choice completeness signal: by wire
        // ordering (NETWORK_ARCHITECTURE.md) the server sends ALL deltas with
        // ac ≤ A before the choice at ac A, so once that choice has arrived every
        // delta with ac ≤ A is present — bounding the reveal apply by it
        // guarantees no entry below the reveal cursor is still in flight (the
        // principled L4 block-on-miss, keyed by game ac).
        let reorder_bound = target_action.min(self.reorder_log.frontier().unwrap_or(0));
        let reveal_bound = watermark.min(self.reveal_log.frontier().unwrap_or(0));

        let reorders: Vec<(u64, StateSyncEntry)> = self
            .reorder_log
            .iter()
            .filter(|(ac, _)| *ac > self.last_applied_reorder_ac && *ac <= reorder_bound)
            .map(|(ac, entry)| (ac, entry.clone()))
            .collect();
        let reveals: Vec<(u64, StateSyncEntry)> = self
            .reveal_log
            .iter()
            .filter(|(ac, _)| *ac > self.last_applied_reveal_ac && *ac <= reveal_bound)
            .map(|(ac, entry)| (ac, entry.clone()))
            .collect();
        if reorders.is_empty() && reveals.is_empty() {
            return 0;
        }
        self.last_applied_reorder_ac = self.last_applied_reorder_ac.max(reorder_bound);
        self.last_applied_reveal_ac = self.last_applied_reveal_ac.max(reveal_bound);

        // CRITICAL ORDERING (mtg-589): apply LibraryReorder entries BEFORE the
        // reveal-like entries — the server-authoritative library order must be in
        // place before a reveal moves a card out of the library, and before any
        // draw pops it. Re-runs after a rewind stay bit-identical because both
        // logs are non-destructive and the cursors are positional/watermarked.
        let mut applied = 0;
        for (ac, entry) in &reorders {
            if let StateSyncEntry::LibraryReorder { player, new_order } = entry {
                log::debug!(
                    "apply_state_sync: library reorder ac={} player={:?} ({} cards)",
                    ac,
                    player,
                    new_order.len()
                );
                if let Some(zones) = shadow.get_player_zones_mut(*player) {
                    zones.library.cards = new_order.iter().rev().copied().collect();
                }
            }
            applied += 1;
        }

        // Pass 2: reveal-like entries (single reveals + search-candidate bundles).
        for (ac, entry) in reveals {
            match entry {
                StateSyncEntry::RevealCard { owner, card, reason } => {
                    log::debug!(
                        "apply_state_sync: reveal ac={} owner={:?} card={}",
                        ac,
                        owner,
                        card.name
                    );
                    crate::wasm::network::game_init::process_card_reveal_wasm(
                        shadow,
                        owner,
                        *card,
                        reason,
                        local_player,
                    );
                }
                StateSyncEntry::SearchCandidates { searcher, cards } => {
                    log::debug!(
                        "apply_state_sync: search candidates ac={} searcher={:?} ({} cards)",
                        ac,
                        searcher,
                        cards.len()
                    );
                    for card in cards {
                        crate::wasm::network::game_init::process_card_reveal_wasm(
                            shadow,
                            searcher,
                            card,
                            crate::network::RevealReason::Searched,
                            local_player,
                        );
                    }
                }
                StateSyncEntry::LibraryReorder { .. } => {}
            }
            // The reveal cursor was already advanced to `reveal_bound` above.
            applied += 1;
        }
        applied
    }

    /// Reset the state-sync cursor so the next apply call re-applies every
    /// entry from scratch.
    ///
    /// Used by snapshot-resume / rewind code paths: when the engine rewinds
    /// `game.action_count`, the shadow state mutations also rewind, so the
    /// log's entries must be re-applied during the forward replay pass.
    /// The log itself stays intact — that's exactly what "non-destructive
    /// reads" buys us (invariant #3 of `docs/NETWORK_ACTION_LOG.md` § 8).
    pub fn reset_state_sync_cursor(&mut self) {
        self.last_applied_reorder_ac = 0;
        self.last_applied_reveal_ac = 0;
    }

    /// **Rebuild the reveal-history materialisation to game `action_count` R**
    /// (mtg-610).
    ///
    /// After a rewind to `action_count = retained_action`, the shadow's async
    /// opponent-instance set must equal exactly what it was at R — independent
    /// of how far the (now-discarded) forward pass had progressed or which
    /// reveals had arrived by the moment of THIS particular rewind. Reveals
    /// materialise opponent cards with a NON-undo-logged `game.cards.insert`
    /// (the shadow's own `move_card` no-ops the opponent's hidden-zone move), so
    /// the undo rewind alone cannot restore that set; the reveal-history buffer
    /// is its sole lifecycle owner and the per-entry effective-`action_count`
    /// stamp is the authoritative "does this instance exist at R" oracle.
    ///
    /// We therefore make it a pure function of R in two steps:
    ///   1. Clear every opponent instance materialised by a reveal — both the
    ///      "future" ones (effective_ac > R, which must NOT exist at R) and the
    ///      "retained" ones (effective_ac ≤ R), so step 2 starts from a clean
    ///      slate and we never depend on the forward pass's leftover state.
    ///   2. Reset the apply cursor and re-apply every reveal with
    ///      effective_ac ≤ R, re-materialising EXACTLY the position-R set.
    /// The turn-start hash, captured right after this call, is then a
    /// deterministic function of R.
    ///
    /// OUR OWN cards are never cleared: they have a real undo-log home
    /// (`draw_card` etc. are undo-logged), so the rewind already restored their
    /// instance + zone; the reveal for our own draw is only an identity echo.
    ///
    /// The log and the effective-ac stamps are untouched (append-only /
    /// non-destructive). No-op outside shadow games.
    pub fn unwind_state_sync_to(&mut self, shadow: &mut crate::game::GameState, retained_action: u64) {
        if !shadow.is_shadow_game {
            self.reset_state_sync_cursor();
            return;
        }
        let our_id = self.our_player_id;

        // Partition opponent reveal card-ids into RETAINED (some reveal with
        // effective_ac ≤ R names them — they belong at R) and "future-only"
        // (named ONLY by reveals with effective_ac > R, or not yet bound to any
        // choice — they must NOT exist at R).
        let mut retained_card_ids: std::collections::BTreeSet<crate::core::CardId> = std::collections::BTreeSet::new();
        let mut future_card_ids: Vec<crate::core::CardId> = Vec::new();
        // Reveals (incl. search candidates) live in `reveal_log`; reorders never
        // materialise instances, so the unwind only scans the reveal log.
        for (ac, entry) in self.reveal_log.iter() {
            if let StateSyncEntry::RevealCard { owner, card, .. } = entry {
                if our_id == Some(*owner) {
                    continue; // our own card — undo log owns its lifecycle
                }
                // A DUMMY reveal (empty name) does NOT materialise an instance
                // (`process_card_reveal` early-returns), so it must not count as
                // "this card belongs at R" — otherwise an opponent card whose
                // ONLY ≤ R reveal is the opening-hand dummy, but whose real
                // identity reveal is a future (> R) Played, would be wrongly
                // retained and its leaked future instance never cleared.
                if card.name.is_empty() {
                    continue;
                }
                // mtg-o99ow L3: the log key IS the game ac.
                if ac <= retained_action {
                    retained_card_ids.insert(card.card_id);
                } else {
                    future_card_ids.push(card.card_id);
                }
            }
        }

        // Step 1: clear ONLY the future-only instances. We must NOT clear a
        // RETAINED card: if it is currently present its per-instance state
        // (tapped / damage / counters) was restored by the undo rewind for a
        // public-zone card, and re-instantiating from the card def would lose
        // it — a real `compute_view_hash` desync. A future-only card that is
        // also covered by a retained reveal is kept (it belongs at R).
        for cid in future_card_ids {
            if retained_card_ids.contains(&cid) {
                continue;
            }
            shadow.cards.clear(cid);
        }

        // Step 2: ensure EVERY retained reveal (effective_ac ≤ R) is
        // materialised, in arrival order. `process_card_reveal` is idempotent —
        // it instantiates ONLY when the card is not already known, so a present
        // instance keeps its undo-restored per-instance state (tapped / damage /
        // counters) while an absent one (a prior rewind hadn't materialised it
        // yet, e.g. its reveal arrived only after that earlier rewind) is
        // re-created. We `filter` (NOT `take_while`) so an unstamped or
        // effective_ac > R entry interleaved among the retained ones does not
        // halt the scan and leave a later retained reveal unapplied — that gap
        // was the `cards[id]` PRIOR-null / CURRENT-present turn-start drift.
        let retained_to_apply: Vec<(PlayerId, Box<crate::network::CardReveal>, crate::network::RevealReason)> = self
            .reveal_log
            .iter()
            .filter(|(ac, _)| *ac <= retained_action)
            .filter_map(|(_, entry)| match entry {
                StateSyncEntry::RevealCard { owner, card, reason } => Some((*owner, card.clone(), *reason)),
                // Search-candidate bundles also materialise identities; replay them
                // at R so a rewind across a search reproduces the candidate set.
                StateSyncEntry::SearchCandidates { searcher, cards } => {
                    // Flatten handled below; map to None here and re-apply via the
                    // dedicated pass to avoid a Vec-in-tuple shape.
                    let _ = (searcher, cards);
                    None
                }
                _ => None,
            })
            .collect();
        for (owner, card, reason) in retained_to_apply {
            crate::wasm::network::game_init::process_card_reveal_wasm(shadow, owner, *card, reason, our_id);
        }
        // Re-apply retained search-candidate bundles (their identities also
        // materialise into the shadow library at R).
        let retained_searches: Vec<(PlayerId, Vec<crate::network::CardReveal>)> = self
            .reveal_log
            .iter()
            .filter(|(ac, _)| *ac <= retained_action)
            .filter_map(|(_, entry)| match entry {
                StateSyncEntry::SearchCandidates { searcher, cards } => Some((*searcher, cards.clone())),
                _ => None,
            })
            .collect();
        for (searcher, cards) in retained_searches {
            for card in cards {
                crate::wasm::network::game_init::process_card_reveal_wasm(
                    shadow,
                    searcher,
                    card,
                    crate::network::RevealReason::Searched,
                    our_id,
                );
            }
        }

        // Reset the apply cursor so the forward replay re-consumes the WHOLE log
        // as it re-advances. Initial library orders are NOT re-applied (the undo
        // rewind + re-applied in-game reorders restore the order at R); the
        // one-shot guard stays set.
        self.reset_state_sync_cursor();
    }

    /// Consume all state-sync deltas whose game `action_count` the shadow has
    /// already reached (`<= shadow.action_count()`), in ac order.
    ///
    /// mtg-o99ow L3: this is now exactly `apply_state_sync_at` with the target
    /// pinned to the shadow's current `action_count`. The interim version
    /// (mtg-610) skipped `LibraryReorder` entries and gated reveals by a separate
    /// effective-ac map; with the log keyed directly by game ac, reorders are
    /// correctly positioned (a shuffle's order applies at its `ShuffleLibrary` ac,
    /// before the post-shuffle draws), so the reveal/reorder two-pass is unified.
    /// Used by the interactive WASM sync point (`fancy_tui`).
    pub fn apply_state_sync_reveals_up_to_frontier(
        &mut self,
        shadow: &mut crate::game::GameState,
        local_player: Option<PlayerId>,
    ) -> usize {
        let target = shadow.action_count();
        self.apply_state_sync_at(shadow, local_player, target)
    }

    /// True iff there is at least one unapplied state-sync entry. Cheap
    /// frontier comparison; preserves the "K > frontier ⇒ yield NeedsInput"
    /// semantics of invariant #5 in design-doc terms.
    pub fn has_unapplied_state_sync(&self) -> bool {
        self.reorder_log
            .frontier()
            .is_some_and(|f| f > self.last_applied_reorder_ac)
            || self
                .reveal_log
                .frontier()
                .is_some_and(|f| f > self.last_applied_reveal_ac)
    }

    /// Diagnostic accessor: current REVEAL apply cursor. Test-only use.
    #[cfg(test)]
    pub(crate) fn last_applied_reveal_ac(&self) -> u64 {
        self.last_applied_reveal_ac
    }

    /// Diagnostic accessor: current REORDER apply cursor. Test-only use.
    #[cfg(test)]
    pub(crate) fn last_applied_reorder_ac(&self) -> u64 {
        self.last_applied_reorder_ac
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // OUTBOUND MESSAGE QUEUE
    // ═══════════════════════════════════════════════════════════════════════════

    /// Queue a SubmitChoice response
    pub fn submit_choice(&mut self, choice_indices: Vec<usize>, action_count: u64, state_hash: Option<u64>) {
        self.submit_choice_with_targets(choice_indices, action_count, state_hash, None)
    }

    /// Queue a SubmitChoice response, including authoritative `target_card_ids`.
    ///
    /// Used for choices like SMART damage assignment (`choose_blocker_for_lethal_damage`,
    /// `choose_blocker_for_remaining_damage`) where the server needs the actual chosen
    /// CardId — not just an index — because the client and server may have different
    /// blocker lists in network mode (mtg-418).
    pub fn submit_choice_with_targets(
        &mut self,
        choice_indices: Vec<usize>,
        action_count: u64,
        state_hash: Option<u64>,
        target_card_ids: Option<Vec<crate::core::CardId>>,
    ) {
        if let Some(ref request) = self.current_choice_request {
            let choice_seq = request.choice_seq;
            let msg = ClientMessage::SubmitChoice {
                choice_seq,
                choice_indices,
                action_count,
                timestamp_ms: crate::network::now_ms(),
                client_state_hash: state_hash,
                debug_info: None,
                spell_ability: None, // WASM client doesn't track spell_ability yet
                target_card_ids,
            };
            // mtg-mb668 class-A snapshot verification: log the DEFINITIVE wire
            // (seq, action_count, hash) at the actual submit point, so it can be
            // cross-checked against (a) the controller's WASM_CARD_DETAIL hash for
            // the same seq and (b) the server's rejected client_hash. If they all
            // agree, the WASM_CARD_DETAIL graveyard IS the rejected state.
            if self.network_debug {
                log::warn!(
                    "WASM_SUBMIT seq={} ac={} hash={}",
                    choice_seq,
                    action_count,
                    state_hash
                        .map(|h| format!("{h:016x}"))
                        .unwrap_or_else(|| "none".to_string()),
                );
            }
            self.queue_outbound(msg);
            self.choice_acknowledged = false;
            self.last_submitted_choice_seq = Some(choice_seq);
            self.current_choice_request = None;
            log::debug!(
                "WasmNetworkClient: Submitted choice seq={}, waiting for ack",
                choice_seq
            );
        } else {
            log::warn!("WasmNetworkClient: submit_choice called without pending request");
        }
    }

    /// Get the last submitted choice sequence number
    pub fn last_submitted_choice_seq(&self) -> Option<u32> {
        self.last_submitted_choice_seq
    }

    /// Clear the last submitted choice sequence (called when ack is processed)
    pub fn clear_last_submitted_choice_seq(&mut self) {
        self.last_submitted_choice_seq = None;
    }

    /// Queue a ping message
    pub fn ping(&mut self) {
        let msg = ClientMessage::Ping {
            timestamp_ms: crate::network::now_ms(),
        };
        self.queue_outbound(msg);
    }

    /// Queue an outbound message
    fn queue_outbound(&mut self, msg: ClientMessage) {
        match serde_json::to_string(&msg) {
            Ok(json) => {
                self.outbound_queue.push_back(json);
            }
            Err(e) => {
                log::error!("WasmNetworkClient: Failed to serialize message: {}", e);
            }
        }
    }

    /// Get the next outbound message (JavaScript polls this)
    pub fn get_outbound_message(&mut self) -> Option<String> {
        self.outbound_queue.pop_front()
    }

    /// Check if there are outbound messages
    pub fn has_outbound_messages(&self) -> bool {
        !self.outbound_queue.is_empty()
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // RESET
    // ═══════════════════════════════════════════════════════════════════════════

    /// Reset the client state for a new game
    pub fn reset(&mut self) {
        self.state = NetworkState::Disconnected;
        self.our_player_id = None;
        self.opponent_id = None;
        self.opponent_name = None;
        self.network_debug = false;
        self.reorder_log = ActionLog::new();
        self.reveal_log = ActionLog::new();
        self.last_applied_reorder_ac = 0;
        self.last_applied_reveal_ac = 0;
        self.initial_library_orders.clear();
        self.initial_library_applied = false;
        self.max_received_choice_ac = 0;
        self.opponent_choices = ActionLog::new();
        self.next_opponent_choice_cursor = 0;
        self.current_choice_request = None;
        self.choice_acknowledged = true;
        self.outbound_queue.clear();
        self.last_error = None;
        self.winner = None;
        // Clear late-binding architecture data (mtg-254)
        self.deck_card_ids = None;
        self.rng_state.clear();
        self.token_definitions.clear();
    }
}

impl Default for WasmNetworkClient {
    fn default() -> Self {
        Self::new()
    }
}

/// Shared reference to WasmNetworkClient for use across controllers
pub type SharedNetworkClient = Rc<RefCell<WasmNetworkClient>>;

/// Create a new shared network client
pub fn new_shared_client() -> SharedNetworkClient {
    Rc::new(RefCell::new(WasmNetworkClient::new()))
}

// ═══════════════════════════════════════════════════════════════════════════
// TESTS — state-sync log invariants (Phase 2 step 1)
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::CardId;
    use crate::network::{CardReveal, RevealReason};

    fn mk_reveal(name: &str) -> StateSyncEntry {
        StateSyncEntry::RevealCard {
            owner: PlayerId::new(0),
            card: Box::new(CardReveal {
                card_id: CardId::new(1),
                name: name.into(),
                card_def: None,
            }),
            reason: RevealReason::Draw,
        }
    }

    fn mk_reorder(player_id: u32) -> StateSyncEntry {
        StateSyncEntry::LibraryReorder {
            player: PlayerId::new(player_id),
            new_order: vec![CardId::new(7), CardId::new(8)],
        }
    }

    /// An OPPONENT-fetch dummy `Searched` reveal: empty name, authoritative
    /// CardId, owned by `searcher`, reason `Searched` (server.rs ~3026).
    fn mk_searched_dummy(searcher: u32, card_id: u32) -> StateSyncEntry {
        StateSyncEntry::RevealCard {
            owner: PlayerId::new(searcher),
            card: Box::new(CardReveal {
                card_id: CardId::new(card_id),
                name: String::new(),
                card_def: None,
            }),
            reason: RevealReason::Searched,
        }
    }

    #[test]
    fn searched_dummy_selected_by_greatest_ac_at_or_below_resolution() {
        // mtg-o99ow L4 RED test — pins the load-bearing invariant that the
        // opponent-fetch dummy `Searched` reveal STAYS stamped at the
        // search-RESOLUTION ac and is selected by "greatest game ac ≤ target".
        //
        // Two distinct opponent tutors resolve at acs 100 and 200 (the dummies
        // are stamped there). When the shadow resolves the FIRST search at game
        // position 150, it must pick card 11 (ac 100), NOT card 22 (ac 200,
        // still in the future). At position 250 it picks card 22.
        //
        // GUARD: if a future refactor mis-stamped the second dummy at an EARLIER
        // RevealCard position (say ac 50, before the first search), then
        // searched_card_for(150) would pick the greatest ≤ 150 = ac 100 still —
        // but searched_card_for for the FIRST resolution would wrongly see TWO
        // candidates ≤ its target and the deterministic "this search's own
        // result" mapping breaks (mtg-mb668 regression). We assert the dummies
        // sit at their resolution acs and selection is exact.
        let opp = PlayerId::new(1);
        let mut c = WasmNetworkClient::new();
        c.push_state_sync(100, mk_searched_dummy(1, 11));
        c.push_state_sync(200, mk_searched_dummy(1, 22));

        // Before the first resolution: no reveal ≤ target.
        assert_eq!(c.searched_card_for(opp, 99), None);
        // First search resolves around ac 150 → its own dummy (card 11 @100).
        assert_eq!(c.searched_card_for(opp, 150), Some(CardId::new(11)));
        // Exactly at the second resolution ac → card 22.
        assert_eq!(c.searched_card_for(opp, 200), Some(CardId::new(22)));
        // After both → still the latest ≤ target (card 22).
        assert_eq!(c.searched_card_for(opp, 250), Some(CardId::new(22)));
        // A NAMED Searched reveal (our own candidate) must be ignored by the
        // opponent-fetch lookup even if owned by `searcher`.
        c.push_state_sync(
            300,
            StateSyncEntry::RevealCard {
                owner: opp,
                card: Box::new(CardReveal {
                    card_id: CardId::new(33),
                    name: "Plains".into(),
                    card_def: None,
                }),
                reason: RevealReason::Searched,
            },
        );
        assert_eq!(c.searched_card_for(opp, 350), Some(CardId::new(22)));
    }

    #[test]
    fn push_state_sync_keys_by_game_action_count() {
        // mtg-o99ow L3: the log key IS the server-stamped game action_count.
        // Reveals route to `reveal_log`, reorders to `reorder_log` (the split).
        let mut c = WasmNetworkClient::new();
        c.push_state_sync(5, mk_reveal("a"));
        c.push_state_sync(8, mk_reorder(0));
        c.push_state_sync(13, mk_reveal("b"));
        let reveal_acs: Vec<u64> = c.reveal_log.iter().map(|(ac, _)| ac).collect();
        let reorder_acs: Vec<u64> = c.reorder_log.iter().map(|(ac, _)| ac).collect();
        assert_eq!(reveal_acs, vec![5, 13]);
        assert_eq!(reorder_acs, vec![8]);
        assert_eq!(c.reveal_log.frontier(), Some(13));
        assert_eq!(c.reorder_log.frontier(), Some(8));
    }

    #[test]
    fn push_state_sync_tolerates_out_of_order_arrival() {
        // mtg-o99ow WASM bug #2: the server emits a choice window's deltas via
        // two uncoordinated paths so a reveal at a larger ac can ARRIVE before one
        // at a smaller ac. Each per-class log is keyed+consumed by game ac, so
        // insert-sorted restores canonical game-position order regardless of wire
        // arrival order — no strict-monotonic-arrival panic.
        let mut c = WasmNetworkClient::new();
        c.push_state_sync(380, mk_reveal("late-but-larger")); // larger ac arrives FIRST
        c.push_state_sync(376, mk_reveal("early-but-smaller")); // smaller ac arrives SECOND
        let acs: Vec<u64> = c.reveal_log.iter().map(|(ac, _)| ac).collect();
        assert_eq!(acs, vec![376, 380], "reveal log must be re-sorted by game ac");
        assert!(c.reveal_log.get(376).is_some());
        assert!(c.reveal_log.get(380).is_some());
    }

    #[test]
    fn reorder_and_reveal_may_share_ac() {
        // THE SPLIT INVARIANT (mtg-o99ow, mirrors the native client test): a
        // shuffle-then-draw resolution (Timetwister / Wheel of Fortune /
        // Windfall) legitimately stamps a LibraryReorder and a RevealCard at the
        // SAME game ac. With ONE shared log this tripped the "two distinct deltas
        // share an ac" fatal assert and crashed the WASM shadow; with the split
        // they land in different logs and both survive. Order of arrival must not
        // matter.
        let mut c = WasmNetworkClient::new();
        c.push_state_sync(100, mk_reorder(0)); // reorder first
        c.push_state_sync(100, mk_reveal("Timetwister-draw")); // reveal at the SAME ac
        assert_eq!(c.reorder_log.get(100).map(|_| ()), Some(()));
        assert_eq!(c.reveal_log.get(100).map(|_| ()), Some(()));
        // Reverse arrival order is equally fine.
        let mut c2 = WasmNetworkClient::new();
        c2.push_state_sync(200, mk_reveal("draw")); // reveal first
        c2.push_state_sync(200, mk_reorder(1)); // reorder at the SAME ac
        assert!(c2.reveal_log.get(200).is_some());
        assert!(c2.reorder_log.get(200).is_some());
    }

    #[test]
    fn reveal_idempotent_resend_is_dropped() {
        // mtg-o99ow WASM bug #2: the server's shared_reveal_index immediate-pusher
        // and collect_reveals_since_last_choice both emit the same reveal, so an
        // IDENTICAL delta can arrive twice at the same game ac. Since distinct
        // SAME-CLASS deltas never share an ac, this is a benign re-send — the
        // second copy is dropped (not fatal), even after the first was already
        // applied (cursor moved past it; the log is non-destructive so it remains).
        let mut c = WasmNetworkClient::new();
        c.push_state_sync(42, mk_reveal("Mountain"));
        c.last_applied_reveal_ac = 50; // simulate reveal cursor advanced past 42
        c.push_state_sync(42, mk_reveal("Mountain")); // identical re-send → dropped
        assert_eq!(c.reveal_log.len(), 1, "idempotent re-send must not duplicate");
    }

    #[test]
    #[should_panic(expected = "two DIFFERENT same-class state-sync deltas share game ac")]
    fn reveal_vs_reveal_distinct_at_same_ac_is_fatal() {
        // The ORIGINAL dual-stamp desync class MUST stay fatal after the split:
        // two DIFFERENT reveals can never legitimately share a game ac (it is
        // undo_log.len(), globally unique per logged action). The split relaxes
        // ONLY cross-class (reorder + reveal); same-class collisions remain fatal.
        let mut c = WasmNetworkClient::new();
        c.push_state_sync(42, mk_reveal("a"));
        c.push_state_sync(42, mk_reveal("b")); // same ac, different card name → fatal
    }

    #[test]
    #[should_panic(expected = "two DIFFERENT same-class state-sync deltas share game ac")]
    fn reorder_vs_reorder_distinct_at_same_ac_is_fatal() {
        // Same-class reorder collisions must also stay fatal.
        let mut c = WasmNetworkClient::new();
        c.push_state_sync(42, mk_reorder(0));
        c.push_state_sync(42, mk_reorder(1)); // same ac, different player → fatal
    }

    #[test]
    #[should_panic(expected = "apply cursor for its class has already advanced past it")]
    fn push_state_sync_panics_when_new_delta_arrives_behind_cursor() {
        // A BRAND-NEW delta (no prior copy logged) arriving at an ac its class
        // apply cursor already passed would be silently dropped from every future
        // apply window — a needed delta lost = desync.
        let mut c = WasmNetworkClient::new();
        c.push_state_sync(10, mk_reveal("a"));
        c.last_applied_reveal_ac = 10; // reveal cursor consumed up to 10
        c.push_state_sync(7, mk_reveal("too-late")); // never seen before, too late
    }

    #[test]
    fn cursor_starts_at_zero_and_advances_with_apply() {
        // The cursors MUST track applied entries so a second apply call
        // re-applies nothing. This is the property that replaces the
        // legacy drain_*() destructive semantics with a non-destructive
        // read (invariant #3, #4). A real GameState round-trip is exercised
        // by the e2e regression test (tests/robots42_state_sync_e2e.sh).
        let mut c = WasmNetworkClient::new();
        assert_eq!(c.last_applied_reveal_ac(), 0);
        assert_eq!(c.last_applied_reorder_ac(), 0);
        assert!(!c.has_unapplied_state_sync());

        c.push_state_sync(1, mk_reveal("a"));
        c.push_state_sync(2, mk_reveal("b"));
        assert!(c.has_unapplied_state_sync());
        assert_eq!(c.last_applied_reveal_ac(), 0);
    }

    #[test]
    fn reset_state_sync_cursor_rewinds_for_replay() {
        // Snapshot/rewind support: after the engine rewinds, the next
        // forward pass must replay every entry. Both logs stay intact —
        // only the cursors reset.
        let mut c = WasmNetworkClient::new();
        c.push_state_sync(1, mk_reveal("x"));
        c.push_state_sync(2, mk_reorder(1));
        c.last_applied_reveal_ac = 1; // simulate post-apply
        c.last_applied_reorder_ac = 2;
        assert!(!c.has_unapplied_state_sync());

        c.reset_state_sync_cursor();
        assert_eq!(c.last_applied_reveal_ac(), 0);
        assert_eq!(c.last_applied_reorder_ac(), 0);
        assert!(c.has_unapplied_state_sync());
        // Logs are intact:
        assert_eq!(c.reveal_log.len(), 1);
        assert_eq!(c.reorder_log.len(), 1);
        assert_eq!(c.reveal_log.frontier(), Some(1));
        assert_eq!(c.reorder_log.frontier(), Some(2));
    }

    #[test]
    fn reset_clears_state_sync_log_and_cursor() {
        // A new game starts fresh: both logs empty, both cursors at 0.
        let mut c = WasmNetworkClient::new();
        c.push_state_sync(1, mk_reveal("x"));
        c.push_state_sync(2, mk_reorder(0));
        c.last_applied_reveal_ac = 1;
        c.reset();
        assert_eq!(c.reveal_log.len(), 0);
        assert_eq!(c.reorder_log.len(), 0);
        assert_eq!(c.reveal_log.frontier(), None);
        assert_eq!(c.reorder_log.frontier(), None);
        assert_eq!(c.last_applied_reveal_ac(), 0);
        assert_eq!(c.last_applied_reorder_ac(), 0);
        assert!(!c.initial_library_applied);
        assert!(!c.has_unapplied_state_sync());
    }

    #[test]
    fn has_unapplied_state_sync_reflects_cursor_position() {
        // Verifies the per-class frontier vs cursor comparison that gates the
        // "K > frontier ⇒ yield NeedsInput" signal at the wasm shim layer.
        let mut c = WasmNetworkClient::new();
        assert!(!c.has_unapplied_state_sync());
        c.push_state_sync(1, mk_reveal("a"));
        assert!(c.has_unapplied_state_sync());
        c.last_applied_reveal_ac = 1;
        assert!(!c.has_unapplied_state_sync());
        // A reorder ahead of the reorder cursor also counts as unapplied.
        c.push_state_sync(2, mk_reorder(0));
        assert!(c.has_unapplied_state_sync());
    }

    // ─── Phase 2 step 2 — per-controller choice buffer ─────────────────

    /// Drive the on_message OpponentChoice path. Bypasses JSON parsing by
    /// directly calling the private handler; verifies the message lands in
    /// `opponent_choices` keyed by the wire `choice_seq` (mtg-sfihb), while
    /// the wire `action_count` is carried on the payload.
    fn push_opponent_choice(client: &mut WasmNetworkClient, ac: u64, choice_seq: u32, desc: &str) {
        client.handle_server_message(crate::network::ServerMessage::OpponentChoice {
            choice_seq,
            player: PlayerId::new(1),
            choice_type: crate::network::ChoiceType::Priority,
            choice_indices: vec![choice_seq as usize],
            description: desc.into(),
            action_count: ac,
            timestamp_ms: 0,
            spell_ability: None,
            library_search_result: None,
            target_card_ids: None,
            state_hash_after: None,
            debug_info: None,
        });
    }

    #[test]
    fn opponent_choice_appended_to_action_log_by_wire_choice_seq() {
        // Wire-protocol `choice_seq` is the ActionLog key (NOT action_count);
        // pushes in strictly-increasing choice_seq order satisfy
        // ActionLog::push's monotonicity invariant. This replaces the legacy
        // VecDeque::push_back.
        let mut c = WasmNetworkClient::new();
        push_opponent_choice(&mut c, 5, 1, "first");
        push_opponent_choice(&mut c, 12, 2, "second");
        push_opponent_choice(&mut c, 100, 3, "third");

        assert_eq!(c.opponent_choices.len(), 3);
        // Frontier is the highest choice_seq (3), not the highest action_count.
        assert_eq!(c.opponent_choices.frontier(), Some(3));
        // Non-destructive: same look-up (by choice_seq) returns the same
        // payload twice; the carried action_count is preserved.
        let a = c.opponent_choices.get(2).unwrap();
        assert_eq!(a.description, "second");
        assert_eq!(a.action_count, 12);
        let b = c.opponent_choices.get(2).unwrap();
        assert_eq!(b.description, "second");
    }

    #[test]
    fn opponent_choices_sharing_one_action_count_do_not_panic() {
        // mtg-sfihb regression: during multi-step combat damage assignment
        // the server emits two OpponentChoices
        // (choose_blocker_for_lethal_damage then
        // choose_blocker_for_remaining_damage for the same attacker) with NO
        // undoable action between them, so BOTH carry the same action_count.
        // Keying the log by action_count made the second push panic with
        // "action_count must be strictly increasing". Keying by choice_seq
        // (strictly unique per choice) is correct. This is the exact shape
        // observed at action_count=978, choice_seq 181 then 182.
        let mut c = WasmNetworkClient::new();
        push_opponent_choice(&mut c, 978, 181, "lethal damage assignment");
        push_opponent_choice(&mut c, 978, 182, "remaining damage assignment");

        assert_eq!(c.opponent_choices.len(), 2);
        assert_eq!(c.opponent_choices.frontier(), Some(182));

        // Both are consumable in choice_seq order, each carrying ac=978.
        let first = c.pop_opponent_choice().unwrap();
        assert_eq!(first.choice_seq, 181);
        assert_eq!(first.action_count, 978);
        let second = c.pop_opponent_choice().unwrap();
        assert_eq!(second.choice_seq, 182);
        assert_eq!(second.action_count, 978);
        assert!(c.pop_opponent_choice().is_none());
    }

    #[test]
    fn cursor_checkpoint_restore_re_consumes_combat_damage_subchoices() {
        // mtg-sfihb layer 2: multi-step combat damage assignment runs in a
        // SYNCHRONOUS first pass that cannot yield mid-loop. On a shadow
        // client, when the engine has consumed the FIRST sub-choice but the
        // SECOND has not yet arrived over the wire, the controller signals
        // NeedInput; `assign_combat_damage` then restores the choice cursor to
        // the pre-pass checkpoint so the re-entry re-consumes BOTH sub-choices
        // from the start. This test exercises the cursor get/set that
        // `WasmRemoteController::{mark,restore}_choice_checkpoint` use.
        let mut c = WasmNetworkClient::new();
        // The opponent's lethal-damage sub-choice arrives first; the
        // remaining-damage sub-choice is still in flight.
        push_opponent_choice(&mut c, 978, 181, "lethal damage assignment");

        // Engine begins the synchronous first pass: checkpoint, then consume
        // the first sub-choice.
        let checkpoint = c.opponent_choice_cursor();
        let first = c.pop_opponent_choice().unwrap();
        assert_eq!(first.choice_seq, 181);
        // Second sub-choice not yet buffered -> the controller would NeedInput.
        assert!(c.pop_opponent_choice().is_none());

        // Restore the cursor (what restore_choice_checkpoint does) and re-raise
        // NeedInput. The first sub-choice is exposed again for the re-run.
        c.set_opponent_choice_cursor(checkpoint);
        assert!(c.has_opponent_choice());

        // The remaining-damage sub-choice now arrives.
        push_opponent_choice(&mut c, 978, 182, "remaining damage assignment");

        // Re-entry: the first pass re-runs and re-consumes BOTH sub-choices in
        // order from the restored checkpoint — no double-consume, no skip.
        let re_first = c.pop_opponent_choice().unwrap();
        assert_eq!(re_first.choice_seq, 181);
        let re_second = c.pop_opponent_choice().unwrap();
        assert_eq!(re_second.choice_seq, 182);
        assert!(c.pop_opponent_choice().is_none());
    }

    #[test]
    fn pop_opponent_choice_advances_cursor_without_dropping_entries() {
        // Replaces the legacy `VecDeque::pop_front`. The log is untouched;
        // only the per-client cursor advances. After three pops, the log
        // still has all three entries — a `reset_opponent_choice_cursor`
        // would re-expose them (the rewind / replay property).
        let mut c = WasmNetworkClient::new();
        push_opponent_choice(&mut c, 3, 1, "a");
        push_opponent_choice(&mut c, 7, 2, "b");
        push_opponent_choice(&mut c, 9, 3, "c");

        assert!(c.has_opponent_choice());
        let first = c.pop_opponent_choice().unwrap();
        assert_eq!(first.choice_seq, 1);
        assert_eq!(first.action_count, 3);
        let second = c.pop_opponent_choice().unwrap();
        assert_eq!(second.choice_seq, 2);
        assert_eq!(second.action_count, 7);
        let third = c.pop_opponent_choice().unwrap();
        assert_eq!(third.choice_seq, 3);
        assert_eq!(third.action_count, 9);
        assert!(!c.has_opponent_choice());
        assert!(c.pop_opponent_choice().is_none());

        // Log itself is intact — non-destructive read invariant.
        assert_eq!(c.opponent_choices.len(), 3);
    }

    #[test]
    fn peek_opponent_choice_does_not_advance_cursor() {
        // Multi-call peek must yield the same entry; only pop advances
        // the cursor. The SMART damage assignment overrides in
        // WasmRemoteController rely on this: peek to grab target_card_ids,
        // then pop the same entry.
        let mut c = WasmNetworkClient::new();
        push_opponent_choice(&mut c, 4, 11, "block");

        let peek_a = c.peek_opponent_choice().unwrap();
        let peek_b = c.peek_opponent_choice().unwrap();
        assert_eq!(peek_a.choice_seq, 11);
        assert_eq!(peek_b.choice_seq, 11);
        assert_eq!(peek_a.action_count, 4);

        let popped = c.pop_opponent_choice().unwrap();
        assert_eq!(popped.choice_seq, 11);
        assert!(c.peek_opponent_choice().is_none());
    }

    #[test]
    fn reset_opponent_choice_cursor_re_exposes_consumed_entries() {
        // Snapshot rewind / replay: the engine rolls back action_count via
        // its undo log; the controller's choice consumption must also roll
        // back. The log is the persistent store, the cursor is what moves.
        let mut c = WasmNetworkClient::new();
        push_opponent_choice(&mut c, 2, 1, "first");
        push_opponent_choice(&mut c, 8, 2, "second");
        let _ = c.pop_opponent_choice();
        let _ = c.pop_opponent_choice();
        assert!(!c.has_opponent_choice());
        assert_eq!(c.opponent_choices.len(), 2);

        c.reset_opponent_choice_cursor();

        assert!(c.has_opponent_choice());
        let replay_first = c.pop_opponent_choice().unwrap();
        assert_eq!(replay_first.choice_seq, 1);
        let replay_second = c.pop_opponent_choice().unwrap();
        assert_eq!(replay_second.choice_seq, 2);
    }

    #[test]
    fn reset_clears_opponent_choice_log_and_cursor() {
        // A new game wipes the buffer and resets the cursor (parallel to
        // the state_sync reset case in step 1).
        let mut c = WasmNetworkClient::new();
        push_opponent_choice(&mut c, 5, 1, "a");
        let _ = c.pop_opponent_choice();
        c.reset();
        assert_eq!(c.opponent_choices.len(), 0);
        assert_eq!(c.opponent_choices.frontier(), None);
        assert!(!c.has_opponent_choice());
    }

    #[test]
    #[should_panic(expected = "strictly increasing")]
    fn duplicate_opponent_choice_seq_panics() {
        // Wire-protocol guarantee: choice_seq is strictly monotonic per
        // session (the server bumps it once per ChoiceRequest). A duplicate
        // choice_seq is a protocol bug; per NETWORK_ARCHITECTURE.md "Desync
        // is ALWAYS a Fatal Error" we crash rather than silently accept it.
        // ActionLog::push provides the panic; this test verifies the wiring
        // forwards it on the NEW (choice_seq) key.
        let mut c = WasmNetworkClient::new();
        push_opponent_choice(&mut c, 5, 1, "first");
        push_opponent_choice(&mut c, 9, 1, "duplicate choice_seq");
    }
}
