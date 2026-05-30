//! Game loop implementation
//!
//! Manages the main game loop, turn progression, and priority system

/// Macro for conditional logging that avoids allocation when feature is disabled
///
/// When verbose-logging feature is disabled, this becomes a no-op at compile time,
/// eliminating all format! allocations that are a major performance bottleneck.
macro_rules! log_if_verbose {
    ($self:expr, $($arg:tt)*) => {
        #[cfg(feature = "verbose-logging")]
        {
            $self.log_normal(&format!($($arg)*));
        }
        #[cfg(not(feature = "verbose-logging"))]
        {
            let _ = &$self; // Suppress unused variable warning
        }
    };
}

/// Macro for logging official game actions with gamelog tagging
///
/// Similar to log_if_verbose! but uses log_gamelog() which adds
/// [GAMELOG TurnN STEP] prefix when --tag-gamelogs is enabled.
/// Use for official game actions that should be comparable across
/// local and network modes.
///
/// Currently unused at GameLoop level — per-card draw logging used to live
/// here but has been centralised inside `GameState::draw_card`. Kept around
/// as a convenience for future GameLoop-level gamelog calls (e.g. step
/// announcements) that need replay/verbosity gating.
#[allow(unused_macros)]
macro_rules! log_gamelog {
    ($self:expr, $($arg:tt)*) => {
        #[cfg(feature = "verbose-logging")]
        {
            $self.log_gamelog(&format!($($arg)*));
        }
        #[cfg(not(feature = "verbose-logging"))]
        {
            let _ = &$self; // Suppress unused variable warning
        }
    };
}

use crate::core::{CardId, PlayerId};
use crate::game::controller::{GameStateView, PlayerController};
use crate::game::phase::Step;
use crate::game::GameState;
use crate::{MtgError, Result};

/// Type alias for the sync callback function (network state synchronization)
///
/// This function is called at synchronization points to process pending
/// network state updates (primarily CardRevealed messages) up to a target action count.
/// The callback should process all pending updates with action_count <= target.
///
/// The callback takes:
/// - `&mut GameState` - mutable game state for applying updates (e.g., instantiating cards)
/// - `target_action: u64` - process all updates up to and including this action count
///
/// This deterministic approach (keyed by action count) replaces the previous
/// greedy drain approach, ensuring consistent synchronization behavior.
///
/// Used by network clients to sync state before operations that need revealed cards.
type SyncCallback = Box<dyn Fn(&mut GameState, u64)>;

// ═══════════════════════════════════════════════════════════════════════════════
// PRE-CHOICE HOOK TYPES (Network Client Architecture)
// ═══════════════════════════════════════════════════════════════════════════════

/// Identifies the type of choice about to be made
///
/// Used by the pre-choice hook to know what message to expect from the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChoiceKind {
    SpellAbility,
    Targets,
    ManaSources,
    Attackers,
    Blockers,
    DamageOrder,
    Discard,
    FromLibrary,
    Sacrifice,
    Modes,
    NotUntap,
    Options,
}

/// Result from the pre-choice hook
///
/// The hook drains messages from the network until it receives a choice signal,
/// processing CardRevealed messages along the way to update GameState.
#[derive(Debug)]
pub enum PreChoiceResult {
    /// Local player: ChoiceRequest received, proceed to call controller
    AskController,
    /// Remote player: OpponentChoice received, use these indices
    UseChoice(RawChoice),
    /// Game ended
    Exit,
}

/// Raw choice data received from network
///
/// Contains indices that the helper functions convert to the appropriate
/// choice type based on the `available` slice.
#[derive(Debug, Clone)]
pub struct RawChoice {
    /// Choice indices (interpretation depends on choice type)
    pub indices: Vec<usize>,
    /// For spell ability choices, the actual ability (server sends it directly)
    pub spell_ability: Option<crate::core::SpellAbility>,
    /// For library search choices, the CardId chosen by the server
    pub library_search_result: Option<crate::core::CardId>,
}

/// Pre-choice hook function type
///
/// Called before each controller choice point with `&mut GameState`.
/// Blocks on the network, processes CardRevealed messages, and returns
/// when a choice signal arrives (ChoiceRequest or OpponentChoice).
///
/// # Arguments
/// * `game` - Mutable game state for processing CardRevealed
/// * `player` - The player about to make a choice
/// * `kind` - What type of choice is being made
///
/// # Returns
/// * `AskController` - For local player, after ChoiceRequest received
/// * `UseChoice` - For remote player, with OpponentChoice data
/// * `Exit` - Game ended
pub type PreChoiceHook<'a> = Box<dyn FnMut(&mut GameState, PlayerId, ChoiceKind) -> PreChoiceResult + 'a>;

/// Callback type for pushing reveals AFTER automatic actions (like draws).
///
/// This function is called after automatic actions that reveal cards.
/// It receives a reference to the game state and the player who performed the action.
/// The callback should collect any new reveals and broadcast them immediately.
///
/// Used by network servers to push reveals to clients without waiting for ChoiceRequests.
type RevealPusher = Box<dyn Fn(&GameState, PlayerId) + Send>;

// Module structure
mod actions;
mod combat;
#[allow(deprecated)]
mod legacy;
mod logging;
mod network_choice;
mod priority;
mod snapshot;
mod steps;

/// Verbosity level for game output
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, serde::Serialize, serde::Deserialize)]
pub enum VerbosityLevel {
    /// Silent - no output during game
    Silent = 0,
    /// Minimal - only game outcome
    Minimal = 1,
    /// Normal - turns, steps, and key actions (default)
    #[default]
    Normal = 2,
    /// Verbose - all actions and state changes
    Verbose = 3,
}

/// Result of running a game to completion
#[derive(Debug, Clone)]
pub struct GameResult {
    /// Winner of the game (None if draw or game didn't complete)
    pub winner: Option<PlayerId>,
    /// Total number of turns played
    pub turns_played: u32,
    /// Reason the game ended
    pub end_reason: GameEndReason,
    /// Final action count (undo log length) for synchronization verification
    pub action_count: u64,
}

/// Reason the game ended
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GameEndReason {
    /// A player won by reducing opponent's life to 0 or less
    PlayerDeath(PlayerId),
    /// A player won by decking their opponent
    Decking(PlayerId),
    /// Game reached maximum turn limit
    TurnLimit,
    /// Game ended in a draw
    Draw,
    /// Game was manually ended
    Manual,
    /// Game was stopped to save a snapshot
    Snapshot,
}

/// State of the game loop when running with human input support
///
/// Used by `run_until_input()` to signal whether the game completed
/// or is waiting for human input.
#[derive(Debug, Clone)]
pub enum GameLoopState {
    /// Game completed (win, loss, draw, or turn limit)
    Complete(GameResult),

    /// Game is waiting for human input
    ///
    /// The UI should display the choice context to the player,
    /// set the pending choice on the WasmHumanController, and
    /// call `run_until_input()` again to continue.
    AwaitingInput(crate::game::controller::ChoiceContext),
}

