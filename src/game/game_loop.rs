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

use crate::core::{CardId, PlayerId};
use crate::game::controller::PlayerController;
use crate::game::controller::{
    format_attackers_prompt, format_blockers_prompt, format_choice_menu, format_discard_prompt, GameStateView,
};
use crate::game::phase::Step;
use crate::game::GameState;
use crate::{MtgError, Result};

// Legacy v1 action type (kept for compatibility with dead code)
#[allow(dead_code)]
#[derive(Debug, Clone)]
enum PlayerAction {
    PlayLand(CardId),
    CastSpell { card_id: CardId, targets: Vec<CardId> },
    TapForMana(CardId),
    DeclareAttacker(CardId),
    DeclareBlocker { blocker: CardId, attackers: Vec<CardId> },
    FinishDeclareAttackers,
    FinishDeclareBlockers,
    PassPriority,
}
use smallvec::SmallVec;

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
}

/// Reason the game ended
#[derive(Debug, Clone, PartialEq, Eq)]
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
    /// Track targets for spells on the stack (spell_id -> chosen_targets)
    /// This is needed because targets are chosen at cast time but used at resolution time
    spell_targets: Vec<(CardId, Vec<CardId>)>,
    /// Global choice counter for tracking all player choices
    /// Increments each time a controller makes any decision
    choice_counter: u32,
    /// Reusable mana engine for checking mana availability
    /// Updated per-player as needed, retains Vec capacity across calls
    mana_engine: crate::game::mana_engine::ManaEngine,
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
    /// The turn number we resumed into (used to suppress header for that specific turn only)
    resumed_turn_number: Option<u32>,
    /// Optional hand setup for Player 1 (controlled initial hand)
    p1_hand_setup: Option<crate::game::HandSetup>,
    /// Optional hand setup for Player 2 (controlled initial hand)
    p2_hand_setup: Option<crate::game::HandSetup>,
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
            spell_targets: Vec::new(),
            choice_counter: 0,
            mana_engine: crate::game::mana_engine::ManaEngine::new(),
            stop_when_fixed_exhausted: false,
            snapshot_path_for_fixed: None,
            snapshot_format: crate::game::snapshot::SnapshotFormat::default(),
            stop_condition_info: None,
            baseline_choice_count: 0,
            replaying: false,
            replay_choices_remaining: 0,
            resumed_from_snapshot: false,
            resumed_turn_number: None,
            p1_hand_setup: None,
            p2_hand_setup: None,
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
        self.spell_targets.clear();
        self.choice_counter = 0;
        self.game.logger.reset_step_header();
    }

    /// Log a choice point to the undo log and increment choice counter
    ///
    /// Call this every time a controller makes a decision.
    ///
    /// # Arguments
    /// * `player_id` - The player who made the choice
    /// * `choice` - The actual choice made (for replay), or None if not available
    fn log_choice_point(&mut self, player_id: PlayerId, choice: Option<crate::game::ReplayChoice>) {
        self.choice_counter += 1;

        self.game.undo_log.log(crate::undo::GameAction::ChoicePoint {
            player_id,
            choice_id: self.choice_counter,
            choice,
        });

        // If we're in replay mode, decrement counter
        // Note: Replay mode stays active until ALL choices are replayed, then cleared before
        // presenting the NEXT choice. This is because snapshots are taken BEFORE presenting
        // a choice, so all choices in the snapshot were already made/executed/logged.
        if self.replaying && self.replay_choices_remaining > 0 {
            self.replay_choices_remaining -= 1;
            if self.verbosity >= VerbosityLevel::Verbose && self.should_print_to_stdout() {
                println!(
                    "🔄 Replay choice: {} remaining (suppressing logs)",
                    self.replay_choices_remaining
                );
            }
        }
    }

    /// Check if we should save a snapshot before asking for next controller choice
    ///
    /// This is the PREAMBLE check that happens BEFORE presenting a choice to the controller.
    /// This ensures snapshots pause the game at a clean point where an external agent can
    /// review the game state and make a decision when resuming.
    ///
    /// It checks two conditions:
    /// 1. If stop_when_fixed_exhausted is enabled and controller is out of choices
    /// 2. If stop condition is set and filtered choice count reached limit
    ///
    /// Returns Some(GameResult) if snapshot should be saved, None to continue.
    fn check_stop_conditions(
        &mut self,
        controller: &dyn PlayerController,
        player_id: PlayerId,
    ) -> Result<Option<GameResult>> {
        // Check 1: Fixed controller exhaustion
        if self.stop_when_fixed_exhausted && !controller.has_more_choices() && self.snapshot_path_for_fixed.is_some() {
            // Just signal - snapshot will be saved at top level
            return Ok(Some(GameResult {
                winner: None,
                turns_played: self.turns_elapsed,
                end_reason: GameEndReason::Snapshot,
            }));
        }

        // Check 2: Stop condition (--stop-on-choice)
        if let Some((p1_id, ref stop_condition, ref _snapshot_path)) = self.stop_condition_info {
            // Only count this choice if it matches the stop condition filter
            if stop_condition.applies_to(p1_id, player_id) {
                let filtered_count = self.count_filtered_choices(p1_id, stop_condition);

                // If we've reached the limit, signal to unwind control flow
                if filtered_count >= stop_condition.choice_count {
                    // Just return a signal - don't save yet!
                    return Ok(Some(GameResult {
                        winner: None,
                        turns_played: self.turns_elapsed,
                        end_reason: GameEndReason::Snapshot,
                    }));
                }
            }
        }

        Ok(None)
    }

    /// Run the game loop with the given player controllers
    ///
    /// Returns when the game reaches a win condition or turn limit
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
        })
    }

    /// Count how many choices in the undo log match the stop condition filter
    fn count_filtered_choices(&self, p1_id: PlayerId, stop_condition: &crate::game::StopCondition) -> usize {
        let total_count = self
            .game
            .undo_log
            .actions()
            .iter()
            .filter_map(|action| {
                if let crate::undo::GameAction::ChoicePoint { player_id, .. } = action {
                    if stop_condition.applies_to(p1_id, *player_id) {
                        Some(())
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .count();

        // Subtract baseline to get choices made since snapshot resume
        total_count.saturating_sub(self.baseline_choice_count)
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
        }

        Ok((player1_id, player2_id))
    }

    /// Assert that we're stopping at a valid point in the game
    ///
    /// Valid stopping points are:
    /// - After a controller choice (last log line contains ">>> CONTROLLER:")
    /// - At game end (last log line contains "Game Over" or "wins!")
    ///
    /// This helps catch bugs where we might stop mid-action (e.g., in the middle
    /// of combat damage resolution).
    fn assert_valid_stopping_point(&self) {
        // Get the buffered logs
        let logs = self.game.logger.logs();

        if logs.is_empty() {
            // No logs yet - could be at the very start of the game
            // This is acceptable (e.g., stopping before any actions)
            return;
        }

        // Check the last few log entries for valid stopping contexts
        // We check the last 5 entries to handle cases where there might be
        // multiple logged items at the same stopping point
        let check_count = logs.len().min(5);
        let recent_logs = &logs[logs.len() - check_count..];

        for log_entry in recent_logs.iter().rev() {
            let message = &log_entry.message;

            // Valid stopping points:
            // 1. Controller choice
            if message.contains(">>> ")
                && (message.contains("chose")
                    || message.contains("RANDOM")
                    || message.contains("HEURISTIC")
                    || message.contains("ZERO"))
            {
                return; // Valid: stopped after a controller choice
            }

            // 2. Game end
            if message.contains("Game Over") || message.contains("wins!") {
                return; // Valid: stopped at game end
            }
        }

        // If we get here, we didn't find a valid stopping point
        // Print the last few log entries for debugging
        eprintln!("\n⚠️  WARNING: Stopping at potentially invalid point!");
        eprintln!("Last {} log entries:", check_count);
        for (i, log_entry) in recent_logs.iter().enumerate() {
            eprintln!("  [{}] {}", logs.len() - check_count + i, log_entry.message);
        }

        // For now, we just warn - we can make this a panic later if needed
        // panic!("Stopped at invalid point - see log entries above");
    }

    /// Save a snapshot when choice limit is reached and exit
    ///
    /// This rewinds the undo log to the most recent turn boundary, extracts
    /// intra-turn choices, saves controller RNG state, and saves a GameSnapshot to disk.
    ///
    /// Returns a GameResult with `GameEndReason::Snapshot`.
    fn save_snapshot_and_exit<P: AsRef<std::path::Path>>(
        &mut self,
        choice_limit: usize,
        snapshot_path: P,
        format: crate::game::snapshot::SnapshotFormat,
        controller1: &dyn PlayerController,
        controller2: &dyn PlayerController,
    ) -> Result<GameResult> {
        // Assert that we're stopping at a valid point (after a choice or game end)
        self.assert_valid_stopping_point();

        // Rewind to the most recent turn boundary and extract intra-turn choices
        // This actually undoes game state to the turn boundary
        // We need to temporarily take ownership of undo_log to avoid borrowing conflicts
        let mut undo_log = std::mem::take(&mut self.game.undo_log);
        let rewind_result = undo_log.rewind_to_turn_start(self.game);
        self.game.undo_log = undo_log;

        let (turn_number, intra_turn_choices, actions_rewound) = if let Some(result) = rewind_result {
            result
        } else {
            // No ChangeTurn action found - we're still in turn 1!
            // Extract all ChoicePoint actions from the undo log as intra-turn choices
            let mut intra_turn_choices = Vec::new();
            for action in self.game.undo_log.actions() {
                if let crate::undo::GameAction::ChoicePoint { .. } = action {
                    intra_turn_choices.push(action.clone());
                }
            }

            if self.verbosity >= VerbosityLevel::Verbose {
                eprintln!(
                    "  (Snapshot during turn 1 - no rewind needed, {} choice points captured)",
                    intra_turn_choices.len()
                );
            }

            // Turn 1, all choices are intra-turn, no actions were rewound
            (1, intra_turn_choices, 0)
        };

        // Clone the game state at the turn boundary (or game start if turn 1)
        let game_state_snapshot = self.game.clone();

        // Capture controller types (ALWAYS needed for resume)
        let p1_controller_type = controller1.get_controller_type();
        let p2_controller_type = controller2.get_controller_type();

        // Capture controller RNG states (only for stateful controllers)
        let p1_controller_state = controller1
            .get_snapshot_state()
            .and_then(|v| serde_json::from_value(v).ok());
        let p2_controller_state = controller2
            .get_snapshot_state()
            .and_then(|v| serde_json::from_value(v).ok());

        // Create snapshot with state + choices + controller types + controller states
        let snapshot = crate::game::GameSnapshot::with_controllers(
            game_state_snapshot,
            turn_number,
            self.choice_counter, // Save total choice count for restoration
            intra_turn_choices,
            p1_controller_type,
            p2_controller_type,
            p1_controller_state,
            p2_controller_state,
        );

        // Save to file
        snapshot
            .save_to_file(&snapshot_path, format)
            .map_err(|e| MtgError::InvalidAction(format!("Failed to save snapshot: {}", e)))?;

        // Log snapshot info to stderr (meta-information, not game output)
        if self.verbosity >= VerbosityLevel::Minimal {
            eprintln!("\n=== Snapshot Saved ===");
            eprintln!("  Choice limit reached: {} choices", choice_limit);
            eprintln!("  Snapshot saved to: {}", snapshot_path.as_ref().display());
            eprintln!("  Turn number: {}", turn_number);
            eprintln!("  Intra-turn choices: {}", snapshot.choice_count());
            eprintln!("  Actions rewound: {}", actions_rewound);
        }

        // Return early with Snapshot end reason
        Ok(GameResult {
            winner: None,
            turns_played: self.turns_elapsed,
            end_reason: GameEndReason::Snapshot,
        })
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
        if self.verbosity >= VerbosityLevel::Normal && !is_resumed_turn {
            let player_name = self.get_player_name(active_player);

            // Debug: Log state hash before turn header
            let turn_msg = format!("Turn {} - {}'s turn", self.turns_elapsed + 1, player_name);
            self.game.debug_log_state_hash(&turn_msg);

            if self.should_print_to_stdout() {
                println!("\n========================================");
                println!("{}", turn_msg);
                println!("========================================");

                // Print detailed battlefield state for both players
                self.print_battlefield_state();
            }
        }

        // Suppress turn header ONLY if we're in the resumed turn (it was already printed before snapshot)
        if is_resumed_turn && self.verbosity >= VerbosityLevel::Verbose && self.should_print_to_stdout() {
            println!("🔄 RESUMING TURN {} (will suppress header)", self.turns_elapsed + 1);
        }

        // Reset turn-based state
        self.reset_turn_state(active_player)?;

        // Run through all steps of the turn
        loop {
            // Execute the step
            if let Some(result) = self.execute_step(controller1, controller2)? {
                // Mid-turn snapshot triggered (e.g., fixed controller exhausted)
                return Ok(Some(result));
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

    /// Get player name for display
    fn get_player_name(&self, player_id: PlayerId) -> String {
        self.game
            .get_player(player_id)
            .map(|p| p.name.to_string())
            .unwrap_or_else(|_| {
                // Use 1-based indexing for human-readable player numbers
                let player_num = player_id.as_u32() + 1;
                format!("Player {}", player_num)
            })
    }

    /// Get step name for display
    fn step_name(&self, step: Step) -> &'static str {
        match step {
            Step::Untap => "Untap Step",
            Step::Upkeep => "Upkeep Step",
            Step::Draw => "Draw Step",
            Step::Main1 => "Main Phase 1",
            Step::BeginCombat => "Beginning of Combat",
            Step::DeclareAttackers => "Declare Attackers Step",
            Step::DeclareBlockers => "Declare Blockers Step",
            Step::CombatDamage => "Combat Damage Step",
            Step::EndCombat => "End of Combat Step",
            Step::Main2 => "Main Phase 2",
            Step::End => "End Step",
            Step::Cleanup => "Cleanup Step",
        }
    }

    /// Print detailed battlefield state for both players
    fn print_battlefield_state(&self) {
        if !self.should_print_to_stdout() {
            return;
        }

        // Print state for each player
        for player in self.game.players.iter() {
            let is_active = player.id == self.game.turn.active_player;
            let marker = if is_active { " (active)" } else { "" };

            println!("{}{}: ", player.name, marker);
            println!("  Life: {}", player.life);

            // Zone sizes
            if let Some(zones) = self.game.get_player_zones(player.id) {
                println!(
                    "  Hand: {} | Library: {} | Graveyard: {} | Exile: {}",
                    zones.hand.len(),
                    zones.library.len(),
                    zones.graveyard.len(),
                    zones.exile.len()
                );

                // Show hand contents for active player (whose turn it is)
                if is_active && !zones.hand.is_empty() {
                    println!("  Hand contents:");
                    for &card_id in &zones.hand.cards {
                        if let Ok(card) = self.game.cards.get(card_id) {
                            println!("    - {} ({})", card.name, card.mana_cost);
                        }
                    }
                }
            }

            // Battlefield permanents controlled by this player
            let mut player_permanents: Vec<_> = self
                .game
                .battlefield
                .cards
                .iter()
                .filter_map(|&card_id| {
                    self.game.cards.get(card_id).ok().and_then(|card| {
                        if card.controller == player.id {
                            Some((card_id, card))
                        } else {
                            None
                        }
                    })
                })
                .collect();

            // Sort by card type for better readability: lands first, then creatures, then others
            player_permanents.sort_by_key(|(_, card)| {
                if card.is_land() {
                    0
                } else if card.is_creature() {
                    1
                } else {
                    2
                }
            });

            if player_permanents.is_empty() {
                println!("  Battlefield: (empty)");
            } else {
                println!("  Battlefield:");
                for (card_id, card) in player_permanents {
                    let tap_status = if card.tapped { " (tapped)" } else { "" };

                    // Check for summoning sickness (creatures that entered this turn and don't have haste)
                    let has_summoning_sickness = if card.is_creature() {
                        if let Some(entered_turn) = card.turn_entered_battlefield {
                            entered_turn == self.game.turn.turn_number
                                && !card.has_keyword(&crate::core::Keyword::Haste)
                        } else {
                            false
                        }
                    } else {
                        false
                    };
                    let sickness_status = if has_summoning_sickness {
                        " (summoning sickness)"
                    } else {
                        ""
                    };

                    // Format card display based on type
                    if card.is_creature() {
                        let power = card.power.unwrap_or(0) + card.power_bonus as i8;
                        let toughness = card.toughness.unwrap_or(0) + card.toughness_bonus as i8;
                        println!(
                            "    {} ({}) - {}/{}{}{}",
                            card.name, card_id, power, toughness, tap_status, sickness_status
                        );
                    } else {
                        println!("    {} ({}){}", card.name, card_id, tap_status);
                    }
                }
            }
        }
        println!();
    }

    /// Print step header lazily (only when first action happens in this step)
    /// Used for Normal verbosity level
    fn print_step_header_if_needed(&mut self) {
        if self.verbosity == VerbosityLevel::Normal
            && !self.step_header_printed
            && !self.replaying
            && self.should_print_to_stdout()
        {
            let step = self.game.turn.current_step;
            println!("--- {} ---", self.step_name(step));
            self.step_header_printed = true;
        }
    }

    // === Logging Helpers ===
    // These methods encapsulate lazy header printing + message output

    /// Check if stdout printing is allowed by the logger's output mode
    /// Returns true if OutputMode is Stdout or Both, false if Memory-only
    fn should_print_to_stdout(&self) -> bool {
        use crate::game::logger::OutputMode;
        matches!(self.game.logger.output_mode(), OutputMode::Stdout | OutputMode::Both)
    }

    /// Log a message at Normal verbosity level (with lazy step header)
    /// Most game events use this level
    fn log_normal(&mut self, message: &str) {
        if self.verbosity >= VerbosityLevel::Normal && !self.replaying && self.should_print_to_stdout() {
            self.print_step_header_if_needed();
            println!("  {message}");
        }
    }

    /// Log a message at Verbose verbosity level (with lazy step header)
    /// Used for detailed action-by-action logging
    #[allow(dead_code)] // Legacy v1 interface, will be removed
    fn log_verbose(&mut self, message: &str) {
        if self.verbosity >= VerbosityLevel::Verbose && !self.replaying && self.should_print_to_stdout() {
            self.print_step_header_if_needed();
            println!("  {message}");
        }
    }

    /// Log a message at Minimal verbosity level (no step header needed)
    /// Used for major game events like outcomes
    #[allow(dead_code)]
    fn log_minimal(&mut self, message: &str) {
        if self.verbosity >= VerbosityLevel::Minimal && self.should_print_to_stdout() {
            println!("{message}");
        }
    }

    /// Log the execution of a spell effect (damage, draw, etc.)
    fn log_effect_execution(
        &self,
        source_name: &str,
        source_id: CardId,
        effect: &crate::core::Effect,
        _source_owner: PlayerId,
    ) {
        use crate::core::{Effect, TargetRef};

        if !self.should_print_to_stdout() {
            return;
        }

        match effect {
            Effect::DealDamage { target, amount } => match target {
                TargetRef::Player(target_player_id) => {
                    let target_name = self.get_player_name(*target_player_id);
                    let new_life = self.game.get_player(*target_player_id).map(|p| p.life).unwrap_or(0);
                    let message = format!(
                        "{source_name} ({source_id}) deals {amount} damage to {target_name} - life: {new_life}"
                    );
                    self.game.logger.normal(&message);
                }
                TargetRef::Permanent(target_card_id) => {
                    let target_name = self
                        .game
                        .cards
                        .get(*target_card_id)
                        .map(|c| c.name.as_str())
                        .unwrap_or("Unknown");
                    let message = format!(
                        "{source_name} ({source_id}) deals {amount} damage to {target_name} ({target_card_id})"
                    );
                    self.game.logger.normal(&message);
                }
                TargetRef::None => {
                    // Target will be filled in by resolve_spell - log against opponent
                    if let Some(opponent_id) = self.game.players.iter().map(|p| p.id).find(|id| *id != _source_owner) {
                        let target_name = self.get_player_name(opponent_id);
                        let new_life = self.game.get_player(opponent_id).map(|p| p.life).unwrap_or(0);
                        let message = format!(
                            "{source_name} ({source_id}) deals {amount} damage to {target_name} - life: {new_life}"
                        );
                        self.game.logger.normal(&message);
                    }
                }
            },
            Effect::DrawCards { player, count } => {
                let player_name = self.get_player_name(*player);
                let message = format!("{source_name} ({source_id}) causes {player_name} to draw {count} card(s)");
                self.game.logger.normal(&message);
            }
            Effect::GainLife { player, amount } => {
                let player_name = self.get_player_name(*player);
                let new_life = self.game.get_player(*player).map(|p| p.life).unwrap_or(0);
                let message = format!(
                    "{source_name} ({source_id}) causes {player_name} to gain {amount} life - life: {new_life}"
                );
                self.game.logger.normal(&message);
            }
            Effect::DestroyPermanent { target } => {
                let target_name = self
                    .game
                    .cards
                    .get(*target)
                    .map(|c| c.name.as_str())
                    .unwrap_or("Unknown");
                let message = format!("{source_name} ({source_id}) destroys {target_name} ({target})");
                self.game.logger.normal(&message);
            }
            Effect::TapPermanent { target } => {
                let target_name = self
                    .game
                    .cards
                    .get(*target)
                    .map(|c| c.name.as_str())
                    .unwrap_or("Unknown");
                let message = format!("{source_name} ({source_id}) taps {target_name} ({target})");
                self.game.logger.normal(&message);
            }
            Effect::UntapPermanent { target } => {
                let target_name = self
                    .game
                    .cards
                    .get(*target)
                    .map(|c| c.name.as_str())
                    .unwrap_or("Unknown");
                let message = format!("{source_name} ({source_id}) untaps {target_name} ({target})");
                self.game.logger.normal(&message);
            }
            Effect::PumpCreature {
                target,
                power_bonus,
                toughness_bonus,
            } => {
                let target_name = self
                    .game
                    .cards
                    .get(*target)
                    .map(|c| c.name.as_str())
                    .unwrap_or("Unknown");
                let message = format!(
                    "{source_name} ({source_id}) gives {target_name} ({target}) {power_bonus:+}/{toughness_bonus:+} until end of turn"
                );
                self.game.logger.normal(&message);
            }
            Effect::Mill { player, count } => {
                let player_name = self.get_player_name(*player);
                let message = format!("{source_name} ({source_id}) causes {player_name} to mill {count} card(s)");
                self.game.logger.normal(&message);
            }
            Effect::CounterSpell { target } => {
                let target_name = self
                    .game
                    .cards
                    .get(*target)
                    .map(|c| c.name.as_str())
                    .unwrap_or("Unknown");
                let message = format!("{source_name} ({source_id}) counters {target_name} ({target})");
                self.game.logger.normal(&message);
            }
            Effect::AddMana { player, mana } => {
                let player_name = self.get_player_name(*player);
                let message = format!("{source_name} ({source_id}) adds {mana} to {player_name}'s mana pool");
                self.game.logger.normal(&message);
            }
            Effect::PutCounter {
                target,
                counter_type,
                amount,
            } => {
                let target_name = self
                    .game
                    .cards
                    .get(*target)
                    .map(|c| c.name.as_str())
                    .unwrap_or("Unknown");
                println!(
                    "  {source_name} ({source_id}) puts {amount} {counter_type:?} counter(s) on {target_name} ({target})"
                );
            }
            Effect::RemoveCounter {
                target,
                counter_type,
                amount,
            } => {
                let target_name = self
                    .game
                    .cards
                    .get(*target)
                    .map(|c| c.name.as_str())
                    .unwrap_or("Unknown");
                println!(
                    "  {source_name} ({source_id}) removes {amount} {counter_type:?} counter(s) from {target_name} ({target})"
                );
            }
            Effect::ExilePermanent { target } => {
                let target_name = self
                    .game
                    .cards
                    .get(*target)
                    .map(|c| c.name.as_str())
                    .unwrap_or("Unknown");
                println!("  {source_name} ({source_id}) exiles {target_name} ({target})");
            }
            Effect::SearchLibrary {
                player,
                card_type_filter,
                destination,
                enters_tapped,
                shuffle: _,
            } => {
                let player_name = self.get_player_name(*player);
                let tapped_text = if *enters_tapped { " tapped" } else { "" };
                println!(
                    "  {source_name} ({source_id}) searches {player_name}'s library for a {card_type_filter} card and puts it into {:?}{tapped_text}",
                    destination
                );
            }
        }
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

        Ok(())
    }

    /// Execute a single step
    pub fn execute_step(
        &mut self,
        controller1: &mut dyn PlayerController,
        controller2: &mut dyn PlayerController,
    ) -> Result<Option<GameResult>> {
        let step = self.game.turn.current_step;

        // Reset step header tracking for each new step
        self.step_header_printed = false;

        // In verbose mode, always print step header immediately
        if self.verbosity >= VerbosityLevel::Verbose && !self.replaying {
            println!("--- {} ---", self.step_name(step));
        }

        match step {
            Step::Untap => {
                self.untap_step()?;
                Ok(None)
            }
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

    /// Untap step - untap all permanents controlled by active player
    fn untap_step(&mut self) -> Result<()> {
        let active_player = self.game.turn.active_player;

        // Untap all permanents controlled by active player
        // Use SmallVec to avoid heap allocation for typical small counts of tapped cards
        let cards_to_untap: SmallVec<[CardId; 8]> = self
            .game
            .battlefield
            .cards
            .iter()
            .copied()
            .filter(|&card_id| {
                self.game
                    .cards
                    .get(card_id)
                    .map(|c| c.owner == active_player && c.tapped)
                    .unwrap_or(false)
            })
            .collect();

        for card_id in cards_to_untap {
            if let Ok(card) = self.game.cards.get_mut(card_id) {
                card.untap();
            }
        }

        Ok(())
    }

    /// Check and execute phase-triggered abilities
    fn check_phase_triggers(&mut self, trigger_event: crate::core::TriggerEvent) -> Result<()> {
        // Collect all permanents with triggers matching this event
        // Also collect trigger descriptions for logging
        let triggered_info: SmallVec<[(CardId, Vec<String>); 4]> = self
            .game
            .battlefield
            .cards
            .iter()
            .filter_map(|&card_id| {
                if let Ok(card) = self.game.cards.get(card_id) {
                    let matching_descriptions: Vec<String> = card
                        .triggers
                        .iter()
                        .filter(|t| t.event == trigger_event)
                        .map(|t| format!("Trigger: {} - {}", card.name, t.description))
                        .collect();

                    if !matching_descriptions.is_empty() {
                        Some((card_id, matching_descriptions))
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect();

        // For each card with a matching trigger, log and execute
        // Note: In the future, this will need to handle optional triggers, conditions, etc.
        for (card_id, descriptions) in triggered_info {
            // Log trigger activation if verbose
            if self.verbosity >= VerbosityLevel::Verbose {
                for desc in descriptions {
                    self.log_verbose(&desc);
                }
            }

            // Use the existing check_triggers method to execute effects
            // Pass the card_id as the source for filling in placeholders
            self.game.check_triggers(trigger_event, card_id)?;
        }

        Ok(())
    }

    /// Upkeep step - priority round for triggers and actions
    fn upkeep_step(
        &mut self,
        controller1: &mut dyn PlayerController,
        controller2: &mut dyn PlayerController,
    ) -> Result<Option<GameResult>> {
        // Check for beginning of upkeep triggers
        self.check_phase_triggers(crate::core::TriggerEvent::BeginningOfUpkeep)?;

        // Pass priority
        if let Some(result) = self.priority_round(controller1, controller2)? {
            return Ok(Some(result));
        }
        Ok(None)
    }

    /// Draw step - active player draws a card
    fn draw_step(
        &mut self,
        controller1: &mut dyn PlayerController,
        controller2: &mut dyn PlayerController,
    ) -> Result<Option<GameResult>> {
        let active_player = self.game.turn.active_player;

        // Skip draw on first turn (player going first doesn't draw)
        if self.game.turn.turn_number == 1 {
            self.log_normal("(First turn - no draw)");
            return Ok(None);
        }

        // Debug: Log state hash before draw
        #[cfg(feature = "verbose-logging")]
        {
            let player_name = self.get_player_name(active_player);
            let draw_msg = format!("{} draws", player_name);
            self.game.debug_log_state_hash(&draw_msg);
        }

        // Draw a card
        self.game.draw_card(active_player)?;

        #[cfg(feature = "verbose-logging")]
        {
            // Skip draw logging during replay mode (already logged in previous game segment)
            if !self.replaying {
                let player_name = self.get_player_name(active_player);
                if let Some(zones) = self.game.get_player_zones(active_player) {
                    if let Some(&card_id) = zones.hand.cards.last() {
                        if let Ok(card) = self.game.cards.get(card_id) {
                            log_if_verbose!(self, "{} draws {} ({})", player_name, card.name, card_id);
                        } else {
                            log_if_verbose!(self, "{} draws a card", player_name);
                        }
                    } else {
                        log_if_verbose!(self, "{} draws a card", player_name);
                    }
                } else {
                    log_if_verbose!(self, "{} draws a card", player_name);
                }
            }
        }

        // MTG Rules 504.2: After draw, players receive priority
        if let Some(result) = self.priority_round(controller1, controller2)? {
            return Ok(Some(result));
        }

        Ok(None)
    }

    /// Main phase - players can play spells and lands
    fn main_phase(
        &mut self,
        controller1: &mut dyn PlayerController,
        controller2: &mut dyn PlayerController,
    ) -> Result<Option<GameResult>> {
        // Priority round where players can take actions
        if let Some(result) = self.priority_round(controller1, controller2)? {
            return Ok(Some(result));
        }
        Ok(None)
    }

    /// Combat phases (simplified for now)
    fn begin_combat_step(
        &mut self,
        controller1: &mut dyn PlayerController,
        controller2: &mut dyn PlayerController,
    ) -> Result<Option<GameResult>> {
        if let Some(result) = self.priority_round(controller1, controller2)? {
            return Ok(Some(result));
        }
        Ok(None)
    }

    fn declare_attackers_step(
        &mut self,
        controller1: &mut dyn PlayerController,
        controller2: &mut dyn PlayerController,
    ) -> Result<Option<GameResult>> {
        // Active player declares attackers
        let active_player = self.game.turn.active_player;
        let controller: &mut dyn PlayerController = if active_player == controller1.player_id() {
            controller1
        } else {
            controller2
        };

        // Get available creatures that can attack
        let available_creatures = self.get_available_attacker_creatures(active_player);

        if !available_creatures.is_empty() {
            // Clear replay mode if all choices have been replayed
            // This happens BEFORE checking stop conditions, so a snapshot taken here will NOT
            // include the upcoming choice (which hasn't been presented yet)
            //
            // We stay in replay mode until BOTH conditions are met:
            // 1. All intra-turn choices have been replayed (replay_choices_remaining == 0)
            // 2. We've passed the baseline choice count from the snapshot
            //
            // This ensures that automatic actions (like draws) that happen before the first
            // NEW choice point are properly suppressed, avoiding duplicate logging.
            if self.replaying
                && self.replay_choices_remaining == 0
                && (self.choice_counter as usize) >= self.baseline_choice_count
            {
                eprintln!(
                    "🔍 [REPLAY_CLEAR_ATTACKERS] choice_counter={}, baseline={}, CLEARING replay mode",
                    self.choice_counter, self.baseline_choice_count
                );
                self.replaying = false;
                if self.verbosity >= VerbosityLevel::Verbose {
                    println!("✅ REPLAY MODE COMPLETE - will present attacker choice to controller");
                }
            } else if self.replaying {
                eprintln!(
                    "🔍 [REPLAY_STILL_ACTIVE_ATTACKERS] choice_counter={}, baseline={}, remaining={}",
                    self.choice_counter, self.baseline_choice_count, self.replay_choices_remaining
                );
            }

            // Create view and print prompt BEFORE checking stop conditions
            // so users see what choice was about to be made when using --stop-when-fixed-exhausted
            {
                let view = GameStateView::new(self.game, active_player);
                // Print attacker selection prompt (controlled by show_choice_menu flag)
                if view.logger().should_show_choice_menu() && !available_creatures.is_empty() {
                    print!("{}", format_attackers_prompt(&view, &available_creatures));
                }
            } // Drop view before mutable borrow

            // PREAMBLE: Check stop conditions before asking for choice
            if let Some(result) = self.check_stop_conditions(controller, active_player)? {
                return Ok(Some(result));
            }

            // Ask controller to choose all attackers at once (v2 interface)
            let view = GameStateView::new(self.game, active_player);
            let attackers = controller.choose_attackers(&view, &available_creatures);

            // Log this choice point for snapshot/replay
            let replay_choice = crate::game::ReplayChoice::Attackers(attackers.clone());
            self.log_choice_point(active_player, Some(replay_choice));

            // Declare each chosen attacker
            for attacker_id in attackers.iter() {
                // Use GameState::declare_attacker() which taps the creature (MTG Rules 508.1f)
                // NOT Combat::declare_attacker() which only adds to the attackers list
                if let Err(e) = self.game.declare_attacker(active_player, *attacker_id) {
                    if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                        eprintln!("  Error declaring attacker: {e}");
                    }
                    continue;
                }

                if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                    let card_name = self
                        .game
                        .cards
                        .get(*attacker_id)
                        .map(|c| c.name.as_str())
                        .unwrap_or("Unknown");

                    // Get power/toughness for more detail
                    if let Ok(card) = self.game.cards.get(*attacker_id) {
                        let power = card.power.unwrap_or(0);
                        let toughness = card.toughness.unwrap_or(0);
                        let message = format!(
                            "{} declares {} ({}) ({}/{}) as attacker",
                            self.get_player_name(active_player),
                            card_name,
                            attacker_id,
                            power,
                            toughness
                        );
                        self.game.logger.normal(&message);
                    }
                }
            }
        }

        // MTG Rules 508.4: After attackers are declared, players receive priority
        if let Some(result) = self.priority_round(controller1, controller2)? {
            return Ok(Some(result));
        }

        Ok(None)
    }

    fn declare_blockers_step(
        &mut self,
        controller1: &mut dyn PlayerController,
        controller2: &mut dyn PlayerController,
    ) -> Result<Option<GameResult>> {
        // Defending player declares blockers
        let active_player = self.game.turn.active_player;
        let defending_player = self
            .game
            .get_other_player_id(active_player)
            .expect("Should have defending player");

        let controller: &mut dyn PlayerController = if defending_player == controller1.player_id() {
            controller1
        } else {
            controller2
        };

        // Get available blockers and attackers
        let available_blockers = self.get_available_blocker_creatures(defending_player);
        let attackers = self.get_current_attackers();

        if !available_blockers.is_empty() && !attackers.is_empty() {
            // Clear replay mode if all choices have been replayed
            // This happens BEFORE checking stop conditions, so a snapshot taken here will NOT
            // include the upcoming choice (which hasn't been presented yet)
            //
            // We stay in replay mode until BOTH conditions are met:
            // 1. All intra-turn choices have been replayed (replay_choices_remaining == 0)
            // 2. We've passed the baseline choice count from the snapshot
            //
            // This ensures that automatic actions (like draws) that happen before the first
            // NEW choice point are properly suppressed, avoiding duplicate logging.
            if self.replaying
                && self.replay_choices_remaining == 0
                && (self.choice_counter as usize) >= self.baseline_choice_count
            {
                eprintln!(
                    "🔍 [REPLAY_CLEAR_BLOCKERS] choice_counter={}, baseline={}, CLEARING replay mode",
                    self.choice_counter, self.baseline_choice_count
                );
                self.replaying = false;
                if self.verbosity >= VerbosityLevel::Verbose {
                    println!("✅ REPLAY MODE COMPLETE - will present blocker choice to controller");
                }
            } else if self.replaying {
                eprintln!(
                    "🔍 [REPLAY_STILL_ACTIVE_BLOCKERS] choice_counter={}, baseline={}, remaining={}",
                    self.choice_counter, self.baseline_choice_count, self.replay_choices_remaining
                );
            }

            // Create view and print prompt BEFORE checking stop conditions
            // so users see what choice was about to be made when using --stop-when-fixed-exhausted
            {
                let view = GameStateView::new(self.game, defending_player);
                // Print blocker selection prompt (controlled by show_choice_menu flag)
                if view.logger().should_show_choice_menu() {
                    print!("{}", format_blockers_prompt(&view, &available_blockers, &attackers));
                }
            } // Drop view before mutable borrow

            // PREAMBLE: Check stop conditions before asking for choice
            if let Some(result) = self.check_stop_conditions(controller, defending_player)? {
                return Ok(Some(result));
            }

            // Ask controller to choose all blocker assignments at once (v2 interface)
            let view = GameStateView::new(self.game, defending_player);
            let blocks = controller.choose_blockers(&view, &available_blockers, &attackers);

            // Log this choice point for snapshot/replay
            let replay_choice = crate::game::ReplayChoice::Blockers(blocks.clone());
            self.log_choice_point(defending_player, Some(replay_choice));

            // Declare each blocking assignment
            for (blocker_id, attacker_id) in blocks.iter() {
                let mut attackers_vec = SmallVec::new();
                attackers_vec.push(*attacker_id);
                self.game.combat.declare_blocker(*blocker_id, attackers_vec);

                if self.verbosity >= VerbosityLevel::Verbose && !self.replaying {
                    let blocker_name = self
                        .game
                        .cards
                        .get(*blocker_id)
                        .map(|c| c.name.as_str())
                        .unwrap_or("Unknown");
                    let attacker_name = self
                        .game
                        .cards
                        .get(*attacker_id)
                        .map(|c| c.name.as_str())
                        .unwrap_or("Unknown");
                    let message = format!(
                        "{} blocks {} with {}",
                        self.get_player_name(defending_player),
                        attacker_name,
                        blocker_name
                    );
                    self.game.logger.normal(&message);
                }
            }
        }

        // MTG Rules 509.4: After blockers are declared, players receive priority
        if let Some(result) = self.priority_round(controller1, controller2)? {
            return Ok(Some(result));
        }

        Ok(None)
    }

    fn combat_damage_step(
        &mut self,
        controller1: &mut dyn PlayerController,
        controller2: &mut dyn PlayerController,
    ) -> Result<Option<GameResult>> {
        // Check if any attacking or blocking creature has first strike or double strike
        // MTG Rules 510.4: If so, we have two combat damage steps
        let has_first_strike = self.has_first_strike_combat();

        if has_first_strike {
            // First strike damage step
            if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                self.game.logger.normal("--- First Strike Combat Damage ---");
            }
            self.log_combat_damage(true)?;
            self.game.assign_combat_damage(controller1, controller2, true)?;
            if let Some(result) = self.priority_round(controller1, controller2)? {
                return Ok(Some(result));
            }
        }

        // Normal combat damage step (or only step if no first strike)
        if self.verbosity >= VerbosityLevel::Normal && has_first_strike && !self.replaying {
            self.game.logger.normal("--- Normal Combat Damage ---");
        }
        self.log_combat_damage(false)?;
        self.game.assign_combat_damage(controller1, controller2, false)?;

        // After damage is dealt, players get priority
        if let Some(result) = self.priority_round(controller1, controller2)? {
            return Ok(Some(result));
        }
        Ok(None)
    }

    /// Check if any attacking or blocking creature has first strike or double strike
    fn has_first_strike_combat(&self) -> bool {
        // Check all attackers (using iterator to avoid Vec allocation)
        for attacker_id in self.game.combat.attackers_iter() {
            if let Ok(attacker) = self.game.cards.get(attacker_id) {
                if attacker.has_first_strike() || attacker.has_double_strike() {
                    return true;
                }
            }

            // Check all blockers of this attacker
            if self.game.combat.is_blocked(attacker_id) {
                let blockers = self.game.combat.get_blockers(attacker_id);
                for blocker_id in &blockers {
                    if let Ok(blocker) = self.game.cards.get(*blocker_id) {
                        if blocker.has_first_strike() || blocker.has_double_strike() {
                            return true;
                        }
                    }
                }
            }
        }

        false
    }

    /// Log combat damage for debugging
    fn log_combat_damage(&self, first_strike_step: bool) -> Result<()> {
        if self.verbosity < VerbosityLevel::Normal || self.replaying {
            return Ok(());
        }

        let mut attackers = self.game.combat.get_attackers();
        // Sort for deterministic logging output
        attackers.sort_by_key(|id| id.as_u32());

        for attacker_id in &attackers {
            // Skip creatures that are no longer on the battlefield
            // MTG Rule 510.1c: Only creatures still on the battlefield deal combat damage
            if !self.game.battlefield.contains(*attacker_id) {
                continue;
            }

            if let Ok(attacker) = self.game.cards.get(*attacker_id) {
                // Check if this attacker deals damage in this step
                let deals_damage = if first_strike_step {
                    attacker.has_first_strike() || attacker.has_double_strike()
                } else {
                    attacker.has_normal_strike()
                };

                if !deals_damage {
                    continue;
                }

                let power = attacker.current_power();
                let attacker_name = &attacker.name;

                if self.game.combat.is_blocked(*attacker_id) {
                    let mut blockers = self.game.combat.get_blockers(*attacker_id);
                    // Sort for deterministic logging output
                    blockers.sort_by_key(|id| id.as_u32());
                    for blocker_id in &blockers {
                        if let Ok(blocker) = self.game.cards.get(*blocker_id) {
                            // Check if blocker deals damage in this step
                            let blocker_deals_damage = if first_strike_step {
                                blocker.has_first_strike() || blocker.has_double_strike()
                            } else {
                                blocker.has_normal_strike()
                            };

                            if !blocker_deals_damage {
                                continue;
                            }

                            let blocker_power = blocker.current_power();
                            let blocker_name = &blocker.name;
                            let message = format!(
                                "Combat: {attacker_name} ({attacker_id}) ({power} damage) ↔ {blocker_name} ({blocker_id}) ({blocker_power} damage)"
                            );
                            self.game.logger.normal(&message);
                        }
                    }
                } else {
                    // Unblocked attacker
                    if let Some(defending_player) = self.game.combat.get_defending_player(*attacker_id) {
                        let defender_name = self.get_player_name(defending_player);
                        if power > 0 {
                            let message =
                                format!("{attacker_name} ({attacker_id}) deals {power} damage to {defender_name}");
                            self.game.logger.normal(&message);
                        }
                    }
                }
            }
        }

        Ok(())
    }

    fn end_combat_step(
        &mut self,
        controller1: &mut dyn PlayerController,
        controller2: &mut dyn PlayerController,
    ) -> Result<Option<GameResult>> {
        // Clear combat state at end of combat
        self.game.combat.clear();

        // Players get priority
        if let Some(result) = self.priority_round(controller1, controller2)? {
            return Ok(Some(result));
        }
        Ok(None)
    }

    fn end_step(
        &mut self,
        controller1: &mut dyn PlayerController,
        controller2: &mut dyn PlayerController,
    ) -> Result<Option<GameResult>> {
        // Check for beginning of end step triggers
        self.check_phase_triggers(crate::core::TriggerEvent::BeginningOfEndStep)?;

        if let Some(result) = self.priority_round(controller1, controller2)? {
            return Ok(Some(result));
        }
        Ok(None)
    }

    /// Cleanup step - discard to hand size, remove damage
    fn cleanup_step(
        &mut self,
        controller1: &mut dyn PlayerController,
        controller2: &mut dyn PlayerController,
    ) -> Result<Option<GameResult>> {
        let active_player = self.game.turn.active_player;

        // Get non-active player
        let non_active_player = self
            .game
            .get_other_player_id(active_player)
            .expect("Should have non-active player");

        // Process active player first, then non-active player
        for &player_id in &[active_player, non_active_player] {
            let hand_size = self
                .game
                .get_player_zones(player_id)
                .map(|z| z.hand.cards.len())
                .unwrap_or(0);

            let max_hand_size = self.game.get_player(player_id)?.max_hand_size;

            if hand_size > max_hand_size {
                let discard_count = hand_size - max_hand_size;

                log_if_verbose!(
                    self,
                    "{} must discard {} cards (hand size: {}, max: {})",
                    self.get_player_name(player_id),
                    discard_count,
                    hand_size,
                    max_hand_size
                );

                // Get the appropriate controller
                let controller: &mut dyn PlayerController = if player_id == controller1.player_id() {
                    controller1
                } else {
                    controller2
                };

                // Create view and print prompt BEFORE checking stop conditions
                // so users see what choice was about to be made when using --stop-when-fixed-exhausted
                {
                    let view = GameStateView::new(self.game, player_id);
                    let hand = view.hand();
                    // Print discard selection prompt (controlled by show_choice_menu flag)
                    if view.logger().should_show_choice_menu() {
                        print!("{}", format_discard_prompt(&view, hand, discard_count));
                    }
                } // Drop view before mutable borrow

                // PREAMBLE: Check stop conditions before asking for choice
                if let Some(result) = self.check_stop_conditions(controller, player_id)? {
                    return Ok(Some(result));
                }

                // Ask controller which cards to discard
                let view = GameStateView::new(self.game, player_id);
                let hand = view.hand();
                let cards_to_discard = controller.choose_cards_to_discard(&view, hand, discard_count);

                // Log this choice point for snapshot/replay
                let replay_choice = crate::game::ReplayChoice::Discard(cards_to_discard.clone());
                self.log_choice_point(player_id, Some(replay_choice));

                // Verify correct number of cards
                if cards_to_discard.len() != discard_count {
                    return Err(crate::MtgError::InvalidAction(format!(
                        "Must discard exactly {discard_count} cards, got {}",
                        cards_to_discard.len()
                    )));
                }

                // Move cards to graveyard
                for card_id in cards_to_discard {
                    // Verify card is in hand before moving
                    if let Some(zones) = self.game.get_player_zones(player_id) {
                        if !zones.hand.contains(card_id) {
                            return Err(crate::MtgError::InvalidAction(format!(
                                "Card {card_id:?} not in player's hand"
                            )));
                        }
                    }

                    // Use move_card to properly log the action for undo
                    self.game.move_card(
                        card_id,
                        crate::zones::Zone::Hand,
                        crate::zones::Zone::Graveyard,
                        player_id,
                    )?;

                    log_if_verbose!(
                        self,
                        "{} discards {} ({})",
                        self.get_player_name(player_id),
                        self.game
                            .cards
                            .get(card_id)
                            .map(|c| c.name.as_str())
                            .unwrap_or("Unknown"),
                        card_id
                    );
                }
            }
        }

        // Empty mana pools
        for &player_id in &[active_player, non_active_player] {
            if let Ok(player) = self.game.get_player_mut(player_id) {
                player.mana_pool.clear();
            }
        }

        // TODO: Remove damage from creatures

        Ok(None)
    }

    /// Resolve the top spell from the stack
    ///
    /// This removes the spell from the stack and executes its effects.
    /// Implements MTG Comprehensive Rules 608 (Resolving Spells and Abilities).
    fn resolve_top_spell_from_stack(&mut self, spell_id: CardId) -> Result<()> {
        // Look up the targets for this spell
        let targets = self
            .spell_targets
            .iter()
            .find(|(id, _)| *id == spell_id)
            .map(|(_, t)| t.clone())
            .unwrap_or_else(Vec::new);

        // Get card name and effects for logging (before resolution)
        let (card_name, card_effects, card_owner) = if let Ok(card) = self.game.cards.get(spell_id) {
            (card.name.to_string(), card.effects.clone(), card.owner)
        } else {
            return Err(crate::MtgError::EntityNotFound(spell_id.as_u32()));
        };

        if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
            let message = format!("{} ({}) resolves", card_name, spell_id);
            self.game.logger.normal(&message);
        }

        // Resolve the spell (this modifies effects with target replacement)
        self.game.resolve_spell(spell_id, &targets)?;

        // Log effects for instants/sorceries
        // Note: We need to manually replace placeholder targets for logging
        if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
            use crate::core::Effect;
            let mut target_index = 0;
            for effect in &card_effects {
                // Replace placeholder targets with chosen targets for logging
                let effect_to_log = match effect {
                    Effect::CounterSpell { target } if target.as_u32() == 0 && target_index < targets.len() => {
                        let replaced = Effect::CounterSpell {
                            target: targets[target_index],
                        };
                        target_index += 1;
                        replaced
                    }
                    Effect::DestroyPermanent { target } if target.as_u32() == 0 && target_index < targets.len() => {
                        let replaced = Effect::DestroyPermanent {
                            target: targets[target_index],
                        };
                        target_index += 1;
                        replaced
                    }
                    Effect::TapPermanent { target } if target.as_u32() == 0 && target_index < targets.len() => {
                        let replaced = Effect::TapPermanent {
                            target: targets[target_index],
                        };
                        target_index += 1;
                        replaced
                    }
                    Effect::UntapPermanent { target } if target.as_u32() == 0 && target_index < targets.len() => {
                        let replaced = Effect::UntapPermanent {
                            target: targets[target_index],
                        };
                        target_index += 1;
                        replaced
                    }
                    Effect::PumpCreature {
                        target,
                        power_bonus,
                        toughness_bonus,
                    } if target.as_u32() == 0 && target_index < targets.len() => {
                        let replaced = Effect::PumpCreature {
                            target: targets[target_index],
                            power_bonus: *power_bonus,
                            toughness_bonus: *toughness_bonus,
                        };
                        target_index += 1;
                        replaced
                    }
                    _ => effect.clone(),
                };

                self.log_effect_execution(&card_name, spell_id, &effect_to_log, card_owner);
            }

            // Check if it's a permanent entering battlefield
            if let Ok(card) = self.game.cards.get(spell_id) {
                if card.is_creature() {
                    let message = format!(
                        "{} ({}) enters the battlefield as a {}/{} creature",
                        card_name,
                        spell_id,
                        card.power.unwrap_or(0),
                        card.toughness.unwrap_or(0)
                    );
                    self.game.logger.normal(&message);
                }
            }
        }

        // Remove the spell from our targets tracking
        self.spell_targets.retain(|(id, _)| *id != spell_id);

        Ok(())
    }

    /// Priority round - players get chances to act until both pass
    ///
    /// This implements the priority system where players alternate making choices
    /// until both pass in succession, then resolves spells from the stack.
    ///
    /// ## MTG Rules Implementation
    /// - Gets all available spell abilities (lands, spells, abilities)
    /// - Calls controller.choose_spell_ability_to_play() for each priority window
    /// - Handles the chosen ability appropriately:
    ///   - PlayLand: Resolves directly (no stack)
    ///   - CastSpell: Puts spell on stack (MTG Rules 601)
    ///   - ActivateAbility: TODO - should go on stack for non-mana abilities
    /// - When both players pass with spells on stack, resolves top spell (MTG Rules 117.4)
    /// - Repeats until stack is empty and both players pass
    fn priority_round(
        &mut self,
        controller1: &mut dyn PlayerController,
        controller2: &mut dyn PlayerController,
    ) -> Result<Option<GameResult>> {
        let active_player = self.game.turn.active_player;
        let non_active_player = self
            .game
            .get_other_player_id(active_player)
            .expect("Should have non-active player");

        // Outer loop: resolve stack until empty
        loop {
            // Active player gets priority first in each round
            let mut current_priority = active_player;
            let mut consecutive_passes = 0;
            let mut action_count = 0;
            const MAX_ACTIONS_PER_PRIORITY: usize = 1000;

            // Inner loop: pass priority until both players pass
            while consecutive_passes < 2 {
                // Safety check to prevent infinite loops
                action_count += 1;
                if action_count > MAX_ACTIONS_PER_PRIORITY {
                    return Err(crate::MtgError::InvalidAction(format!(
                        "Priority round exceeded max actions ({MAX_ACTIONS_PER_PRIORITY}), possible infinite loop"
                    )));
                }

                // Get the appropriate controller
                let controller: &mut dyn PlayerController = if current_priority == controller1.player_id() {
                    controller1
                } else {
                    controller2
                };

                // Get all available spell abilities for this player
                let available = self.get_available_spell_abilities(current_priority);

                // If no actions available, automatically pass priority without asking controller
                // Only invoke controller when there's an actual choice to make
                let choice = if available.is_empty() {
                    // No available actions - automatically pass priority
                    None
                } else {
                    // Clear replay mode if all choices have been replayed
                    // This happens BEFORE checking stop conditions, so a snapshot taken here will NOT
                    // include the upcoming choice (which hasn't been presented yet)
                    //
                    // We stay in replay mode until BOTH conditions are met:
                    // 1. All intra-turn choices have been replayed (replay_choices_remaining == 0)
                    // 2. We've passed the baseline choice count from the snapshot
                    //
                    // This ensures that automatic actions (like draws) that happen before the first
                    // NEW choice point are properly suppressed, avoiding duplicate logging.
                    if self.replaying
                        && self.replay_choices_remaining == 0
                        && (self.choice_counter as usize) >= self.baseline_choice_count
                    {
                        eprintln!(
                            "🔍 [REPLAY_CLEAR_BEFORE_CHOICE] choice_counter={}, baseline={}, CLEARING replay mode",
                            self.choice_counter, self.baseline_choice_count
                        );
                        self.replaying = false;
                        if self.verbosity >= VerbosityLevel::Verbose {
                            println!("✅ REPLAY MODE COMPLETE - will present new choice to controller");
                        }
                    } else if self.replaying {
                        eprintln!(
                            "🔍 [REPLAY_STILL_ACTIVE] choice_counter={}, baseline={}, remaining={}",
                            self.choice_counter, self.baseline_choice_count, self.replay_choices_remaining
                        );
                    }

                    // PREAMBLE: Check stop conditions BEFORE printing menu
                    // This ensures snapshots are taken BEFORE presenting the choice to the controller,
                    // and prevents duplicate menu printing when resuming from snapshot.
                    if let Some(result) = self.check_stop_conditions(controller, current_priority)? {
                        return Ok(Some(result));
                    }

                    // Print prompt AFTER checking stop conditions to avoid duplicate output
                    {
                        let view = GameStateView::new(self.game, current_priority);
                        // Print spell ability menu (controlled by show_choice_menu flag)
                        if view.logger().should_show_choice_menu() && !available.is_empty() {
                            print!("{}", format_choice_menu(&view, &available));
                        }
                    } // Drop view before mutable borrow

                    // Ask controller to choose one (or None to pass)
                    let view = GameStateView::new(self.game, current_priority);
                    let choice = controller.choose_spell_ability_to_play(&view, &available);

                    // Log this choice point for snapshot/replay
                    let replay_choice = crate::game::ReplayChoice::SpellAbility(choice.clone());
                    self.log_choice_point(current_priority, Some(replay_choice));

                    choice
                };

                match choice {
                    None => {
                        // Controller chose to pass priority
                        consecutive_passes += 1;
                        let view = GameStateView::new(self.game, current_priority);
                        controller.on_priority_passed(&view);

                        // Switch priority to other player
                        current_priority = if current_priority == active_player {
                            non_active_player
                        } else {
                            active_player
                        };
                    }
                    Some(ability) => {
                        // Controller chose an ability to play
                        consecutive_passes = 0; // Reset pass counter

                        match ability {
                            crate::core::SpellAbility::PlayLand { card_id } => {
                                // Debug: Log state hash before playing land
                                let card_name = self
                                    .game
                                    .cards
                                    .get(card_id)
                                    .map(|c| c.name.as_str())
                                    .unwrap_or("Unknown");
                                let play_msg = format!(
                                    "{} plays {} ({})",
                                    self.get_player_name(current_priority),
                                    card_name,
                                    card_id
                                );
                                self.game.debug_log_state_hash(&play_msg);

                                // Play land - resolves directly (no stack)
                                if let Err(e) = self.game.play_land(current_priority, card_id) {
                                    if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                        eprintln!("  Error playing land: {e}");
                                    }
                                    // Treat failed land play like passing priority to prevent infinite loops
                                    // This can happen if controller makes invalid choices
                                    consecutive_passes += 1;
                                    current_priority = if current_priority == active_player {
                                        non_active_player
                                    } else {
                                        active_player
                                    };
                                    continue;
                                } else {
                                    let card_name = self
                                        .game
                                        .cards
                                        .get(card_id)
                                        .map(|c| c.name.as_str())
                                        .unwrap_or("Unknown");

                                    if self.verbosity >= VerbosityLevel::Normal {
                                        if !self.replaying {
                                            let message = format!(
                                                "{} plays {} ({})",
                                                self.get_player_name(current_priority),
                                                card_name,
                                                card_id
                                            );
                                            self.game.logger.normal(&message);
                                        } else if self.verbosity >= VerbosityLevel::Verbose {
                                            let message = format!(
                                                "[SUPPRESSED] {} plays {} ({})",
                                                self.get_player_name(current_priority),
                                                card_name,
                                                card_id
                                            );
                                            self.game.logger.verbose(&message);
                                        }
                                    }

                                    // MTG Rules 116.3: "If a player takes a special action, that player receives priority afterward."
                                    // MTG Rules 117.3c: "If a player has priority when they cast a spell, activate an ability,
                                    //                     or take a special action, that player receives priority afterward."
                                    // Playing a land is a special action, so the player retains priority.
                                    // Continue loop with same current_priority to give player another action opportunity.
                                    continue;
                                }
                            }
                            crate::core::SpellAbility::CastSpell { card_id } => {
                                // Cast spell using 8-step process
                                // Mana will be tapped during step 6 (NOT here!)

                                let card_name = self
                                    .game
                                    .cards
                                    .get(card_id)
                                    .map(|c| c.name.to_string())
                                    .unwrap_or_else(|_| "Unknown".to_string());

                                // Debug: Log state hash before casting spell
                                let cast_msg = format!(
                                    "{} casts {} ({}) (putting on stack)",
                                    self.get_player_name(current_priority),
                                    card_name,
                                    card_id
                                );
                                self.game.debug_log_state_hash(&cast_msg);

                                if self.verbosity >= VerbosityLevel::Normal {
                                    if !self.replaying {
                                        let message = format!(
                                            "{} casts {} ({}) (putting on stack)",
                                            self.get_player_name(current_priority),
                                            card_name,
                                            card_id
                                        );
                                        self.game.logger.normal(&message);
                                    } else if self.verbosity >= VerbosityLevel::Verbose {
                                        let message = format!(
                                            "[SUPPRESSED] {} casts {} ({}) (putting on stack)",
                                            self.get_player_name(current_priority),
                                            card_name,
                                            card_id
                                        );
                                        self.game.logger.verbose(&message);
                                    }
                                }

                                // Get valid targets BEFORE calling cast_spell_8_step
                                // (we can't borrow controller inside the closure)
                                let valid_targets = self
                                    .game
                                    .get_valid_targets_for_spell(card_id)
                                    .unwrap_or_else(|_| SmallVec::new());

                                // Ask controller to choose targets (only if there are valid targets)
                                let chosen_targets_vec: Vec<CardId> = if valid_targets.is_empty() {
                                    // No targets needed - spell has no targeting effects
                                    Vec::new()
                                } else if valid_targets.len() == 1 {
                                    // Only one valid target - auto-select without calling controller
                                    // This is not a choice, so don't log ChoicePoint
                                    vec![valid_targets[0]]
                                } else {
                                    // Multiple valid targets - ask controller to choose
                                    let view = GameStateView::new(self.game, current_priority);

                                    let chosen_targets = controller.choose_targets(&view, card_id, &valid_targets);

                                    // Log this choice point for snapshot/replay
                                    let replay_choice = crate::game::ReplayChoice::Targets(chosen_targets.clone());
                                    self.log_choice_point(current_priority, Some(replay_choice));

                                    chosen_targets.into_iter().collect()
                                };

                                // Clone for closure (which will move it)
                                let targets_for_callback = chosen_targets_vec.clone();

                                // Create targeting callback
                                let targeting_callback = move |_game: &GameState, _spell_id: CardId| {
                                    // Return the pre-selected targets
                                    targets_for_callback.clone()
                                };

                                // Pre-compute ManaEngine for mana payment (step 6)
                                // This avoids allocating a new ManaEngine inside cast_spell_8_step
                                self.mana_engine.update(self.game, current_priority);

                                // Cast using 8-step process
                                if let Err(e) = self.game.cast_spell_8_step(
                                    current_priority,
                                    card_id,
                                    targeting_callback,
                                    &self.mana_engine,
                                ) {
                                    if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                        let message = format!("Error casting spell: {e}");
                                        self.game.logger.normal(&message);
                                    }
                                    // Treat failed spell cast like passing priority to prevent infinite loops
                                    // This can happen if controller makes invalid choices or mana engine has stale state
                                    consecutive_passes += 1;
                                    current_priority = if current_priority == active_player {
                                        non_active_player
                                    } else {
                                        active_player
                                    };
                                    continue;
                                } else {
                                    // Store targets for this spell (will be used when it resolves)
                                    self.spell_targets.push((card_id, chosen_targets_vec));

                                    // Spell is now on the stack - it will resolve later
                                    // when both players pass priority
                                }
                            }
                            crate::core::SpellAbility::ActivateAbility { card_id, ability_index } => {
                                // Activate ability from a permanent
                                // TODO(mtg-70): This should go on the stack for non-mana abilities

                                // Get the card and ability
                                let card_name = self.game.cards.get(card_id).ok().map(|c| c.name.clone());
                                let ability = self
                                    .game
                                    .cards
                                    .get(card_id)
                                    .ok()
                                    .and_then(|c| c.activated_abilities.get(ability_index).cloned());

                                if let Some(ability) = ability {
                                    if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                        let name = card_name.as_ref().map(|n| n.as_str()).unwrap_or("Unknown");
                                        let message = format!("{} activates ability: {}", name, ability.description);
                                        self.game.logger.normal(&message);
                                    }

                                    // Get valid targets for the ability (before paying costs)
                                    let valid_targets = self
                                        .game
                                        .get_valid_targets_for_ability(card_id, ability_index)
                                        .unwrap_or_else(|_| SmallVec::new());

                                    // Ask controller to choose targets (only if there are valid targets)
                                    let chosen_targets_vec: Vec<CardId> = if valid_targets.is_empty() {
                                        // No targets needed - ability has no targeting effects
                                        Vec::new()
                                    } else if valid_targets.len() == 1 {
                                        // Only one valid target - auto-select without calling controller
                                        // This is not a choice, so don't log ChoicePoint
                                        vec![valid_targets[0]]
                                    } else {
                                        // Multiple valid targets - ask controller to choose
                                        let view = GameStateView::new(self.game, current_priority);

                                        let chosen_targets = controller.choose_targets(&view, card_id, &valid_targets);

                                        // Log this choice point for snapshot/replay
                                        let replay_choice = crate::game::ReplayChoice::Targets(chosen_targets.clone());
                                        self.log_choice_point(current_priority, Some(replay_choice));

                                        chosen_targets.into_iter().collect()
                                    };

                                    // Auto-tap lands for mana costs (if the ability has a mana cost)
                                    // This is the same logic as spell casting (step 6 of cast_spell_8_step)
                                    if let Some(mana_cost) = ability.cost.get_mana_cost() {
                                        // Use ManaEngine to compute proper color-aware tap order
                                        use crate::game::mana_engine::ManaEngine;
                                        use crate::game::mana_payment::{GreedyManaResolver, ManaPaymentResolver};

                                        let mut mana_engine = ManaEngine::new();
                                        mana_engine.update(self.game, current_priority);

                                        // Get ManaSource list from engine (already built with proper production info)
                                        let mana_sources = mana_engine.all_sources();

                                        // Use GreedyManaResolver to compute proper tap order
                                        let resolver = GreedyManaResolver::new();
                                        let mut sources_to_tap = Vec::new();
                                        resolver.compute_tap_order(mana_cost, mana_sources, &mut sources_to_tap);

                                        // Tap lands to add mana to pool
                                        for &source_id in &sources_to_tap {
                                            if let Err(e) =
                                                self.game.tap_for_mana_for_cost(current_priority, source_id, mana_cost)
                                            {
                                                if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                                    eprintln!("    Failed to tap land for mana: {e}");
                                                }
                                                // Continue to next source - partial payment might still work
                                            }
                                        }
                                    }

                                    // Pay costs
                                    if let Err(e) = self.game.pay_ability_cost(current_priority, card_id, &ability.cost)
                                    {
                                        if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                            eprintln!("    Failed to pay cost: {e}");
                                        }
                                        // Treat failed ability activation like passing priority to prevent infinite loops
                                        consecutive_passes += 1;
                                        current_priority = if current_priority == active_player {
                                            non_active_player
                                        } else {
                                            active_player
                                        };
                                        continue;
                                    }

                                    // Execute effects immediately (not on the stack)
                                    // TODO(mtg-70): Put non-mana abilities on the stack
                                    for effect in &ability.effects {
                                        // Fix placeholder player IDs and targets for effects
                                        let fixed_effect = match effect {
                                            crate::core::Effect::AddMana { player, mana } if player.as_u32() == 0 => {
                                                // Replace placeholder with current player
                                                crate::core::Effect::AddMana {
                                                    player: current_priority,
                                                    mana: *mana,
                                                }
                                            }
                                            crate::core::Effect::GainLife { player, amount }
                                                if player.as_u32() == 0 =>
                                            {
                                                // Replace placeholder with current player
                                                crate::core::Effect::GainLife {
                                                    player: current_priority,
                                                    amount: *amount,
                                                }
                                            }
                                            crate::core::Effect::DrawCards { player, count }
                                                if player.as_u32() == 0 =>
                                            {
                                                // Replace placeholder with current player
                                                crate::core::Effect::DrawCards {
                                                    player: current_priority,
                                                    count: *count,
                                                }
                                            }
                                            // Replace placeholder targets with chosen targets
                                            crate::core::Effect::DestroyPermanent { target }
                                                if target.as_u32() == 0 && !chosen_targets_vec.is_empty() =>
                                            {
                                                crate::core::Effect::DestroyPermanent {
                                                    target: chosen_targets_vec[0],
                                                }
                                            }
                                            crate::core::Effect::TapPermanent { target }
                                                if target.as_u32() == 0 && !chosen_targets_vec.is_empty() =>
                                            {
                                                crate::core::Effect::TapPermanent {
                                                    target: chosen_targets_vec[0],
                                                }
                                            }
                                            crate::core::Effect::UntapPermanent { target }
                                                if target.as_u32() == 0 && !chosen_targets_vec.is_empty() =>
                                            {
                                                crate::core::Effect::UntapPermanent {
                                                    target: chosen_targets_vec[0],
                                                }
                                            }
                                            crate::core::Effect::PumpCreature {
                                                target,
                                                power_bonus,
                                                toughness_bonus,
                                            } if target.as_u32() == 0 && !chosen_targets_vec.is_empty() => {
                                                crate::core::Effect::PumpCreature {
                                                    target: chosen_targets_vec[0],
                                                    power_bonus: *power_bonus,
                                                    toughness_bonus: *toughness_bonus,
                                                }
                                            }
                                            crate::core::Effect::PutCounter {
                                                target,
                                                counter_type,
                                                amount,
                                            } if target.as_u32() == 0 && !chosen_targets_vec.is_empty() => {
                                                crate::core::Effect::PutCounter {
                                                    target: chosen_targets_vec[0],
                                                    counter_type: *counter_type,
                                                    amount: *amount,
                                                }
                                            }
                                            crate::core::Effect::RemoveCounter {
                                                target,
                                                counter_type,
                                                amount,
                                            } if target.as_u32() == 0 && !chosen_targets_vec.is_empty() => {
                                                crate::core::Effect::RemoveCounter {
                                                    target: chosen_targets_vec[0],
                                                    counter_type: *counter_type,
                                                    amount: *amount,
                                                }
                                            }
                                            _ => effect.clone(),
                                        };

                                        if let Err(e) = self.game.execute_effect(&fixed_effect) {
                                            if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                                eprintln!("    Failed to execute effect: {e}");
                                            }
                                        }
                                    }
                                } else if self.verbosity >= VerbosityLevel::Normal && !self.replaying {
                                    eprintln!("  Ability not found");
                                }
                            }
                        }

                        // After taking an action, switch priority to other player
                        current_priority = if current_priority == active_player {
                            non_active_player
                        } else {
                            active_player
                        };
                    }
                }
            }

            // Both players passed priority
            // Check if there are spells on the stack to resolve
            if self.game.stack.is_empty() {
                // Stack is empty, priority round is complete
                break;
            }

            // Resolve the top spell from the stack (MTG Rules 608: Resolving Spells and Abilities)
            // In MTG, the stack is LIFO (Last In, First Out)
            if let Some(&spell_id) = self.game.stack.cards.last() {
                self.resolve_top_spell_from_stack(spell_id)?;
                // After resolving a spell, players get priority again
                // Loop continues to give priority
            } else {
                // Stack was reported non-empty but has no cards (shouldn't happen)
                break;
            }
        }

        Ok(None)
    }

    /// Get available attackers for a player
    #[allow(dead_code)] // Legacy v1 interface, will be removed
    fn get_available_attackers(&self, player_id: PlayerId) -> Vec<PlayerAction> {
        let mut actions = Vec::new();

        // Add finish action
        actions.push(PlayerAction::FinishDeclareAttackers);

        // Find creatures that can attack
        for &card_id in &self.game.battlefield.cards {
            if let Ok(card) = self.game.cards.get(card_id) {
                if card.controller == player_id
                    && card.is_creature()
                    && !card.tapped
                    && !self.game.combat.is_attacking(card_id)
                {
                    // TODO: Check for summoning sickness
                    actions.push(PlayerAction::DeclareAttacker(card_id));
                }
            }
        }

        actions
    }

    /// Get available blockers for a player
    #[allow(dead_code)] // Legacy v1 interface, will be removed
    fn get_available_blockers(&self, player_id: PlayerId) -> Vec<PlayerAction> {
        let mut actions = Vec::new();

        // Add finish action
        actions.push(PlayerAction::FinishDeclareBlockers);

        // Get all attacking creatures
        let attackers = self.game.combat.get_attackers();
        if attackers.is_empty() {
            return actions;
        }

        // Find creatures that can block
        for &card_id in &self.game.battlefield.cards {
            if let Ok(card) = self.game.cards.get(card_id) {
                if card.controller == player_id
                    && card.is_creature()
                    && !card.tapped
                    && !self.game.combat.is_blocking(card_id)
                {
                    // For each potential blocker, offer to block each attacker
                    // (For simplicity, we only support blocking one attacker at a time)
                    for &attacker in &attackers {
                        actions.push(PlayerAction::DeclareBlocker {
                            blocker: card_id,
                            attackers: vec![attacker],
                        });
                    }
                }
            }
        }

        actions
    }

    /// Get available actions for a player at current game state
    #[allow(dead_code)] // Legacy v1 interface, will be removed
    fn get_available_actions(&self, player_id: PlayerId) -> Vec<PlayerAction> {
        let mut actions = Vec::new();

        // Always can pass priority
        actions.push(PlayerAction::PassPriority);

        let current_step = self.game.turn.current_step;

        // Can play lands in main phases if player hasn't played one this turn
        if current_step.can_play_lands() {
            if let Ok(player) = self.game.get_player(player_id) {
                if player.can_play_land() {
                    // Find lands in hand
                    if let Some(zones) = self.game.get_player_zones(player_id) {
                        for &card_id in &zones.hand.cards {
                            if let Ok(card) = self.game.cards.get(card_id) {
                                if card.is_land() {
                                    actions.push(PlayerAction::PlayLand(card_id));
                                }
                            }
                        }
                    }
                }
            }
        }

        // Can tap lands for mana
        for &card_id in &self.game.battlefield.cards {
            if let Ok(card) = self.game.cards.get(card_id) {
                if card.owner == player_id && card.is_land() && !card.tapped {
                    actions.push(PlayerAction::TapForMana(card_id));
                }
            }
        }

        // Can cast spells from hand
        if let Some(zones) = self.game.get_player_zones(player_id) {
            for &card_id in &zones.hand.cards {
                if let Ok(card) = self.game.cards.get(card_id) {
                    // Check if card is castable (not a land)
                    if !card.is_land() {
                        // Check if player has enough mana
                        if let Ok(player) = self.game.get_player(player_id) {
                            if player.mana_pool.can_pay(&card.mana_cost) {
                                actions.push(PlayerAction::CastSpell {
                                    card_id,
                                    targets: vec![],
                                });
                            }
                        }
                    }
                }
            }
        }

        actions
    }

    /// Get creatures that can attack for a player (v2 interface)
    ///
    /// Results are sorted by card ID to ensure deterministic ordering for snapshot/resume.
    fn get_available_attacker_creatures(&self, player_id: PlayerId) -> Vec<CardId> {
        let mut creatures = Vec::new();

        for &card_id in &self.game.battlefield.cards {
            if let Ok(card) = self.game.cards.get(card_id) {
                if card.controller == player_id
                    && card.is_creature()
                    && !card.tapped
                    && !self.game.combat.is_attacking(card_id)
                {
                    // Check for summoning sickness
                    // Creatures can't attack the turn they entered unless they have haste
                    let has_summoning_sickness = if let Some(entered_turn) = card.turn_entered_battlefield {
                        entered_turn == self.game.turn.turn_number && !card.has_keyword(&crate::core::Keyword::Haste)
                    } else {
                        false
                    };

                    // Check for defender keyword
                    let has_defender = card.has_defender();

                    if !has_summoning_sickness && !has_defender {
                        creatures.push(card_id);
                    }
                }
            }
        }

        // Sort for deterministic ordering
        creatures.sort();
        creatures
    }

    /// Get creatures that can block for a player (v2 interface)
    ///
    /// Results are sorted by card ID to ensure deterministic ordering for snapshot/resume.
    fn get_available_blocker_creatures(&self, player_id: PlayerId) -> Vec<CardId> {
        let mut creatures = Vec::new();

        for &card_id in &self.game.battlefield.cards {
            if let Ok(card) = self.game.cards.get(card_id) {
                if card.controller == player_id
                    && card.is_creature()
                    && !card.tapped
                    && !self.game.combat.is_blocking(card_id)
                {
                    creatures.push(card_id);
                }
            }
        }

        // Sort for deterministic ordering
        creatures.sort();
        creatures
    }

    /// Get currently attacking creatures (v2 interface)
    fn get_current_attackers(&self) -> Vec<CardId> {
        self.game.combat.get_attackers()
    }

    /// Get lands in player's hand (v2 interface)
    fn get_lands_in_hand(&self, player_id: PlayerId) -> Vec<CardId> {
        let mut lands = Vec::new();

        if let Some(zones) = self.game.get_player_zones(player_id) {
            for &card_id in &zones.hand.cards {
                if let Ok(card) = self.game.cards.get(card_id) {
                    if card.is_land() {
                        lands.push(card_id);
                    }
                }
            }
        }

        lands
    }

    /// Get castable spells in player's hand (v2 interface)
    fn get_castable_spells(&mut self, player_id: PlayerId) -> Vec<CardId> {
        let mut spells = Vec::new();

        // Update the mana engine for this player
        self.mana_engine.update(self.game, player_id);

        // Check if this is the active player (only active player can cast sorceries)
        let is_active_player = self.game.turn.active_player == player_id;

        // Check if it's sorcery speed (Main1 or Main2)
        let is_sorcery_speed = self.game.turn.current_step.is_sorcery_speed();

        // Check if stack is empty (required for sorcery-speed spells)
        // MTG Rules 307.5: Sorceries and creatures can only be cast when stack is empty
        let stack_is_empty = self.game.stack.is_empty();

        if let Some(zones) = self.game.get_player_zones(player_id) {
            for &card_id in &zones.hand.cards {
                if let Ok(card) = self.game.cards.get(card_id) {
                    // Check if card is castable (not a land)
                    if !card.is_land() {
                        // Check timing restrictions
                        let can_cast_now = if card.is_instant() {
                            // Instants can be cast anytime with priority
                            true
                        } else {
                            // Creatures and sorceries require:
                            // - Sorcery speed (Main1 or Main2)
                            // - Active player
                            // - Stack is empty
                            is_sorcery_speed && is_active_player && stack_is_empty
                        };

                        if can_cast_now {
                            // Check if we can pay for this spell's mana cost
                            if self.mana_engine.can_pay(&card.mana_cost) {
                                // For Aura spells, check if there are valid targets
                                // MTG Rule 303.4a: You can only cast an Aura spell if there's a legal object or player it could enchant
                                if card.is_aura() {
                                    // Check if there are valid enchantment targets on the battlefield
                                    let has_valid_targets = self.game.battlefield.cards.iter().any(|&target_id| {
                                        if let Ok(target_card) = self.game.cards.get(target_id) {
                                            // Paralyze enchants creatures, so check for creatures
                                            // TODO: Parse enchant restrictions from card data (e.g., "Enchant creature")
                                            // For now, assume Auras enchant creatures
                                            target_card.is_creature()
                                        } else {
                                            false
                                        }
                                    });

                                    if has_valid_targets {
                                        spells.push(card_id);
                                    }
                                } else if Self::spell_requires_stack_target(card) {
                                    // For counterspells and similar effects, check if stack has valid targets
                                    // MTG Rule 608.2b: If a spell/ability targets, it's countered if all targets are illegal
                                    if !self.game.stack.is_empty() {
                                        spells.push(card_id);
                                    }
                                } else {
                                    spells.push(card_id);
                                }
                            }
                        }
                    }
                }
            }
        }

        spells
    }

    /// Get activatable abilities on player's permanents (v2 interface)
    fn get_activatable_abilities(&mut self, player_id: PlayerId) -> Vec<(CardId, usize)> {
        let mut abilities = Vec::new();

        // Update the mana engine for this player
        self.mana_engine.update(self.game, player_id);

        // Check all permanents controlled by this player
        for &card_id in &self.game.battlefield.cards {
            if let Ok(card) = self.game.cards.get(card_id) {
                // Only check permanents controlled by this player
                if card.controller != player_id {
                    continue;
                }

                // Check each activated ability on this card
                for (ability_index, ability) in card.activated_abilities.iter().enumerate() {
                    // Skip mana abilities for now (they'll be handled specially)
                    if ability.is_mana_ability {
                        continue;
                    }

                    // Check if cost can be paid
                    let mut can_activate = true;

                    // Check tap cost
                    if ability.cost.includes_tap() && card.tapped {
                        can_activate = false;
                    }

                    // Check mana cost
                    if let Some(mana_cost) = ability.cost.get_mana_cost() {
                        if !self.mana_engine.can_pay(mana_cost) {
                            can_activate = false;
                        }
                    }

                    // Check life cost
                    if let Some(life_cost) = ability.cost.get_life_cost() {
                        if let Ok(player) = self.game.get_player(player_id) {
                            if player.life <= life_cost {
                                // Can't pay life cost (would go to 0 or below)
                                can_activate = false;
                            }
                        } else {
                            can_activate = false;
                        }
                    }

                    // TODO: Check other cost types (sacrifice, discard, etc.)
                    // TODO: Check timing restrictions (sorcery speed abilities)
                    // TODO: Check activation limits

                    // TODO(mtg-70): Check if ability has valid targets
                    // For targeting abilities, check that there's at least one valid target
                    if can_activate {
                        // Check if this ability requires targets
                        let valid_targets = self
                            .game
                            .get_valid_targets_for_ability(card_id, ability_index)
                            .unwrap_or_else(|_| SmallVec::new());

                        // If get_valid_targets_for_ability returned an empty list,
                        // it might mean either:
                        // 1. The ability doesn't require targets (non-targeting ability)
                        // 2. The ability requires targets but none are available
                        //
                        // We need to distinguish between these cases.
                        // For now, check if the ability description contains "target"
                        let requires_targets = ability.description.to_lowercase().contains("target");

                        if requires_targets && valid_targets.is_empty() {
                            // Ability requires targets but none are available
                            can_activate = false;
                        }
                    }

                    if can_activate {
                        abilities.push((card_id, ability_index));
                    }
                }
            }
        }

        abilities
    }

    /// Get all available spell abilities for a player
    ///
    /// This matches Java Forge's approach where lands, spells, and activated
    /// abilities are all represented as SpellAbility objects that can be
    /// chosen from a unified list.
    ///
    /// Returns a list of all abilities the player can currently play:
    /// - Land plays (if player can play lands and it's a main phase)
    /// - Castable spells (if player has mana and targeting is valid)
    /// - Activated abilities (TODO: not yet implemented)
    ///
    /// IMPORTANT: Results are sorted by card ID to ensure deterministic ordering.
    /// This is critical for snapshot/resume determinism where choice indices
    /// must map to the same logical cards across runs.
    fn get_available_spell_abilities(&mut self, player_id: PlayerId) -> Vec<crate::core::SpellAbility> {
        use crate::core::SpellAbility;
        let mut abilities = Vec::new();

        // Check if stack is empty (required for sorcery-speed actions)
        let stack_is_empty = self.game.stack.is_empty();

        // Add playable lands (only in main phases when player can play lands AND stack is empty)
        // MTG Rules 307.4: Can only play land when stack is empty and you have priority during your main phase
        if stack_is_empty
            && matches!(self.game.turn.current_step, Step::Main1 | Step::Main2)
            && self.game.turn.active_player == player_id
        {
            if let Ok(player) = self.game.get_player(player_id) {
                if player.can_play_land() {
                    let lands = self.get_lands_in_hand(player_id);
                    for land_id in lands {
                        abilities.push(SpellAbility::PlayLand { card_id: land_id });
                    }
                }
            }
        }

        // Add castable spells
        let spells = self.get_castable_spells(player_id);
        for spell_id in spells {
            abilities.push(SpellAbility::CastSpell { card_id: spell_id });
        }

        // Add activated abilities
        let activatable = self.get_activatable_abilities(player_id);
        for (card_id, ability_index) in activatable {
            abilities.push(SpellAbility::ActivateAbility { card_id, ability_index });
        }

        // Sort by card ID to ensure deterministic ordering
        // This is critical for snapshot/resume: if two runs have the same cards available
        // but in different hand order, we need to present them in the same order so that
        // index-based choice replay (FixedScriptController) selects the same logical card
        abilities.sort_by_key(|ability| match ability {
            SpellAbility::PlayLand { card_id } => *card_id,
            SpellAbility::CastSpell { card_id } => *card_id,
            SpellAbility::ActivateAbility { card_id, .. } => *card_id,
        });

        abilities
    }

    /// Execute a player action
    #[allow(dead_code)] // Legacy v1 interface, will be removed
    fn execute_action(&mut self, player_id: PlayerId, action: &PlayerAction) -> Result<()> {
        if !matches!(action, PlayerAction::PassPriority) {
            let player_name = self.get_player_name(player_id);
            let action_desc = self.describe_action(action);
            self.log_verbose(&format!("{player_name} {action_desc}"));
        }

        match action {
            PlayerAction::PlayLand(card_id) => {
                self.game.play_land(player_id, *card_id)?;
            }
            PlayerAction::TapForMana(card_id) => {
                self.game.tap_for_mana(player_id, *card_id)?;
            }
            PlayerAction::CastSpell { card_id, targets } => {
                // Show spell being cast (added to stack)
                log_if_verbose!(
                    self,
                    "{} casts {}",
                    self.get_player_name(player_id),
                    self.game
                        .cards
                        .get(*card_id)
                        .map(|c| c.name.as_str())
                        .unwrap_or("Unknown")
                );

                self.game.cast_spell(player_id, *card_id, targets.clone())?;

                // Immediately resolve spell (simplified - no stack interaction yet)
                // Legacy v1 path - no targets chosen, rely on auto-targeting
                self.game.resolve_spell(*card_id, &[])?;

                log_if_verbose!(
                    self,
                    "{} resolves",
                    self.game
                        .cards
                        .get(*card_id)
                        .map(|c| c.name.as_str())
                        .unwrap_or("Unknown")
                );
            }
            PlayerAction::DeclareAttacker(card_id) => {
                self.game.declare_attacker(player_id, *card_id)?;
            }
            PlayerAction::DeclareBlocker { blocker, attackers } => {
                self.game.declare_blocker(player_id, *blocker, attackers.clone())?;
            }
            PlayerAction::FinishDeclareAttackers | PlayerAction::FinishDeclareBlockers => {
                // Handled by the combat step logic, not here
            }
            PlayerAction::PassPriority => {
                // Nothing to do
            }
        }
        Ok(())
    }

    /// Describe an action for verbose output
    #[allow(dead_code)] // Legacy v1 interface, will be removed
    fn describe_action(&self, action: &PlayerAction) -> String {
        match action {
            PlayerAction::PlayLand(card_id) => {
                let card_name = self
                    .game
                    .cards
                    .get(*card_id)
                    .map(|c| c.name.as_str())
                    .unwrap_or("Unknown");
                format!("plays {card_name}")
            }
            PlayerAction::TapForMana(card_id) => {
                let card_name = self
                    .game
                    .cards
                    .get(*card_id)
                    .map(|c| c.name.as_str())
                    .unwrap_or("Unknown");
                format!("taps {card_name} for mana")
            }
            PlayerAction::CastSpell { card_id, .. } => {
                let card_name = self
                    .game
                    .cards
                    .get(*card_id)
                    .map(|c| c.name.as_str())
                    .unwrap_or("Unknown");
                format!("casts {card_name}")
            }
            PlayerAction::DeclareAttacker(card_id) => {
                let card_name = self
                    .game
                    .cards
                    .get(*card_id)
                    .map(|c| c.name.as_str())
                    .unwrap_or("Unknown");
                format!("declares {card_name} as attacker")
            }
            PlayerAction::DeclareBlocker { blocker, attackers } => {
                let blocker_name = self
                    .game
                    .cards
                    .get(*blocker)
                    .map(|c| c.name.as_str())
                    .unwrap_or("Unknown");
                let attacker_names: Vec<_> = attackers
                    .iter()
                    .filter_map(|id| self.game.cards.get(*id).ok().map(|c| c.name.as_str()))
                    .collect();
                format!("blocks with {blocker_name} (blocking {attacker_names:?})")
            }
            PlayerAction::FinishDeclareAttackers => "finishes declaring attackers".to_string(),
            PlayerAction::FinishDeclareBlockers => "finishes declaring blockers".to_string(),
            PlayerAction::PassPriority => "passes priority".to_string(),
        }
    }

    /// Check if the game has reached a win condition
    fn check_win_condition(&self) -> Option<GameResult> {
        // Check for player death (life <= 0)
        for player in &self.game.players {
            if player.life <= 0 {
                let winner = self.game.get_other_player_id(player.id);
                return Some(GameResult {
                    winner,
                    turns_played: self.turns_elapsed,
                    end_reason: GameEndReason::PlayerDeath(player.id),
                });
            }
        }

        // Check for decking (empty library when trying to draw)
        for player in &self.game.players {
            if let Some(zones) = self.game.get_player_zones(player.id) {
                if zones.library.is_empty() {
                    let winner = self.game.get_other_player_id(player.id);
                    return Some(GameResult {
                        winner,
                        turns_played: self.turns_elapsed,
                        end_reason: GameEndReason::Decking(player.id),
                    });
                }
            }
        }

        None
    }

    /// Check if a spell requires a target on the stack (e.g., Counterspell)
    ///
    /// Returns true if the spell has effects that target spells on the stack,
    /// meaning it can only be cast when there's a spell to target.
    fn spell_requires_stack_target(card: &crate::core::Card) -> bool {
        use crate::core::Effect;

        // Check if any effect is CounterSpell with a placeholder target
        // Placeholder target (CardId(0)) means the spell needs to choose a target when cast
        card.effects
            .iter()
            .any(|effect| matches!(effect, Effect::CounterSpell { target } if target.as_u32() == 0))
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
        let mut game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
        let alice = { game.players.iter().map(|p| p.id).next().expect("Should have player 1") };

        // Create a tapped land on battlefield
        let land_id = game.next_card_id();
        let mut land = crate::core::Card::new(land_id, "Mountain".to_string(), alice);
        land.types.push(crate::core::CardType::Land);
        land.tap();
        game.cards.insert(land_id, land);
        game.battlefield.add(land_id);

        // Run untap step
        let mut game_loop = GameLoop::new(&mut game);
        game_loop.untap_step().unwrap();

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
        let mut game_loop = GameLoop::new(&mut game);
        game_loop.draw_step(&mut controller1, &mut controller2).unwrap();

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

        let game_loop = GameLoop::new(&mut game);
        let result = game_loop.check_win_condition();

        assert!(result.is_some());
        let result = result.unwrap();
        assert_eq!(result.end_reason, GameEndReason::PlayerDeath(bob));
    }
}
