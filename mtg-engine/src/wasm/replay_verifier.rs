//! Debug scaffolding for the WASM rewind/replay loop.
//!
//! The fancy-TUI WASM controller serves human input by *rewinding* the game to
//! the start of the current turn, *replaying* every choice that was made on
//! that turn (including the new one the user just picked), and continuing
//! forward. Bugs in this loop are insidious: a subtle desync between the
//! original forward pass and the replay can corrupt game state, drop log
//! entries, or duplicate choice points — all without surfacing a clear error.
//!
//! When the "Debug Mode" checkbox in `web/fancy.html` is enabled, the
//! `WasmFancyTuiState` wires this module up to:
//!
//! 1. **Capture** the game-state hash, action count, log count, and the slice
//!    of log entries about to be retracted **before** each rewind.
//! 2. **Cache** the post-rewind, turn-start hash per `turn_number`. If we ever
//!    rewind to the same turn again and produce a different turn-start hash,
//!    rewind is no longer a faithful inverse — fatal.
//! 3. **Verify** the replay by checking that the regenerated log entries match
//!    the captured prefix one-for-one. The N originally-made choices must
//!    produce identical log output on replay; only entries beyond that prefix
//!    (from the user's new (N+1)th choice) are allowed to differ.
//!
//! All checks are zero-cost when verification is disabled: the hot path only
//! reads a bool, and no captures, hashing, or comparisons run.

use crate::game::compute_state_hash;
use crate::game::logger::LogEntry;
use crate::game::GameState;

/// Snapshot taken immediately before (and during) a rewind, used to verify
/// that the subsequent replay reproduces the same intermediate state.
///
/// All fields are populated in two phases:
/// - [`capture_pre_rewind`] fills the "pre" fields just before
///   `UndoLog::rewind_to_turn_start` mutates game state.
/// - [`record_turn_start_hash`] fills `post_rewind_turn_start_hash` after the
///   rewind completes (and before logs are truncated).
#[derive(Debug, Clone)]
pub struct RewindVerification {
    /// Turn number at the rewind start (used for diagnostics and for keying
    /// the per-turn cache of turn-start hashes).
    pub turn_number: u32,
    /// Game-state hash at the moment we asked for a new user choice.
    pub pre_rewind_state_hash: u64,
    /// `undo_log.len()` at the moment we asked for a new user choice.
    pub pre_rewind_action_count: usize,
    /// `logger.log_count()` at the moment we asked for a new user choice.
    pub pre_rewind_log_count: usize,
    /// Log entries that exist at indices `[log_size_at_turn, pre_rewind_log_count)`
    /// when the rewind starts. They are about to be truncated; replay should
    /// regenerate exactly the same prefix when it re-runs the previously-made
    /// choices.
    pub pre_rewind_log_tail: Vec<LogEntry>,
    /// Log buffer length at the turn boundary (where logs are truncated to).
    pub log_size_at_turn: usize,
    /// State hash AFTER rewind, i.e. at turn start. Filled in by
    /// [`record_turn_start_hash`] and compared against any cached value for
    /// the same `turn_number`.
    pub post_rewind_turn_start_hash: u64,
}

/// Result of running the post-replay consistency checks. `Ok` means everything
/// matched; the other variants describe a fatal divergence with enough context
/// to point at the offending action or log entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplayCheckOutcome {
    /// Everything matched: regenerated log prefix is identical to the captured
    /// prefix and the turn-start hash is consistent with prior rewinds.
    Ok,
    /// The post-rewind state hash for `turn_number` differs from a value we
    /// cached on a previous rewind to the same turn — the undo log is no
    /// longer a faithful inverse of forward play for this turn.
    TurnStartHashChanged {
        turn_number: u32,
        expected: u64,
        actual: u64,
    },
    /// Replay produced FEWER log entries than the original forward pass. The
    /// engine should reproduce at least every entry that was retracted by the
    /// rewind (because every previously-made choice is replayed); a shorter
    /// tail means replay stalled or skipped work.
    LogTruncated {
        captured_len: usize,
        replay_tail_len: usize,
        first_missing: Option<String>,
    },
    /// A regenerated log entry differs from the captured original at a
    /// specific position in the truncated tail.
    LogMismatch {
        /// Absolute log buffer index (so the user can correlate with the
        /// global `logger.logs()` view).
        index: usize,
        /// Position within the captured prefix (0-based, easier to scan).
        prefix_offset: usize,
        expected: String,
        actual: String,
        captured_len: usize,
        replay_tail_len: usize,
    },
}

impl ReplayCheckOutcome {
    pub fn is_ok(&self) -> bool {
        matches!(self, ReplayCheckOutcome::Ok)
    }