/// Game loop manager
///
/// Handles turn progression, priority, and win condition checking
pub struct GameLoop<'a> {
    /// The game state
    pub game: &'a mut GameState,
    /// Maximum turns before forcing a draw
    max_turns: u32,
    /// Turn counter for the loop
    turns_elapsed: u32,
    /// Verbosity level for output (cached from game.logger)
    pub verbosity: VerbosityLevel,
    /// Track if current step header has been printed (for lazy printing)
    step_header_printed: bool,
    // spell_targets is now in GameState (game.spell_targets) to survive WASM step_harness() re-entry.
    /// Global choice counter for tracking all player choices
    /// Increments each time a controller makes any decision
    choice_counter: u32,
    /// Reusable mana engine for checking mana availability
    /// Updated per-player as needed, retains Vec capacity across calls
    mana_engine: crate::game::mana_engine::ManaEngine,
    /// Reusable buffer for collecting available spell abilities
    /// Cleared and reused each priority round to avoid allocations
    abilities_buffer: Vec<crate::core::SpellAbility>,
    /// Reusable buffer for mana source tap order computation
    /// Cleared and reused for each spell cast to avoid allocations
    sources_to_tap_buffer: Vec<CardId>,
    /// Stop and snapshot when fixed controller is exhausted
    stop_when_fixed_exhausted: bool,
    /// Snapshot path for fixed-exhausted snapshots
    snapshot_path_for_fixed: Option<std::path::PathBuf>,
    /// Serialization format for snapshots
    snapshot_format: crate::game::snapshot::SnapshotFormat,
    /// Stop condition tracking for --stop-on-choice (p1_id, stop_condition, snapshot_path)
    stop_condition_info: Option<(PlayerId, crate::game::StopCondition, std::path::PathBuf)>,
    /// Baseline choice count when resuming from snapshot (to avoid counting pre-snapshot choices)
    baseline_choice_count: usize,
    /// Execution mode: are we replaying choices from a snapshot?
    /// When true, all logging is suppressed to avoid duplicate output.
    replaying: bool,
    /// Number of choices remaining to replay from snapshot
    /// When this reaches 0, we switch from replaying mode back to playing forward.
    replay_choices_remaining: usize,
    /// Flag indicating we just resumed from snapshot and should skip turn header on first turn
    /// Gets cleared after the first turn executes.
    resumed_from_snapshot: bool,
    /// Tracks whether the ">>> Turn 1 - ..." gamelog header has been emitted.
    /// Subsequent turn headers are emitted by GameState::next_turn(), but Turn 1
    /// has no preceding next_turn() call, so it must be emitted from run_turn()
    /// (or setup_game()) on the first turn. The flag prevents double emission
    /// when both paths fire.
    turn_one_header_emitted: bool,
    /// The turn number we resumed into (used to suppress header for that specific turn only)
    resumed_turn_number: Option<u32>,
    /// Optional hand setup for Player 1 (controlled initial hand)
    p1_hand_setup: Option<crate::game::HandSetup>,
    /// Optional hand setup for Player 2 (controlled initial hand)
    p2_hand_setup: Option<crate::game::HandSetup>,
    /// Optional separate seed for deck shuffling (--deck-seed)
    /// If set, library shuffling uses this seed, then game continues with game_seed
    deck_seed: Option<u64>,
    /// The main game seed to use after shuffling (only needed when deck_seed is set)
    game_seed: Option<u64>,
    /// Optional sync callback for network mode (client-side)
    ///
    /// When set, this function is called at synchronization points to process
    /// pending network state updates up to the current action count. This includes
    /// CardRevealed messages that instantiate cards before they're needed.
    sync_callback: Option<SyncCallback>,
    /// Optional reveal pusher for network mode (server-side)
    ///
    /// When set, this function is called after automatic actions (like draws) to
    /// push reveals to clients immediately without waiting for ChoiceRequests.
    reveal_pusher: Option<RevealPusher>,
    /// Skip opening hand setup (for network clients)
    ///
    /// When true, run_game skips shuffling and drawing opening hands.
    /// Network clients use this because the server has already performed setup
    /// and the client draws cards via the reveal drainer mechanism.
    skip_opening_hands: bool,
    /// Enable fail-fast validation that all cards in hand/battlefield are revealed
    ///
    /// When true, panics if any card in a player's hand or on battlefield is not
    /// revealed when building available actions. This catches missing reveals
    /// early in network mode where desync can occur from missing CardRevealed messages.
    debug_validate_reveals: bool,
    /// Local player ID for network mode validation
    ///
    /// In network mode (hidden info architecture), only the local player's cards
    /// are revealed. Set this to skip validation for opponent's cards.
    /// When None, validation checks all players (local/single-player mode).
    local_player_id: Option<PlayerId>,
    /// Pre-choice hook for network mode
    ///
    /// When set, this hook is called before each controller choice point.
    /// It blocks on the network, processes CardRevealed messages, and returns
    /// when a choice signal arrives (ChoiceRequest for local, OpponentChoice for remote).
    pre_choice_hook: Option<PreChoiceHook<'a>>,
    /// Defer game-end checks to end of turn (for network clients)
    ///
    /// When true, mid-step game-end checks (e.g., after combat damage) are skipped.
    /// This is for network clients where the server is authoritative - the client
    /// waits for GameEnded from the server rather than detecting locally.
    defer_game_end_check: bool,
}

impl<'a> GameLoop<'a> {
    /// Create a new game loop for the given game state
    pub fn new(game: &'a mut GameState) -> Self {
        let verbosity = game.logger.verbosity();
        GameLoop {
            game,
            max_turns: 1000, // Default maximum turns
            turns_elapsed: 0,
            verbosity,
            step_header_printed: false,
            // spell_targets lives in game.spell_targets
            choice_counter: 0,
            mana_engine: crate::game::mana_engine::ManaEngine::new(),
            abilities_buffer: Vec::with_capacity(16), // Pre-allocate for typical game (lands + spells + abilities)
            sources_to_tap_buffer: Vec::with_capacity(8), // Pre-allocate for typical mana costs (0-6 sources)
            stop_when_fixed_exhausted: false,
            snapshot_path_for_fixed: None,
            snapshot_format: crate::game::snapshot::SnapshotFormat::default(),
            stop_condition_info: None,
            baseline_choice_count: 0,
            replaying: false,
            replay_choices_remaining: 0,
            resumed_from_snapshot: false,
            resumed_turn_number: None,
            turn_one_header_emitted: false,
            p1_hand_setup: None,
            p2_hand_setup: None,
            deck_seed: None,
            game_seed: None,
            sync_callback: None,
            reveal_pusher: None,
            skip_opening_hands: false,
            debug_validate_reveals: false,
            local_player_id: None,
            pre_choice_hook: None,
            defer_game_end_check: false,
        }
    }

    /// Set maximum turns before forcing a draw
    pub fn with_max_turns(mut self, max_turns: u32) -> Self {
        self.max_turns = max_turns;
        self
    }

    /// Set the snapshot serialization format
    pub fn with_snapshot_format(mut self, format: crate::game::snapshot::SnapshotFormat) -> Self {
        self.snapshot_format = format;
        self
    }

    /// Enable debug mode for mana cache verification
    ///
    /// When enabled, every mana query will be verified against a full battlefield
    /// scan to ensure the incremental cache-based computation matches the from-scratch
    /// result. This is expensive and should only be used in stress tests.
    ///
    /// This implements the "from-scratch consistency" principle: incremental
    /// computation must match full recomputation.
    pub fn with_mana_debug_verification(mut self) -> Self {
        self.mana_engine = self.mana_engine.with_debug_verification();
        self
    }

    /// Enable fail-fast reveal validation for network debugging
    ///
    /// When enabled, panics immediately if any card in the local player's hand is not
    /// revealed when building available actions. This catches missing CardRevealed
    /// messages early, before they cause desync.
    ///
    /// In network mode (hidden info architecture), only the local player's cards are
    /// revealed. Pass the local player's ID to skip validation for opponent's cards.
    ///
    /// This should only be enabled when `network_debug` is true - the validation adds
    /// overhead that should be avoided in production network games.
    ///
    /// # Arguments
    /// * `local_player` - The local player's ID (for skipping opponent validation)
    /// * `enabled` - Whether to actually enable validation (typically `network_debug`)
    pub fn with_reveal_validation(mut self, local_player: PlayerId, enabled: bool) -> Self {
        self.debug_validate_reveals = enabled;
        self.local_player_id = Some(local_player);
        self
    }

    /// Set verbosity level for output
    ///
    /// This sets the verbosity on both the game loop and the game's centralized logger,
    /// which is accessed by controllers via GameStateView.
    pub fn with_verbosity(mut self, verbosity: VerbosityLevel) -> Self {
        self.verbosity = verbosity;
        self.game.logger.set_verbosity(verbosity);
        self
    }

    /// Set initial turn counter (for resuming from snapshots)
    ///
    /// This should be called when loading a game from a snapshot to ensure
    /// turn numbering continues correctly.
    pub fn with_turn_counter(mut self, turns_elapsed: u32) -> Self {
        self.turns_elapsed = turns_elapsed;
        self
    }

