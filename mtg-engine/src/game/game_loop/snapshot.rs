//! Snapshot management for game state persistence
//!
//! Handles saving game snapshots at specific choice points for debugging and replay.

use crate::core::PlayerId;
use crate::game::controller::PlayerController;
#[cfg(feature = "native")]
use crate::MtgError;
use crate::Result;

use super::{GameEndReason, GameLoop, GameResult, VerbosityLevel};

impl<'a> GameLoop<'a> {
    /// Log a choice point to the undo log and increment choice counter
    ///
    /// Call this every time a controller makes a decision.
    ///
    /// # Arguments
    /// * `player_id` - The player who made the choice
    /// * `choice` - The actual choice made (for replay), or None if not available
    /// * `prior_log_size` - The logger size BEFORE the controller logged its choice
    pub(super) fn log_choice_point(
        &mut self,
        player_id: PlayerId,
        choice: Option<crate::game::ReplayChoice>,
        prior_log_size: usize,
    ) {
        self.choice_counter += 1;

        // Use the provided prior_log_size (captured BEFORE controller logged)
        // This ensures undo restores to the state before the choice was logged

        self.game.undo_log.log(
            crate::undo::GameAction::ChoicePoint {
                player_id,
                choice_id: self.choice_counter,
                choice,
            },
            prior_log_size,
        );

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
    pub(super) fn check_stop_conditions(
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
                action_count: self.game.action_count(),
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
                        action_count: self.game.action_count(),
                    }));
                }
            }
        }

        Ok(None)
    }

    /// Count how many choices in the undo log match the stop condition filter
    pub(super) fn count_filtered_choices(&self, p1_id: PlayerId, stop_condition: &crate::game::StopCondition) -> usize {
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

    /// Assert that we're stopping at a valid point in the game
    ///
    /// Valid stopping points are:
    /// - After a controller choice (last log line contains ">>> CONTROLLER:")
    /// - At game end (last log line contains "Game Over" or "wins!")
    ///
    /// This helps catch bugs where we might stop mid-action (e.g., in the middle
    /// of combat damage resolution).
    ///
    /// Native only - only used by save_snapshot_and_exit.
    #[cfg(feature = "native")]
    pub(super) fn assert_valid_stopping_point(&self) {
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

            // 3. Turn start (valid for --stop-on-choice at game start before any choices)
            if message.contains(">>> Turn") && message.contains("<<<<") {
                return; // Valid: stopped at turn start before any choices
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
    ///
    /// Native only - requires filesystem access.
    #[cfg(feature = "native")]
    pub(super) fn save_snapshot_and_exit<P: AsRef<std::path::Path>>(
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
            action_count: self.game.action_count(),
        })
    }
}