    /// Render a fatal error message suitable for surfacing in the UI when a
    /// non-`Ok` outcome occurs. Returns `None` for `Ok` so callers can use
    /// `if let Some(msg) = outcome.fatal_message() { ... }`.
    pub fn fatal_message(&self) -> Option<String> {
        match self {
            ReplayCheckOutcome::Ok => None,
            ReplayCheckOutcome::TurnStartHashChanged {
                turn_number,
                expected,
                actual,
            } => Some(format!(
                "REWIND/REPLAY FATAL: turn-start state hash for turn {turn_number} \
                 changed across rewinds (expected {expected:#018x}, got {actual:#018x}). \
                 The undo log is no longer a faithful inverse of forward play."
            )),
            ReplayCheckOutcome::LogTruncated {
                captured_len,
                replay_tail_len,
                first_missing,
            } => Some(format!(
                "REWIND/REPLAY FATAL: replay produced {replay_tail_len} log entries \
                 after the turn boundary, but the captured forward pass had {captured_len}. \
                 First missing entry: {missing}",
                missing = first_missing.as_deref().unwrap_or("<none>")
            )),
            ReplayCheckOutcome::LogMismatch {
                index,
                prefix_offset,
                expected,
                actual,
                captured_len,
                replay_tail_len,
            } => Some(format!(
                "REWIND/REPLAY FATAL: log entry at buffer index {index} \
                 (prefix offset {prefix_offset} of {captured_len}, replay tail length {replay_tail_len}) \
                 diverged.\n  expected: {expected:?}\n  actual:   {actual:?}"
            )),
        }
    }
}

/// Snapshot of the fields that must be observed BEFORE `UndoLog::rewind_to_turn_start`
/// touches game state: state hash, action count, log count, and turn number.
/// The log tail and turn-start hash get filled in afterwards via
/// [`finish_capture`] / [`record_turn_start_hash`].
#[derive(Debug, Clone)]
pub struct PreRewindCapture {
    pub turn_number: u32,
    pub pre_rewind_state_hash: u64,
    pub pre_rewind_action_count: usize,
    pub pre_rewind_log_count: usize,
}

/// First phase of capture: observe state-at-choice-time. Must be called
/// BEFORE the rewind actually mutates `game`.
pub fn capture_pre_rewind(game: &GameState) -> PreRewindCapture {
    PreRewindCapture {
        turn_number: game.turn.turn_number,
        pre_rewind_state_hash: compute_state_hash(game),
        pre_rewind_action_count: game.undo_log.len(),
        pre_rewind_log_count: game.logger.log_count(),
    }
}

/// Second phase of capture: snapshot the log entries that are about to be
/// truncated and produce the final [`RewindVerification`]. Call this AFTER
/// `UndoLog::rewind_to_turn_start` has returned (so we know
/// `log_size_at_turn`) but BEFORE `logger.truncate_to(log_size_at_turn)` runs
/// — otherwise the tail is gone.
///
/// `post_rewind_turn_start_hash` is left at zero — fill it via
/// [`record_turn_start_hash`] after the rewind completes.
pub fn finish_capture(pre: PreRewindCapture, game: &GameState, log_size_at_turn: usize) -> RewindVerification {
    let logs = game.logger.logs();
    let pre_rewind_log_tail = if log_size_at_turn < pre.pre_rewind_log_count {
        logs[log_size_at_turn..pre.pre_rewind_log_count].to_vec()
    } else {
        Vec::new()
    };
    RewindVerification {
        turn_number: pre.turn_number,
        pre_rewind_state_hash: pre.pre_rewind_state_hash,
        pre_rewind_action_count: pre.pre_rewind_action_count,
        pre_rewind_log_count: pre.pre_rewind_log_count,
        pre_rewind_log_tail,
        log_size_at_turn,
        post_rewind_turn_start_hash: 0,
    }
}

/// Fill in the post-rewind hash. Call this after `UndoLog::rewind_to_turn_start`
/// has restored the game state but BEFORE the engine starts replaying choices.
pub fn record_turn_start_hash(verification: &mut RewindVerification, game: &GameState) {
    verification.post_rewind_turn_start_hash = compute_state_hash(game);
}