    /// Set the initial choice counter value when loading from a snapshot
    ///
    /// This preserves the cumulative choice count across snapshot/resume boundaries.
    /// Without this, choice IDs would restart from 0 on each resume, breaking determinism.
    pub fn with_choice_counter(mut self, choice_count: u32) -> Self {
        self.choice_counter = choice_count;
        self
    }

    /// Enable stop-when-fixed-exhausted mode with snapshot path
    ///
    /// When enabled, the game will automatically save a snapshot and exit
    /// when a FixedScriptController runs out of predetermined choices.
    pub fn with_stop_when_fixed_exhausted<P: AsRef<std::path::Path>>(mut self, snapshot_path: P) -> Self {
        self.stop_when_fixed_exhausted = true;
        self.snapshot_path_for_fixed = Some(snapshot_path.as_ref().to_path_buf());
        // Enable choice menu display when in stop/go mode
        self.game.logger.set_show_choice_menu(true);
        // Enable log buffering (Both mode: output to stdout AND capture to memory)
        self.game.logger.set_output_mode(crate::game::OutputMode::Both);
        self
    }

    /// Enable stop condition for --stop-on-choice (mid-turn exit at exact choice count)
    ///
    /// When enabled, the game will save a snapshot and exit as soon as the filtered
    /// choice count reaches the limit specified in the stop condition. This provides
    /// precise stopping at the exact choice point (no overshooting).
    pub fn with_stop_condition<P: AsRef<std::path::Path>>(
        mut self,
        p1_id: PlayerId,
        stop_condition: crate::game::StopCondition,
        snapshot_path: P,
    ) -> Self {
        self.stop_condition_info = Some((p1_id, stop_condition, snapshot_path.as_ref().to_path_buf()));
        // Enable choice menu display when in stop/go mode
        self.game.logger.set_show_choice_menu(true);
        // Enable log buffering (Both mode: output to stdout AND capture to memory)
        self.game.logger.set_output_mode(crate::game::OutputMode::Both);
        self
    }

    /// Set baseline choice count when resuming from snapshot
    ///
    /// This is needed so that count_filtered_choices() doesn't count choices
    /// that were made before the snapshot was saved.
    pub fn with_baseline_choice_count(mut self, count: usize) -> Self {
        self.baseline_choice_count = count;
        self
    }

    /// Set hand setup for Player 1 (controlled initial hand for testing)
    pub fn with_p1_hand_setup(mut self, hand_setup: crate::game::HandSetup) -> Self {
        self.p1_hand_setup = Some(hand_setup);
        self
    }

    /// Set hand setup for Player 2 (controlled initial hand for testing)
    pub fn with_p2_hand_setup(mut self, hand_setup: crate::game::HandSetup) -> Self {
        self.p2_hand_setup = Some(hand_setup);
        self
    }

    /// Set a separate seed for deck shuffling
    ///
    /// When provided, library shuffling uses `deck_seed` and then the RNG is re-seeded
    /// with `game_seed` for the rest of gameplay. This allows varying the game seed
    /// independently while keeping the same initial hands.
    ///
    /// # Arguments
    /// * `deck_seed` - Seed used for initial library shuffling
    /// * `game_seed` - Seed used for game RNG after shuffling (if None, keeps deck_seed)
    pub fn with_deck_seed(mut self, deck_seed: u64, game_seed: Option<u64>) -> Self {
        self.deck_seed = Some(deck_seed);
        self.game_seed = game_seed;
        self
    }

    /// Set a sync callback for network mode
    ///
    /// The sync callback is called at synchronization points to process pending
    /// network state updates (primarily CardRevealed messages) up to a target action count.
    /// This ensures cards are instantiated before they're needed for validation or display.
    ///
    /// The callback receives:
    /// - `&mut GameState` - mutable game state for applying updates
    /// - `target_action: u64` - process all updates with action_count <= target
    ///
    /// # Example
    ///
    /// ```ignore
    /// let pending_reveals: Arc<Mutex<VecDeque<(u64, PlayerId, CardReveal)>>> = ...;
    /// let reveals_clone = pending_reveals.clone();
    ///
    /// game_loop.with_sync_callback(move |game, target_action| {
    ///     if let Ok(mut queue) = reveals_clone.lock() {
    ///         // Process reveals up to target action count
    ///         while queue.front().map(|(ac, _, _)| *ac <= target_action).unwrap_or(false) {
    ///             let (_, owner, reveal) = queue.pop_front().unwrap();
    ///             process_card_reveal(game, owner, reveal);
    ///         }
    ///     }
    /// });
    /// ```
    pub fn with_sync_callback<F>(mut self, callback: F) -> Self
    where
        F: Fn(&mut GameState, u64) + 'static,
    {
        self.sync_callback = Some(Box::new(callback));
        self
    }

    /// Set the reveal pusher for network server mode.
    ///
    /// The pusher is called AFTER automatic actions (like draws) to push reveals
    /// to clients immediately. This ensures clients receive reveals before their
    /// GameLoop needs them for synchronization.
    ///
    /// The callback receives:
    /// - `game`: The current game state to collect reveals from
    /// - `player`: The player who performed the automatic action
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // Network server pushes reveals after draws
    /// let reveal_tx = Arc::new(Mutex::new(reveal_tx));
    /// let tx_clone = reveal_tx.clone();
    ///
    /// game_loop.with_reveal_pusher(move |game, player| {
    ///     // Collect and send reveals for both players
    ///     if let Ok(tx) = tx_clone.lock() {
    ///         // Push reveals to channel for WebSocket handlers to broadcast
    ///     }
    /// });
    /// ```
    pub fn with_reveal_pusher<F>(mut self, pusher: F) -> Self
    where
        F: Fn(&GameState, PlayerId) + Send + 'static,
    {
        self.reveal_pusher = Some(Box::new(pusher));
        self
    }

    /// Skip opening hand setup (for network clients)
    ///
    /// When enabled, `run_game` will not shuffle libraries or draw opening hands.
    /// This is used by network clients where the server has already performed game
    /// setup and the client receives opening hands via CardRevealed messages.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // Network client skips local opening hand draw
    /// let mut game_loop = GameLoop::new(&mut game)
    ///     .with_sync_callback(sync_callback)
    ///     .skip_opening_hands();
    /// ```
    pub fn skip_opening_hands(mut self) -> Self {
        self.skip_opening_hands = true;
        self
    }

    /// Defer game-end checks to end of turn
    ///
    /// For network clients, the server is authoritative about game end. This flag
    /// skips mid-step game-end checks (e.g., after combat damage) to prevent the
    /// client from detecting game end before the server sends GameEnded.
    pub fn with_deferred_game_end(mut self) -> Self {
        self.defer_game_end_check = true;
        self
    }

    /// Set the pre-choice hook for network mode
    ///
    /// The pre-choice hook is called before each controller choice point.
    /// It blocks on the network, processes CardRevealed messages to update
    /// GameState, and returns when a choice signal arrives.
    ///
    /// # Arguments
    /// * `hook` - Closure that takes `(&mut GameState, PlayerId, ChoiceKind)`
    ///   and returns `PreChoiceResult`
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let msg_rx = /* network message receiver */;
    /// let card_db = /* card database */;
    /// let our_player = PlayerId::new(0);
    ///
    /// let hook = move |game: &mut GameState, player: PlayerId, kind: ChoiceKind| {
    ///     loop {
    ///         let msg = msg_rx.recv().expect("channel closed");
    ///         match msg {
    ///             NetworkMessage::CardRevealed { owner, card, reason } => {
    ///                 process_card_reveal(game, &card_db, owner, card, reason);
    ///             }
    ///             NetworkMessage::ChoiceRequest { .. } if player == our_player => {
    ///                 return PreChoiceResult::AskController;
    ///             }
    ///             NetworkMessage::OpponentChoice { indices, spell_ability, .. } if player != our_player => {
    ///                 return PreChoiceResult::UseChoice(RawChoice { indices, spell_ability });
    ///             }
    ///             NetworkMessage::GameEnded { .. } => {
    ///                 return PreChoiceResult::Exit;
    ///             }
    ///             _ => {}
    ///         }
    ///     }
    /// };
    ///
    /// game_loop.with_pre_choice_hook(hook);
    /// ```
    pub fn with_pre_choice_hook<F>(mut self, hook: F) -> Self
    where
        F: FnMut(&mut GameState, PlayerId, ChoiceKind) -> PreChoiceResult + 'a,
    {
        self.pre_choice_hook = Some(Box::new(hook));
        self
    }

    /// Check if network mode is enabled (pre-choice hook is set)
    pub fn is_network_mode(&self) -> bool {
        self.pre_choice_hook.is_some()
    }

    /// Call the pre-choice hook if configured
    ///
    /// Returns `None` if no hook is configured (non-network mode).
    /// In non-network mode, callers should proceed directly to calling the controller.
    pub(super) fn call_pre_choice_hook(&mut self, player: PlayerId, kind: ChoiceKind) -> Option<PreChoiceResult> {
        if let Some(ref mut hook) = self.pre_choice_hook {
            Some(hook(self.game, player, kind))
        } else {
            None
        }
    }

    /// Sync network state up to the current action count
    ///
    /// This is called at synchronization points to process pending network
    /// state updates (primarily CardRevealed messages) before operations that
    /// need revealed cards (e.g., validation, building available actions).
    ///
    /// Uses the current game action_count as the target, ensuring deterministic
    /// synchronization behavior.
    pub(super) fn sync_to_action(&mut self) {
        if let Some(ref callback) = self.sync_callback {
            let target = self.game.action_count();
            callback(self.game, target);
        }
    }

    /// Sync network state up to a specific action count
    ///
    /// Like `sync_to_action()` but allows specifying the target action count.
    /// Used when you need to sync to a specific point (e.g., before validation).
    #[allow(dead_code)]
    pub(super) fn sync_to_action_count(&mut self, target: u64) {
        if let Some(ref callback) = self.sync_callback {
            callback(self.game, target);
        }
    }

    /// Push reveals for an automatic action if a pusher is configured
    ///
    /// This is called automatically after automatic actions (like draws) to push
    /// reveals to network clients immediately. The player parameter indicates who
    /// performed the action.
    pub(super) fn push_reveals(&self, player: PlayerId) {
        if let Some(ref pusher) = self.reveal_pusher {
            pusher(self.game, player);
        }
    }

    /// Set replay mode for resuming from snapshot
    ///
    /// When resuming from a snapshot, we replay intra-turn choices to restore game state.
    /// During this replay, ALL logging is suppressed because snapshots are taken BEFORE
    /// presenting a choice to the controller. This means all choices in the snapshot were
    /// already made, executed, and logged in previous segments.
    ///
    /// After all choices are replayed, replay mode is cleared and the NEXT choice is
    /// presented fresh to the controller (this is where the snapshot paused).
    ///
    /// This method enables replay mode and sets the number of choices to replay.
    /// Also sets resumed_from_snapshot flag to suppress turn header on first turn.
    pub fn with_replay_mode(mut self, choice_count: usize) -> Self {
        // Always enable replay mode when resuming from snapshot
        // Even if there are 0 intra-turn choices to replay, we still need to suppress
        // logging for automatic actions (like draws) until we reach the first NEW choice
        self.replaying = true;
        self.replay_choices_remaining = choice_count;
        if self.verbosity >= VerbosityLevel::Verbose && self.should_print_to_stdout() {
            if choice_count > 0 {
                println!("🔄 REPLAY MODE ENABLED: {} choices to replay", choice_count);
            } else {
                println!("🔄 REPLAY MODE ENABLED: 0 intra-turn choices, will suppress until first new choice");
            }
        }
        // Always set resumed flag when loading from snapshot (even if 0 intra-turn choices)
        self.resumed_from_snapshot = true;
        // Track which turn we resumed into (use turns_elapsed since that's the turn we're in)
        self.resumed_turn_number = Some(self.turns_elapsed);
        if self.verbosity >= VerbosityLevel::Verbose && self.should_print_to_stdout() {
            println!(
                "📸 RESUMED FROM SNAPSHOT into turn {} (resumed_from_snapshot flag set)",
                self.turns_elapsed + 1
            );
        }
        self
    }

    /// Enable verbose output (deprecated, use with_verbosity)
    #[deprecated(note = "Use with_verbosity instead")]
    pub fn with_verbose(mut self, verbose: bool) -> Self {
        let verbosity = if verbose {
            VerbosityLevel::Verbose
        } else {
            VerbosityLevel::Silent
        };
        self.verbosity = verbosity;
        self.game.logger.set_verbosity(verbosity);
        self
    }

    /// Reset the game loop state (turn counter, step header flag)
    ///
    /// Call this after rewinding game state to prepare for replay.
    /// Note: This does NOT reset the underlying GameState - use game.undo() for that.
    pub fn reset(&mut self) {
        self.turns_elapsed = 0;
        self.step_header_printed = false;
        self.game.spell_targets.clear();
        self.choice_counter = 0;
        self.game.logger.reset_step_header();
    }

    /// Run the game loop with the given player controllers
    ///
    /// Returns when the game reaches a win condition or turn limit
    ///
    /// # Errors
    ///
    /// Returns an error if game setup fails or a fatal game state error occurs.
    pub fn run_game(
        &mut self,
        controller1: &mut dyn PlayerController,
        controller2: &mut dyn PlayerController,
    ) -> Result<GameResult> {
        // Setup: verify controllers and shuffle libraries
        let (player1_id, player2_id) = self.setup_game(controller1, controller2)?;

        // Main game loop - repeatedly run turns until game ends
        loop {
            // Run one turn and check if game should end
            if let Some(result) = self.run_turn_once(controller1, controller2)? {
                // Check if this is a snapshot request
                #[cfg(feature = "native")]
                if result.end_reason == GameEndReason::Snapshot {
                    // We're at the top level - save snapshot with access to both controllers!

                    // Determine which snapshot type and path to use
                    let (choice_count, snapshot_path) =
                        if let Some((_, ref stop_condition, ref path)) = self.stop_condition_info {
                            // --stop-on-choice snapshot
                            (stop_condition.choice_count, path.clone())
                        } else if let Some(ref path) = self.snapshot_path_for_fixed {
                            // --stop-when-fixed-exhausted snapshot
                            (self.choice_counter as usize, path.clone())
                        } else {
                            // Should never happen, but handle gracefully
                            return Ok(result);
                        };

                    return self.save_snapshot_and_exit(
                        choice_count,
                        &snapshot_path,
                        self.snapshot_format,
                        controller1,
                        controller2,
                    );
                }

                // Notify controllers of game end
                self.notify_game_end(controller1, controller2, player1_id, player2_id, result.winner);
                return Ok(result);
            }
        }
    }

    /// Run a bounded number of turns
    ///
    /// This is a convenience method for testing that runs up to `turns_to_run` turns,
    /// stopping early if the game ends.
    ///
    /// Returns:
    /// - `Ok(GameResult)` with the game outcome if the game ended
    /// - `Ok(GameResult)` with `GameEndReason::Manual` if all turns completed without ending
    ///
    /// # Errors
    ///
    /// Returns an error if a fatal game state error occurs during turn execution.
    pub fn run_turns(
        &mut self,
        controller1: &mut dyn PlayerController,
        controller2: &mut dyn PlayerController,
        turns_to_run: u32,
    ) -> Result<GameResult> {
        for _ in 0..turns_to_run {
            if let Some(result) = self.run_turn_once(controller1, controller2)? {
                // Game ended, return the result
                return Ok(result);
            }
        }

        // Completed all turns without game ending
        Ok(GameResult {
            winner: None,
            turns_played: self.turns_elapsed,
            end_reason: GameEndReason::Manual,
            action_count: self.game.action_count(),
        })
    }