/// Run all post-replay consistency checks against the live `game` state.
///
/// `prior_turn_start_hash` is the hash we previously cached for this turn (if
/// any). If supplied and it disagrees with the freshly-recorded
/// `post_rewind_turn_start_hash`, that's a turn-start determinism violation
/// and is reported in preference to log checks (since a corrupt turn-start
/// state will usually cascade into log divergence and the root cause is the
/// hash drift).
pub fn verify_replay(
    verification: &RewindVerification,
    game: &GameState,
    prior_turn_start_hash: Option<u64>,
) -> ReplayCheckOutcome {
    if let Some(prior) = prior_turn_start_hash {
        if prior != verification.post_rewind_turn_start_hash {
            return ReplayCheckOutcome::TurnStartHashChanged {
                turn_number: verification.turn_number,
                expected: prior,
                actual: verification.post_rewind_turn_start_hash,
            };
        }
    }

    let captured_len = verification.pre_rewind_log_tail.len();
    let logs = game.logger.logs();
    let total_replay_len = logs.len();
    let replay_tail_len = total_replay_len.saturating_sub(verification.log_size_at_turn);

    if replay_tail_len < captured_len {
        let first_missing = verification
            .pre_rewind_log_tail
            .get(replay_tail_len)
            .map(|e| e.message.clone());
        return ReplayCheckOutcome::LogTruncated {
            captured_len,
            replay_tail_len,
            first_missing,
        };
    }

    for offset in 0..captured_len {
        let captured = &verification.pre_rewind_log_tail[offset];
        let actual = &logs[verification.log_size_at_turn + offset];
        if captured.message != actual.message {
            return ReplayCheckOutcome::LogMismatch {
                index: verification.log_size_at_turn + offset,
                prefix_offset: offset,
                expected: captured.message.clone(),
                actual: actual.message.clone(),
                captured_len,
                replay_tail_len,
            };
        }
    }

    ReplayCheckOutcome::Ok
}

#[cfg(test)]
#[allow(clippy::wildcard_enum_match_arm)] // Tests use wildcards in panic branches
mod tests {
    use super::*;
    use crate::game::logger::LogEntry;
    use crate::game::{GameState, VerbosityLevel};

    /// Build a minimal `GameState` whose only interesting field is its log
    /// buffer. The verifier doesn't read any other game fields directly
    /// (state-hash determinism is checked via `compute_state_hash`, which runs
    /// here too — we're just not exercising it across mutations in unit
    /// tests, so the hash is stable across calls within a single test).
    fn fresh_game_with_logs(messages: &[&str]) -> GameState {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        // The default OutputMode is Stdout (no buffer capture). Tests need
        // the in-memory buffer populated to exercise verifier behaviour.
        game.logger.set_output_mode(crate::game::logger::OutputMode::Memory);
        for msg in messages {
            // gamelog() is the public path that goes through the buffer; using
            // it keeps us coupled to the same code path as production logging.
            game.logger.gamelog(msg);
        }
        game
    }

    fn make_log_entry(msg: &str) -> LogEntry {
        LogEntry {
            level: VerbosityLevel::Normal,
            message: msg.to_string(),
            category: None,
        }
    }

    #[test]
    fn test_verify_replay_ok_when_logs_match_exactly() {
        // Pretend the rewind boundary is at log index 1 (so "turn header" stays
        // and "choice A" + "choice B" are part of the captured tail).
        let game = fresh_game_with_logs(&["turn header", "choice A", "choice B"]);
        let pre = capture_pre_rewind(&game);
        let mut verification = finish_capture(pre, &game, 1);
        record_turn_start_hash(&mut verification, &game);

        // Replay produced the same two tail entries (the simulated "replay"
        // here is just the same game; `truncate_to` was never called).
        let outcome = verify_replay(&verification, &game, None);
        assert_eq!(outcome, ReplayCheckOutcome::Ok);
    }

    #[test]
    fn test_verify_replay_detects_log_message_mismatch() {
        // We don't need an actual forward-pass game here — the verifier only
        // reads `pre_rewind_log_tail`, so we hand-build the verification.
        let verification = RewindVerification {
            turn_number: 1,
            pre_rewind_state_hash: 0xDEAD_BEEF,
            pre_rewind_action_count: 7,
            pre_rewind_log_count: 3,
            pre_rewind_log_tail: vec![
                make_log_entry("draws Lightning Bolt"),
                make_log_entry("discards Mountain"),
            ],
            log_size_at_turn: 1,
            post_rewind_turn_start_hash: 0xCAFE,
        };

        // Build a "replay" log buffer where the second entry diverged
        // (regenerated as "discards Forest" instead of "discards Mountain").
        let replayed = fresh_game_with_logs(&["turn header", "draws Lightning Bolt", "discards Forest"]);
        // Simulate a third entry from the user's new (N+1)th choice so the
        // replay tail is longer than the captured prefix; the verifier must
        // STILL flag the mismatch in the prefix.
        replayed.logger.gamelog("attacks with Grizzly Bears");

        let outcome = verify_replay(&verification, &replayed, Some(0xCAFE));
        match outcome {
            ReplayCheckOutcome::LogMismatch {
                index,
                prefix_offset,
                expected,
                actual,
                captured_len,
                replay_tail_len,
            } => {
                assert_eq!(index, 2, "absolute log buffer index of the mismatch");
                assert_eq!(prefix_offset, 1, "second entry of the captured prefix");
                assert_eq!(expected, "discards Mountain");
                assert_eq!(actual, "discards Forest");
                assert_eq!(captured_len, 2);
                assert_eq!(replay_tail_len, 3);
            }
            other => panic!("expected LogMismatch, got {:?}", other),
        }
    }