    /// Run game until completion or human input is needed
    ///
    /// This is the main entry point for WASM games with human players.
    /// It runs the game loop until either:
    /// - The game ends (returns `GameLoopState::Complete`)
    /// - A human player needs to make a choice (returns `GameLoopState::AwaitingInput`)
    ///
    /// ## Usage Pattern (WASM)
    ///
    /// ```ignore
    /// // Initial run
    /// let state = game_loop.run_until_input(&mut human, &mut ai)?;
    ///
    /// match state {
    ///     GameLoopState::Complete(result) => {
    ///         // Game ended, show result
    ///     }
    ///     GameLoopState::AwaitingInput(context) => {
    ///         // Display choices to user
    ///         // Wait for user input...
    ///         // When user chooses:
    ///         human.set_pending_choice(PendingChoice::SpellAbility(Some(idx)));
    ///         // Call run_until_input again to continue
    ///     }
    /// }
    /// ```
    ///
    /// ## How It Works
    ///
    /// When a `WasmHumanController` returns `ChoiceResult::NeedInput`, the macros
    /// convert this to `MtgError::NeedInput`. This method catches that error and
    /// returns `GameLoopState::AwaitingInput` instead.
    ///
    /// The caller should:
    /// 1. Display the choice context to the user
    /// 2. Wait for user input (via JavaScript events)
    /// 3. Set the pending choice on the controller
    /// 4. Call this method again to continue
    ///
    /// Returns:
    /// - `Ok(GameLoopState::Complete(result))` when game ends
    /// - `Ok(GameLoopState::AwaitingInput(context))` when human input is needed
    /// - `Err(_)` on actual errors
    ///
    /// # Errors
    ///
    /// Returns an error for fatal game state errors (not for awaiting input).
    pub fn run_until_input(
        &mut self,
        controller1: &mut dyn PlayerController,
        controller2: &mut dyn PlayerController,
    ) -> Result<GameLoopState> {
        // Try to run the game, catching NeedInput as a special case
        match self.run_game(controller1, controller2) {
            Ok(result) => Ok(GameLoopState::Complete(result)),
            Err(MtgError::NeedInput(context)) => Ok(GameLoopState::AwaitingInput(*context)),
            Err(e) => Err(e),
        }
    }

    /// Run exactly one turn of the game
    ///
    /// This is used for step-through mode in WASM TUI (AI vs AI games).
    ///
    /// Returns:
    /// - `Ok(Some(GameResult))` if the game ended during this turn
    /// - `Ok(None)` if the turn completed and the game continues
    /// - `Err(_)` on error
    ///
    /// # Errors
    ///
    /// Returns an error if game setup or turn execution fails.
    pub fn run_one_turn(
        &mut self,
        controller1: &mut dyn PlayerController,
        controller2: &mut dyn PlayerController,
    ) -> Result<Option<GameResult>> {
        // Setup on first turn (when undo log is empty and not already set up)
        let is_fresh_start = self.game.undo_log.actions().is_empty() && self.game.turn.turn_number <= 1;
        if is_fresh_start {
            // Verify controllers match players
            let (player1_id, player2_id) = {
                let mut players_iter = self.game.players.iter().map(|p| p.id);
                let player1_id = players_iter
                    .next()
                    .ok_or_else(|| MtgError::InvalidAction("Game loop requires exactly 2 players".to_string()))?;
                let player2_id = players_iter
                    .next()
                    .ok_or_else(|| MtgError::InvalidAction("Game loop requires exactly 2 players".to_string()))?;
                (player1_id, player2_id)
            };

            if controller1.player_id() != player1_id || controller2.player_id() != player2_id {
                return Err(MtgError::InvalidAction(
                    "Controller player IDs don't match game players".to_string(),
                ));
            }
        }

        // Run one turn
        self.run_turn_once(controller1, controller2)
    }

    /// Set up a game for two-player gameplay
    ///
    /// This verifies that:
    /// - Exactly 2 players exist in the game
    /// - Controllers match the player IDs
    /// - Libraries are shuffled using the game's RNG seed (unless resuming from snapshot)
    ///
    /// Returns the player IDs for both players.
    fn setup_game(
        &mut self,
        controller1: &mut dyn PlayerController,
        controller2: &mut dyn PlayerController,
    ) -> Result<(PlayerId, PlayerId)> {
        // Verify controllers match players (extract exactly 2 player IDs without allocating)
        let (player1_id, player2_id) = {
            let mut players_iter = self.game.players.iter().map(|p| p.id);
            let player1_id = players_iter
                .next()
                .ok_or_else(|| MtgError::InvalidAction("Game loop requires exactly 2 players".to_string()))?;
            let player2_id = players_iter
                .next()
                .ok_or_else(|| MtgError::InvalidAction("Game loop requires exactly 2 players".to_string()))?;
            if players_iter.next().is_some() {
                return Err(MtgError::InvalidAction(
                    "Game loop requires exactly 2 players".to_string(),
                ));
            }
            (player1_id, player2_id)
        };

        if controller1.player_id() != player1_id || controller2.player_id() != player2_id {
            return Err(MtgError::InvalidAction(
                "Controller player IDs don't match game players".to_string(),
            ));
        }

        // Only shuffle libraries and draw opening hands for fresh games
        // Skip for:
        // - Snapshot resume (has actions in undo log)
        // - Puzzle-loaded games (hands/battlefield already set up)
        // - Network mode (server handles setup, skip_opening_hands is true)
        let is_resuming_from_snapshot = !self.game.undo_log.actions().is_empty();

        // Detect puzzle-loaded games: they have turn > 1 or cards already in zones other than library
        let player_ids_for_check = [player1_id, player2_id];
        let has_cards_in_play = !self.game.battlefield.cards.is_empty()
            || player_ids_for_check.iter().any(|&pid| {
                if let Some(zones) = self.game.get_player_zones(pid) {
                    !zones.hand.cards.is_empty() || !zones.graveyard.cards.is_empty()
                } else {
                    false
                }
            });
        let is_puzzle_game = self.game.turn.turn_number > 1 || has_cards_in_play;

        if !is_resuming_from_snapshot && !is_puzzle_game {
            if self.skip_opening_hands {
                // Positional ID mode or network mode: library already shuffled.
                // Skip the shuffle but still handle controlled hand setup if specified.
                self.sync_to_action();

                log::debug!(
                    "Skip opening hands mode: undo_log before draws = {}",
                    self.game.undo_log.len()
                );

                // Handle controlled hands if configured, otherwise draw 7 randomly
                if self.p1_hand_setup.is_some() || self.p2_hand_setup.is_some() {
                    // Use controlled hand setup (without shuffle)
                    for (idx, &player_id) in [player1_id, player2_id].iter().enumerate() {
                        let setup = match idx {
                            0 => self.p1_hand_setup.as_ref(),
                            1 => self.p2_hand_setup.as_ref(),
                            _ => None,
                        };

                        if let Some(hand_setup) = setup {
                            // Find and move specific cards from library to hand
                            for card_name in &hand_setup.specific_cards {
                                let card_id = {
                                    let zones = self.game.get_player_zones(player_id).ok_or_else(|| {
                                        crate::MtgError::InvalidAction(format!("Player {:?} not found", player_id))
                                    })?;

                                    let matching_card = zones.library.cards.iter().find(|&&cid| {
                                        self.game
                                            .cards
                                            .get(cid)
                                            .map(|card| card.name.as_str() == card_name.as_str())
                                            .unwrap_or(false)
                                    });

                                    match matching_card {
                                        Some(&id) => id,
                                        None => {
                                            return Err(crate::MtgError::InvalidAction(format!(
                                                "Card '{}' not found in player {:?}'s library",
                                                card_name, player_id
                                            )));
                                        }
                                    }
                                };

                                // Remove from library and add to hand
                                let zones = self.game.get_player_zones_mut(player_id).ok_or_else(|| {
                                    crate::MtgError::InvalidAction(format!("Player {:?} not found", player_id))
                                })?;
                                zones.library.remove(card_id);
                                zones.hand.add(card_id);
                            }

                            // Draw remaining cards randomly to reach 7 total (opening hands don't trigger)
                            // Use draw_card_silent so opening-hand draws aren't surfaced as
                            // "P draws CARD (id)" gamelog noise (see bug-bazaar-no-draw fix).
                            let cards_in_hand = hand_setup.specific_cards.len();
                            let remaining_to_draw = 7usize.saturating_sub(cards_in_hand);
                            for _ in 0..remaining_to_draw {
                                let _ = self.game.draw_card_silent(player_id)?;
                            }
                        } else {
                            // No controlled setup, draw 7 cards normally (opening hands don't trigger)
                            for _ in 0..7 {
                                let _ = self.game.draw_card_silent(player_id)?;
                            }
                        }
                    }
                } else {
                    // No controlled hand setup, just draw 7 cards for each player (opening hands don't trigger)
                    for &player_id in &[player1_id, player2_id] {
                        for _ in 0..7 {
                            let _ = self.game.draw_card_silent(player_id)?;
                        }
                    }
                }

                log::debug!(
                    "Skip opening hands mode: undo_log after draws = {}",
                    self.game.undo_log.len()
                );
            } else {
                // Normal mode: shuffle and draw opening hands locally

                // If a separate deck seed is configured, apply it before shuffling
                // This allows sampling different games (via --seed) with the same initial hands (--deck-seed)
                if let Some(deck_seed) = self.deck_seed {
                    self.game.seed_rng(deck_seed);
                }

                // Setup opening hands using unified hand setup logic (MTG Rules 103.2-103.4)
                // This handles shuffling, drawing, and optional controlled hand setup for testing
                // TODO(mtg-102): Implement mulligan system (MTG Rules 103.5)
                let player_ids: [PlayerId; 2] = [player1_id, player2_id];
                crate::game::setup_opening_hands(
                    self.game,
                    &player_ids,
                    self.p1_hand_setup.as_ref(),
                    self.p2_hand_setup.as_ref(),
                )?;

                // If a game seed is configured (different from deck seed), re-seed after shuffling
                // This allows the game to proceed with a different RNG stream than was used for shuffling
                if let Some(game_seed) = self.game_seed {
                    self.game.seed_rng(game_seed);
                }
            }

            // Log the start of Turn 1 (for fresh games only)
            self.emit_turn_one_header();
        }

        Ok((player1_id, player2_id))
    }