    #[test]
    fn test_verify_replay_flags_truncated_tail() {
        // Captured 3 entries but replay only regenerated 1 — replay stalled
        // before consuming all of the previously-made choices.
        let verification = RewindVerification {
            turn_number: 2,
            pre_rewind_state_hash: 1,
            pre_rewind_action_count: 10,
            pre_rewind_log_count: 4,
            pre_rewind_log_tail: vec![
                make_log_entry("entry A"),
                make_log_entry("entry B"),
                make_log_entry("entry C"),
            ],
            log_size_at_turn: 1,
            post_rewind_turn_start_hash: 0x1234,
        };

        let game = fresh_game_with_logs(&["preserved turn header", "entry A"]);
        let outcome = verify_replay(&verification, &game, Some(0x1234));
        match outcome {
            ReplayCheckOutcome::LogTruncated {
                captured_len,
                replay_tail_len,
                first_missing,
            } => {
                assert_eq!(captured_len, 3);
                assert_eq!(replay_tail_len, 1);
                assert_eq!(first_missing.as_deref(), Some("entry B"));
            }
            other => panic!("expected LogTruncated, got {:?}", other),
        }
    }

    #[test]
    fn test_verify_replay_flags_turn_start_hash_drift() {
        // We rewound to turn 3 once and cached its turn-start hash. On a later
        // rewind to the same turn the hash differs — fatal.
        let verification = RewindVerification {
            turn_number: 3,
            pre_rewind_state_hash: 999,
            pre_rewind_action_count: 0,
            pre_rewind_log_count: 0,
            pre_rewind_log_tail: Vec::new(),
            log_size_at_turn: 0,
            post_rewind_turn_start_hash: 0xABCD_EF01_2345_6789,
        };

        // Logs are empty and identical, so the only thing that *could* fail is
        // the turn-start hash check.
        let game = fresh_game_with_logs(&[]);
        let outcome = verify_replay(&verification, &game, Some(0x1111_2222_3333_4444));
        match outcome {
            ReplayCheckOutcome::TurnStartHashChanged {
                turn_number,
                expected,
                actual,
            } => {
                assert_eq!(turn_number, 3);
                assert_eq!(expected, 0x1111_2222_3333_4444);
                assert_eq!(actual, 0xABCD_EF01_2345_6789);
            }
            other => panic!("expected TurnStartHashChanged, got {:?}", other),
        }
    }

    #[test]
    fn test_capture_pre_rewind_snapshots_only_the_tail() {
        // Buffer has 4 entries; with log_size_at_turn=2 only the last two
        // are part of the captured tail.
        let game = fresh_game_with_logs(&["turn 1 header", "turn 1 stuff", "choice X", "choice Y"]);
        let pre = capture_pre_rewind(&game);
        let verification = finish_capture(pre, &game, 2);
        assert_eq!(verification.pre_rewind_log_count, 4);
        assert_eq!(verification.log_size_at_turn, 2);
        assert_eq!(verification.pre_rewind_log_tail.len(), 2);
        assert_eq!(verification.pre_rewind_log_tail[0].message, "choice X");
        assert_eq!(verification.pre_rewind_log_tail[1].message, "choice Y");
        assert_eq!(verification.post_rewind_turn_start_hash, 0); // not yet recorded
    }

    #[test]
    fn test_fatal_message_renders_for_each_non_ok_variant() {
        assert!(ReplayCheckOutcome::Ok.fatal_message().is_none());

        let m = ReplayCheckOutcome::TurnStartHashChanged {
            turn_number: 5,
            expected: 0xAA,
            actual: 0xBB,
        }
        .fatal_message()
        .expect("turn-start hash variant must render a message");
        assert!(m.contains("turn 5"), "message must reference the turn number: {m}");

        let m = ReplayCheckOutcome::LogTruncated {
            captured_len: 3,
            replay_tail_len: 1,
            first_missing: Some("entry B".into()),
        }
        .fatal_message()
        .expect("truncated variant must render a message");
        assert!(
            m.contains("entry B"),
            "message must reference the first missing entry: {m}"
        );

        let m = ReplayCheckOutcome::LogMismatch {
            index: 7,
            prefix_offset: 2,
            expected: "expected X".into(),
            actual: "actual Y".into(),
            captured_len: 5,
            replay_tail_len: 8,
        }
        .fatal_message()
        .expect("mismatch variant must render a message");
        assert!(m.contains("expected X") && m.contains("actual Y"));
    }
}