    /// Emit the ">>> Turn 1 - ..." gamelog header (idempotent).
    ///
    /// Subsequent turn headers (Turn 2+) are emitted by `GameState::next_turn()`
    /// when transitioning between turns. Turn 1 has no preceding `next_turn()`
    /// call, so this helper handles the special case. Idempotent via the
    /// `turn_one_header_emitted` flag so it can safely be invoked from both
    /// `setup_game()` (run_game path) and `run_turn()` (run_turns / run_one_turn
    /// paths used by WASM step-through and direct test harnesses).
    fn emit_turn_one_header(&mut self) {
        if self.turn_one_header_emitted {
            return;
        }
        let active_player = self.game.turn.active_player;
        let active_player_name = self
            .game
            .get_player(active_player)
            .map(|p| p.name.as_str())
            .unwrap_or("Unknown");
        let active_player_life = self.game.get_player(active_player).map(|p| p.life).unwrap_or(0);
        let other_player_name = self
            .game
            .get_other_player_id(active_player)
            .and_then(|id| self.game.get_player(id).ok())
            .map(|p| p.name.as_str())
            .unwrap_or("Unknown");
        let other_player_life = self
            .game
            .get_other_player_id(active_player)
            .and_then(|id| self.game.get_player(id).ok())
            .map(|p| p.life)
            .unwrap_or(0);

        let turn_msg = format!(
            "  >>> Turn 1 - {} {} ({} {}) <<<<",
            active_player_name, active_player_life, other_player_name, other_player_life
        );
        self.game.logger.turn_separator(&turn_msg);
        self.turn_one_header_emitted = true;
    }

    /// Notify both controllers that the game has ended
    ///
    /// Calls the `on_game_end` callback for each controller with their view
    /// of the game state and whether they won.
    fn notify_game_end(
        &self,
        controller1: &mut dyn PlayerController,
        controller2: &mut dyn PlayerController,
        player1_id: PlayerId,
        player2_id: PlayerId,
        winner_id: Option<PlayerId>,
    ) {
        controller1.on_game_end(
            &GameStateView::new(self.game, player1_id),
            winner_id == Some(player1_id),
        );
        controller2.on_game_end(
            &GameStateView::new(self.game, player2_id),
            winner_id == Some(player2_id),
        );
    }

    /// Run a single turn and check for game-ending conditions
    ///
    /// This method runs exactly one turn of the game, including all phases and steps.
    /// After the turn completes, it checks for win conditions and turn limits.
    ///
    /// Returns:
    /// - `Ok(Some(GameResult))` if the game should end (win condition or turn limit reached)
    /// - `Ok(None)` if the game should continue with another turn
    /// - `Err(_)` if an error occurred during turn execution
    ///
    /// # Errors
    ///
    /// Returns an error if turn execution encounters a fatal game state error.
    pub fn run_turn_once(
        &mut self,
        controller1: &mut dyn PlayerController,
        controller2: &mut dyn PlayerController,
    ) -> Result<Option<GameResult>> {
        // Check win conditions before running the turn
        if let Some(result) = self.check_win_condition() {
            return Ok(Some(result));
        }

        // Check turn limit
        if self.turns_elapsed >= self.max_turns {
            return Ok(Some(GameResult {
                winner: None,
                turns_played: self.turns_elapsed,
                end_reason: GameEndReason::TurnLimit,
                action_count: self.game.action_count(),
            }));
        }

        // Run the turn
        if let Some(result) = self.run_turn(controller1, controller2)? {
            // Mid-turn snapshot triggered
            return Ok(Some(result));
        }
        self.turns_elapsed += 1;

        // Check win conditions after running the turn
        if let Some(result) = self.check_win_condition() {
            return Ok(Some(result));
        }

        // Game continues
        Ok(None)
    }

    /// Run a single turn through all its phases and steps
    ///
    /// This is an internal method that executes one complete turn from untap through cleanup.
    /// For running one turn and checking end conditions, use `run_turn_once` instead.
    fn run_turn(
        &mut self,
        controller1: &mut dyn PlayerController,
        controller2: &mut dyn PlayerController,
    ) -> Result<Option<GameResult>> {
        let active_player = self.game.turn.active_player;

        // Check if we're in the resumed turn (skip header) or a new turn (print header)
        let is_resumed_turn = self.resumed_turn_number == Some(self.turns_elapsed);

        // Skip turn header ONLY if we're in the resumed turn (it was already printed before snapshot)
        // Note: We intentionally do NOT check self.replaying here, because replaying can span
        // multiple turns and we want to print headers for new turns even during replay.
        //
        // IMPORTANT: We print ONLY the turn header here, NOT the battlefield state.
        // The battlefield state (including hand contents) is printed AFTER the draw step
        // so that newly drawn cards are visible. See draw_step() in steps.rs.
        if self.verbosity >= VerbosityLevel::Normal && !is_resumed_turn {
            let player_name = self.get_player_name(active_player);

            // Debug: Log state hash before turn header
            let turn_msg = format!("Turn {} - {}'s turn", self.turns_elapsed + 1, player_name);
            self.game.debug_log_state_hash(&turn_msg);

            if self.should_print_to_stdout() {
                println!("\n========================================");
                println!("{}", turn_msg);
                println!("========================================");
                // NOTE: Battlefield state is printed after draw step - see draw_step()
            }

            // Emit the ">>> Turn 1 <<<<" gamelog header for entry paths that
            // bypass setup_game (run_turns / WASM run_one_turn). Idempotent:
            // does nothing if setup_game already emitted it.
            if self.game.turn.turn_number == 1 {
                self.emit_turn_one_header();
            }
        }

        // Suppress turn header ONLY if we're in the resumed turn (it was already printed before snapshot)
        if is_resumed_turn && self.verbosity >= VerbosityLevel::Verbose && self.should_print_to_stdout() {
            println!("🔄 RESUMING TURN {} (will suppress header)", self.turns_elapsed + 1);
        }

        // Reset turn-based state (guarded to prevent re-entry when WASM harness recreates
        // GameLoop mid-turn: re-running reset_turn_state would zero lands_played_this_turn,
        // causing the local WASM client to offer PlayLand when the server correctly denies it)
        let turn_num = self.game.turn.turn_number;
        if self.game.turn.turn_state_reset_turn != Some(turn_num) {
            self.game.turn.turn_state_reset_turn = Some(turn_num);
            self.reset_turn_state(active_player)?;
        }

        // Run through all steps of the turn
        loop {
            // Record the step before execution to detect if undo changes it
            let step_before = self.game.turn.current_step;
            let turn_before = self.game.turn.turn_number;

            // Execute the step
            if let Some(result) = self.execute_step(controller1, controller2)? {
                // Mid-turn snapshot triggered (e.g., fixed controller exhausted)
                return Ok(Some(result));
            }

            // Check if the step/turn changed during execution (undo happened)
            let step_after = self.game.turn.current_step;
            let turn_after = self.game.turn.turn_number;

            if step_after != step_before || turn_after != turn_before {
                // Step or turn changed during execution - undo must have occurred
                // Don't advance, just loop again to re-execute from the rewound state
                eprintln!(
                    "[STEP LOOP] Undo detected: step changed from {:?} to {:?}, turn {} to {}",
                    step_before, step_after, turn_before, turn_after
                );
                continue;
            }

            // Try to advance to next step
            // IMPORTANT: Call game.advance_step() not turn.advance_step()
            // to ensure step changes are logged to undo log
            self.game.advance_step()?;

            // Check if we reached end of turn
            if self.game.turn.current_step == crate::game::Step::Untap {
                // We wrapped back to Untap, which means a new turn started
                // The turn change was already logged by advance_step()

                // Clear resumed tracking after we finish the resumed turn
                if is_resumed_turn {
                    if self.verbosity >= VerbosityLevel::Verbose && self.should_print_to_stdout() {
                        println!(
                            "✅ FINISHING RESUMED TURN {} (will clear resumed tracking)",
                            self.turns_elapsed
                        );
                    }
                    self.resumed_from_snapshot = false;
                    self.resumed_turn_number = None;

                    // Also clear replay mode at end of resumed turn
                    // This handles the case where all intra-turn choices have been replayed
                    // but we haven't yet reached the next choice point (e.g., turn ended naturally)
                    //
                    // Only clear if we've actually moved past the baseline (made new choices)
                    // If choice_counter is still at baseline, we didn't make any new choices this turn
                    // and should keep replaying mode active for the next turn
                    if self.replaying && (self.choice_counter as usize) >= self.baseline_choice_count {
                        if self.verbosity >= VerbosityLevel::Verbose && self.should_print_to_stdout() {
                            println!("✅ CLEARING REPLAY MODE at end of resumed turn");
                        }
                        self.replaying = false;
                        self.replay_choices_remaining = 0;
                    }
                }

                break;
            }
        }

        Ok(None)
    }

    /// Reset turn-based state for the active player
    fn reset_turn_state(&mut self, active_player: PlayerId) -> Result<()> {
        // Reset lands played this turn
        if let Ok(player) = self.game.get_player_mut(active_player) {
            player.reset_lands_played();
        }

        // Empty mana pools at start of turn
        // Use fixed-size array instead of Vec allocation (MTG always has 2 players)
        let player_ids: [PlayerId; 2] = [self.game.players[0].id, self.game.players[1].id];
        for player_id in player_ids {
            if let Ok(player) = self.game.get_player_mut(player_id) {
                player.mana_pool.clear();
            }
        }

        // Reset planeswalker loyalty activation flag (MTG CR 606.3)
        // Each planeswalker can have one loyalty ability activated per turn.
        for &card_id in &self.game.battlefield.cards {
            if let Ok(card) = self.game.cards.get_mut(card_id) {
                card.loyalty_activated_this_turn = false;
            }
        }

        Ok(())
    }

    /// Execute a single step
    ///
    /// # Errors
    ///
    /// Returns an error if step execution encounters a fatal game state error.
    pub fn execute_step(
        &mut self,
        controller1: &mut dyn PlayerController,
        controller2: &mut dyn PlayerController,
    ) -> Result<Option<GameResult>> {
        let step = self.game.turn.current_step;
        let turn = self.game.turn.turn_number;

        // Reset step header tracking for each new step
        self.step_header_printed = false;

        // Update logger's gamelog context (turn and step)
        self.game.logger.set_gamelog_turn(turn);
        self.game.logger.set_gamelog_step(step.abbreviation());

        // In verbose mode, always print step header immediately
        if self.verbosity >= VerbosityLevel::Verbose && !self.replaying {
            println!("--- {} ---", self.step_name(step));
        }

        match step {
            Step::Untap => self.untap_step(controller1, controller2),
            Step::Upkeep => self.upkeep_step(controller1, controller2),
            Step::Draw => self.draw_step(controller1, controller2),
            Step::Main1 | Step::Main2 => self.main_phase(controller1, controller2),
            Step::BeginCombat => self.begin_combat_step(controller1, controller2),
            Step::DeclareAttackers => self.declare_attackers_step(controller1, controller2),
            Step::DeclareBlockers => self.declare_blockers_step(controller1, controller2),
            Step::CombatDamage => self.combat_damage_step(controller1, controller2),
            Step::EndCombat => self.end_combat_step(controller1, controller2),
            Step::End => self.end_step(controller1, controller2),
            Step::Cleanup => self.cleanup_step(controller1, controller2),
        }
    }

    /// Check if the game has reached a win condition
    fn check_win_condition(&mut self) -> Option<GameResult> {
        // Check for player death (life <= 0)
        for player in &self.game.players {
            if player.life <= 0 {
                let loser_id = player.id;
                let loser_name = player.name.clone();
                let loser_life = player.life;
                let winner = self.game.get_other_player_id(loser_id);
                let winner_name = winner
                    .and_then(|id| self.game.get_player(id).ok())
                    .map(|p| p.name.clone())
                    .unwrap_or_else(|| "Unknown".to_string().into());

                // Log the game end
                self.game.logger.gamelog(&format!(
                    "{loser_name} has lost the game (life: {loser_life}). {winner_name} wins!"
                ));

                return Some(GameResult {
                    winner,
                    turns_played: self.turns_elapsed,
                    end_reason: GameEndReason::PlayerDeath(loser_id),
                    action_count: self.game.action_count(),
                });
            }
        }

        // Check for decking (empty library when trying to draw)
        for player in &self.game.players {
            if let Some(zones) = self.game.get_player_zones(player.id) {
                if zones.library.is_empty() {
                    let loser_id = player.id;
                    let loser_name = player.name.clone();
                    let winner = self.game.get_other_player_id(loser_id);
                    let winner_name = winner
                        .and_then(|id| self.game.get_player(id).ok())
                        .map(|p| p.name.clone())
                        .unwrap_or_else(|| "Unknown".to_string().into());

                    // Log the game end
                    self.game
                        .logger
                        .gamelog(&format!("{loser_name} has lost the game (decked). {winner_name} wins!"));

                    return Some(GameResult {
                        winner,
                        turns_played: self.turns_elapsed,
                        end_reason: GameEndReason::Decking(loser_id),
                        action_count: self.game.action_count(),
                    });
                }
            }
        }

        None
    }
}

// Test helpers - intentionally exposed (not behind cfg(test)) so external
// integration tests in `mtg-engine/tests/` can drive the engine without
// reaching into private state. The `_for_test` suffix marks these as
// helpers and they do not appear in production controller code paths.
impl<'a> GameLoop<'a> {
    /// Expose push_activatable_abilities for testing summoning sickness checks
    pub fn push_activatable_abilities_for_test(&mut self, player_id: PlayerId) {
        self.abilities_buffer.clear();
        self.push_activatable_abilities(player_id);
    }

    /// Get a reference to the abilities buffer for test assertions
    pub fn get_abilities_buffer(&self) -> &[crate::core::SpellAbility] {
        &self.abilities_buffer
    }

    /// Expose the untap step for testing "doesn't untap" locks (Paralyze,
    /// Exhaustion, ...). Runs the same untap_step the turn loop runs.
    ///
    /// # Errors
    /// Returns an error if the untap step encounters an invalid game state.
    pub fn untap_step_for_test(
        &mut self,
        controller1: &mut dyn crate::game::controller::PlayerController,
        controller2: &mut dyn crate::game::controller::PlayerController,
    ) -> crate::Result<Option<crate::game::GameResult>> {
        self.untap_step(controller1, controller2)
    }

    /// Expose cleanup_step for testing discard logic
    ///
    /// # Errors
    /// Returns an error if the cleanup step encounters an invalid game state.
    pub fn cleanup_step_for_test(
        &mut self,
        controller1: &mut dyn crate::game::controller::PlayerController,
        controller2: &mut dyn crate::game::controller::PlayerController,
    ) -> crate::Result<Option<crate::game::GameResult>> {
        self.cleanup_step(controller1, controller2)
    }

    /// Test hook: list creatures the declare-attackers step would offer for
    /// `player_id`. Mirrors the filter inside `declare_attackers_step` exactly
    /// (delegates to the same private helper) so regression tests can verify
    /// e.g. that an animated Mishra's Factory shows up as an attacker.
    pub fn get_available_attacker_creatures_for_test(
        &self,
        player_id: crate::core::PlayerId,
    ) -> smallvec::SmallVec<[crate::core::CardId; 8]> {
        self.get_available_attacker_creatures(player_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_game_loop_creation() {
        let mut game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
        let _game_loop = GameLoop::new(&mut game);
    }

    #[test]
    fn test_untap_step() {
        use crate::game::ZeroController;

        let mut game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
        let (alice, bob) = {
            let mut players_iter = game.players.iter().map(|p| p.id);
            (
                players_iter.next().expect("Should have player 1"),
                players_iter.next().expect("Should have player 2"),
            )
        };

        // Create a tapped land on battlefield
        let land_id = game.next_card_id();
        let mut land = crate::core::Card::new(land_id, "Mountain".to_string(), alice);
        land.types.push(crate::core::CardType::Land);
        land.tap();
        game.cards.insert(land_id, land);
        game.battlefield.add(land_id);

        // Run untap step with controllers
        {
            let mut game_loop = GameLoop::new(&mut game);
            let mut controller1 = ZeroController::new(alice);
            let mut controller2 = ZeroController::new(bob);
            game_loop.untap_step(&mut controller1, &mut controller2).unwrap();
        } // game_loop is dropped here, releasing borrow of game

        // Land should now be untapped
        let land = game.cards.get(land_id).unwrap();
        assert!(!land.tapped);
    }

    #[test]
    fn test_draw_step() {
        let mut game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
        let (alice, bob) = {
            let mut players_iter = game.players.iter().map(|p| p.id);
            (
                players_iter.next().expect("Should have player 1"),
                players_iter.next().expect("Should have player 2"),
            )
        };

        // Add a card to Player1's library
        let card_id = game.next_card_id();
        let card = crate::core::Card::new(card_id, "Test Card".to_string(), alice);
        game.cards.insert(card_id, card);
        if let Some(zones) = game.get_player_zones_mut(alice) {
            zones.library.add(card_id);
        }

        // Set turn to 2 (so draw happens)
        game.turn.turn_number = 2;

        // Create mock controllers
        let mut controller1 = crate::game::ZeroController::new(alice);
        let mut controller2 = crate::game::ZeroController::new(bob);

        // Run draw step
        {
            let mut game_loop = GameLoop::new(&mut game);
            game_loop.draw_step(&mut controller1, &mut controller2).unwrap();
        } // game_loop is dropped here, releasing borrow of game

        // Card should be in hand
        if let Some(zones) = game.get_player_zones(alice) {
            assert!(zones.hand.contains(card_id));
            assert!(!zones.library.contains(card_id));
        }
    }

    #[test]
    fn test_check_win_condition_life() {
        let mut game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
        let bob = {
            let mut players_iter = game.players.iter().map(|p| p.id);
            let _alice = players_iter.next().expect("Should have player 1");
            players_iter.next().expect("Should have player 2")
        };

        // Set Player2's life to 0
        game.get_player_mut(bob).unwrap().life = 0;

        let mut game_loop = GameLoop::new(&mut game);
        let result = game_loop.check_win_condition();

        assert!(result.is_some());
        let result = result.unwrap();
        assert_eq!(result.end_reason, GameEndReason::PlayerDeath(bob));
    }

    /// Regression test: Turn 1 header must appear in the gamelog regardless of
    /// which entry path runs the first turn. WASM step-through (`run_turns`)
    /// and direct `run_one_turn` calls both bypass `setup_game`, so the
    /// `>>> Turn 1 - ... <<<<` header must be emitted from `run_turn` as a
    /// fallback. Without this, the game log shows Turn 2+ headers but no
    /// Turn 1 header (bug in tui_game.html, native_game.html, native TUI logs).
    #[test]
    fn test_turn_one_header_emitted_via_run_turns() {
        use crate::game::ZeroController;

        let mut game = GameState::new_two_player("Alice".to_string(), "Bob".to_string(), 20);
        let (alice, bob) = {
            let mut players_iter = game.players.iter().map(|p| p.id);
            (
                players_iter.next().expect("Should have player 1"),
                players_iter.next().expect("Should have player 2"),
            )
        };

        // Give each player a tiny library so draw step succeeds.
        for &pid in &[alice, bob] {
            for _ in 0..10 {
                let card_id = game.next_card_id();
                let card = crate::core::Card::new(card_id, "Forest".to_string(), pid);
                game.cards.insert(card_id, card);
                if let Some(zones) = game.get_player_zones_mut(pid) {
                    zones.library.add(card_id);
                }
            }
        }

        // Use the WASM-style entry path (run_turns) which bypasses setup_game.
        {
            let mut game_loop = GameLoop::new(&mut game).with_max_turns(2);
            let mut c1 = ZeroController::new(alice);
            let mut c2 = ZeroController::new(bob);
            let _ = game_loop.run_turns(&mut c1, &mut c2, 1);
        }

        let logs: Vec<String> = game.logger.logs().iter().map(|l| l.message.clone()).collect();
        let turn_one_headers: Vec<&String> = logs.iter().filter(|m| m.contains(">>> Turn 1")).collect();
        assert_eq!(
            turn_one_headers.len(),
            1,
            "Expected exactly one Turn 1 header in gamelog, found {}. Log entries: {:?}",
            turn_one_headers.len(),
            logs
        );
    }

    /// Companion test: the run_game path (used by native TUI and WasmGame::run_ai_game)
    /// must also emit exactly ONE Turn 1 header (not zero, not two). This guards against
    /// the helper firing twice when both setup_game and run_turn invoke it.
    #[test]
    fn test_turn_one_header_emitted_exactly_once_via_run_game() {
        use crate::game::ZeroController;

        let mut game = GameState::new_two_player("Alice".to_string(), "Bob".to_string(), 20);
        let (alice, bob) = {
            let mut players_iter = game.players.iter().map(|p| p.id);
            (
                players_iter.next().expect("Should have player 1"),
                players_iter.next().expect("Should have player 2"),
            )
        };

        for &pid in &[alice, bob] {
            for _ in 0..10 {
                let card_id = game.next_card_id();
                let card = crate::core::Card::new(card_id, "Forest".to_string(), pid);
                game.cards.insert(card_id, card);
                if let Some(zones) = game.get_player_zones_mut(pid) {
                    zones.library.add(card_id);
                }
            }
        }

        {
            let mut game_loop = GameLoop::new(&mut game).with_max_turns(2).skip_opening_hands();
            let mut c1 = ZeroController::new(alice);
            let mut c2 = ZeroController::new(bob);
            let _ = game_loop.run_game(&mut c1, &mut c2);
        }

        let logs: Vec<String> = game.logger.logs().iter().map(|l| l.message.clone()).collect();
        let turn_one_headers: Vec<&String> = logs.iter().filter(|m| m.contains(">>> Turn 1")).collect();
        assert_eq!(
            turn_one_headers.len(),
            1,
            "Expected exactly one Turn 1 header in gamelog (no duplicates), found {}",
            turn_one_headers.len()
        );
    }
}
