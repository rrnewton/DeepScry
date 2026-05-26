//! Game snapshot functionality for stop-and-resume gameplay
//!
//! This module provides snapshot/resume functionality using a replay-based approach:
//! - Snapshots are saved at turn boundaries (clean save points)
//! - Intra-turn choices are logged and replayed on resume
//! - Uses existing undo log infrastructure for choice tracking

use crate::game::state::GameState;
use crate::undo::GameAction;
use serde::{Deserialize, Serialize};

/// Serialization format for snapshots
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "native", derive(clap::ValueEnum))]
pub enum SnapshotFormat {
    /// JSON format (human-readable, slower)
    Json,
    /// Bincode format (binary, faster)
    #[default]
    Bincode,
}

/// Controller type identifier for snapshot persistence
///
/// This enum identifies which controller was used, separate from its state.
/// Even stateless controllers (Heuristic, Zero, TUI) need their type preserved
/// so they can be correctly restored from snapshots.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ControllerType {
    Zero,
    Random,
    Tui,
    Heuristic,
    Fixed,
    #[serde(rename = "fancyfixed")]
    FancyFixed,
    /// Remote controller - receives choices from network server
    /// Used in network games where the opponent's choices are unknown locally
    Remote,
    /// Network controller - server-side controller that proxies to remote client
    /// Used on server to represent remote players, must not auto-pass
    Network,
}

/// Default controller type for backward compatibility with old snapshots
fn default_controller_type() -> ControllerType {
    ControllerType::Zero
}

/// Controller state that can be preserved across snapshot/resume
///
/// This enum allows us to serialize and restore the state of different controller types.
/// Each variant contains the full state needed to reconstruct the controller.
///
/// Stateless controllers (`Zero`, `Interactive`, `Replay` wrappers) don't appear
/// here because they have nothing to preserve. Note that `Heuristic` IS stateful
/// (it carries an RNG seeded from the master `--seed` and consumes it in
/// `is_safe_to_hold_land_for_main2`); a previous version of this comment claimed
/// otherwise and snapshots silently dropped the heuristic RNG state, causing
/// stop-and-go runs to diverge from single-process runs.
/// NOTE: This enum intentionally uses the default *externally-tagged* serde
/// representation (e.g. JSON `{"Random": {...}}`). It must NOT use
/// `#[serde(tag = "...")]` (internally-tagged), `#[serde(untagged)]`, or
/// `#[serde(flatten)]` on any field, because those representations invoke
/// `Deserializer::deserialize_any`, which **bincode does not support**
/// (snapshots in the `.bincode` format would fail to load — see mtg-c232f4).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ControllerState {
    /// Fixed script controller with predetermined choices
    Fixed(crate::game::FixedScriptController),

    /// Random controller with its own RNG state
    Random(crate::game::RandomController),

    /// Heuristic AI controller with its own RNG state
    ///
    /// Preserving this is required for stop-and-go determinism: without it,
    /// every snapshot reload would re-seed the heuristic RNG and the
    /// `is_safe_to_hold_land_for_main2` coin flip would diverge from the
    /// single-process run.
    Heuristic(crate::game::HeuristicController),
    // Remaining controller types don't need state preservation:
    // - Zero: Deterministic, no state needed
    // - Interactive: Human input, no state to preserve
    // - Replay: Wrapper around another controller (state handled separately)
}

/// A game snapshot saved at a turn boundary with intra-turn replay data
///
/// This format allows us to save only at clean turn boundaries while still
/// resuming at arbitrary intra-turn choice points via replay.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameSnapshot {
    /// The complete game state at the start of the turn
    pub game_state: GameState,

    /// Turn number when this snapshot was created
    pub turn_number: u32,

    /// Total number of choices made up to this point (cumulative across all turns)
    ///
    /// This must be restored when resuming to maintain consistent choice IDs.
    /// Without this, choice IDs would restart from 0 on each resume, causing divergence.
    #[serde(default)]
    pub total_choice_count: u32,

    /// Sequence of choice points made during this turn up to the stop point
    ///
    /// These will be replayed (with buffered logging) when resuming
    /// to restore the exact intra-turn state.
    pub intra_turn_choices: Vec<GameAction>,

    /// Controller type for player 1
    ///
    /// Identifies which controller was used (Zero, Random, Heuristic, etc).
    /// This is separate from controller_state - even stateless controllers need their type saved.
    #[serde(default = "default_controller_type")]
    pub p1_controller_type: ControllerType,

    /// Controller type for player 2
    ///
    /// Identifies which controller was used (Zero, Random, Heuristic, etc).
    /// This is separate from controller_state - even stateless controllers need their type saved.
    #[serde(default = "default_controller_type")]
    pub p2_controller_type: ControllerType,

    /// Optional controller state for player 1
    ///
    /// Preserves the full state of player 1's controller across snapshot/resume.
    /// Supports Fixed and Random controllers (others are stateless).
    #[serde(default)]
    pub p1_controller_state: Option<ControllerState>,

    /// Optional controller state for player 2
    ///
    /// Preserves the full state of player 2's controller across snapshot/resume.
    /// Supports Fixed and Random controllers (others are stateless).
    #[serde(default)]
    pub p2_controller_state: Option<ControllerState>,
}

impl GameSnapshot {
    /// Create a new snapshot from a game state, turn number, and choice history
    pub fn new(game_state: GameState, turn_number: u32, intra_turn_choices: Vec<GameAction>) -> Self {
        GameSnapshot {
            game_state,
            turn_number,
            total_choice_count: 0,
            intra_turn_choices,
            p1_controller_type: default_controller_type(),
            p2_controller_type: default_controller_type(),
            p1_controller_state: None,
            p2_controller_state: None,
        }
    }

    /// Create a snapshot with controller types and states preserved
    #[allow(clippy::too_many_arguments)]
    pub fn with_controllers(
        game_state: GameState,
        turn_number: u32,
        total_choice_count: u32,
        intra_turn_choices: Vec<GameAction>,
        p1_controller_type: ControllerType,
        p2_controller_type: ControllerType,
        p1_controller_state: Option<ControllerState>,
        p2_controller_state: Option<ControllerState>,
    ) -> Self {
        GameSnapshot {
            game_state,
            turn_number,
            total_choice_count,
            intra_turn_choices,
            p1_controller_type,
            p2_controller_type,
            p1_controller_state,
            p2_controller_state,
        }
    }

    /// Create a snapshot with controller state preserved (deprecated, use with_controllers)
    #[deprecated(
        since = "0.1.0",
        note = "Use with_controllers() instead to preserve controller types"
    )]
    pub fn with_controller_state(
        game_state: GameState,
        turn_number: u32,
        intra_turn_choices: Vec<GameAction>,
        p1_controller_state: Option<ControllerState>,
        p2_controller_state: Option<ControllerState>,
    ) -> Self {
        GameSnapshot {
            game_state,
            turn_number,
            total_choice_count: 0,
            intra_turn_choices,
            p1_controller_type: default_controller_type(),
            p2_controller_type: default_controller_type(),
            p1_controller_state,
            p2_controller_state,
        }
    }

    /// Save this snapshot to a file (native only - requires filesystem access)
    ///
    /// # Errors
    ///
    /// Returns `SnapshotError::Serialization` if encoding fails, or
    /// `SnapshotError::Io` if file writing fails.
    #[cfg(feature = "native")]
    pub fn save_to_file<P: AsRef<std::path::Path>>(
        &self,
        path: P,
        format: SnapshotFormat,
    ) -> Result<(), SnapshotError> {
        match format {
            SnapshotFormat::Json => {
                let json = serde_json::to_string(self).map_err(|e| SnapshotError::Serialization(e.to_string()))?;
                std::fs::write(path.as_ref(), json).map_err(|e| SnapshotError::Io(e.to_string()))?;
            }
            SnapshotFormat::Bincode => {
                let encoded: Vec<u8> =
                    bincode::serialize(self).map_err(|e| SnapshotError::Serialization(e.to_string()))?;
                std::fs::write(path.as_ref(), encoded).map_err(|e| SnapshotError::Io(e.to_string()))?;
            }
        }
        Ok(())
    }

    /// Load a snapshot from a file (native only - requires filesystem access)
    ///
    /// # Errors
    ///
    /// Returns `SnapshotError::Io` if the file cannot be read, or
    /// `SnapshotError::Deserialization` if the content is invalid.
    #[cfg(feature = "native")]
    pub fn load_from_file<P: AsRef<std::path::Path>>(path: P, format: SnapshotFormat) -> Result<Self, SnapshotError> {
        match format {
            SnapshotFormat::Json => {
                let json = std::fs::read_to_string(path.as_ref()).map_err(|e| SnapshotError::Io(e.to_string()))?;
                let snapshot =
                    serde_json::from_str(&json).map_err(|e| SnapshotError::Deserialization(e.to_string()))?;
                Ok(snapshot)
            }
            SnapshotFormat::Bincode => {
                let encoded = std::fs::read(path.as_ref()).map_err(|e| SnapshotError::Io(e.to_string()))?;
                let decoded: GameSnapshot =
                    bincode::deserialize(&encoded[..]).map_err(|e| SnapshotError::Deserialization(e.to_string()))?;
                Ok(decoded)
            }
        }
    }

    /// Serialize to JSON string (platform-independent)
    ///
    /// # Errors
    ///
    /// Returns `SnapshotError::Serialization` if JSON serialization fails.
    pub fn to_json(&self) -> Result<String, SnapshotError> {
        serde_json::to_string(self).map_err(|e| SnapshotError::Serialization(e.to_string()))
    }

    /// Serialize to bincode bytes (platform-independent)
    ///
    /// # Errors
    ///
    /// Returns `SnapshotError::Serialization` if bincode serialization fails.
    pub fn to_bincode(&self) -> Result<Vec<u8>, SnapshotError> {
        bincode::serialize(self).map_err(|e| SnapshotError::Serialization(e.to_string()))
    }

    /// Deserialize from JSON string (platform-independent)
    ///
    /// # Errors
    ///
    /// Returns `SnapshotError::Deserialization` if JSON parsing fails.
    pub fn from_json(json: &str) -> Result<Self, SnapshotError> {
        serde_json::from_str(json).map_err(|e| SnapshotError::Deserialization(e.to_string()))
    }

    /// Deserialize from bincode bytes (platform-independent)
    ///
    /// # Errors
    ///
    /// Returns `SnapshotError::Deserialization` if bincode parsing fails.
    pub fn from_bincode(bytes: &[u8]) -> Result<Self, SnapshotError> {
        bincode::deserialize(bytes).map_err(|e| SnapshotError::Deserialization(e.to_string()))
    }

    /// Get the number of intra-turn choices in this snapshot
    pub fn choice_count(&self) -> usize {
        self.intra_turn_choices.len()
    }

    /// Check if this snapshot has any intra-turn state to replay
    pub fn has_intra_turn_state(&self) -> bool {
        !self.intra_turn_choices.is_empty()
    }

    /// Extract replay choices from the intra-turn choice log
    ///
    /// Converts GameAction::ChoicePoint entries into a Vec<ReplayChoice> that can be
    /// fed to ReplayController for deterministic replay.
    pub fn extract_replay_choices(&self) -> Vec<crate::game::ReplayChoice> {
        self.intra_turn_choices
            .iter()
            .filter_map(|action| {
                if let GameAction::ChoicePoint { choice, .. } = action {
                    choice.clone()
                } else {
                    None
                }
            })
            .collect()
    }

    /// Extract replay choices for a specific player
    ///
    /// Filters the intra-turn choice log to only include choices made by the specified player.
    /// This is critical for snapshot resume - each controller should only replay its own choices!
    pub fn extract_replay_choices_for_player(
        &self,
        player_id: crate::core::PlayerId,
    ) -> Vec<crate::game::ReplayChoice> {
        self.intra_turn_choices
            .iter()
            .filter_map(|action| {
                if let GameAction::ChoicePoint {
                    player_id: choice_player,
                    choice,
                    ..
                } = action
                {
                    if *choice_player == player_id {
                        choice.clone()
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect()
    }
}

/// Errors that can occur during snapshot operations
#[derive(Debug, thiserror::Error)]
pub enum SnapshotError {
    #[error("Failed to serialize snapshot: {0}")]
    Serialization(String),

    #[error("Failed to deserialize snapshot: {0}")]
    Deserialization(String),

    #[error("I/O error: {0}")]
    Io(String),

    #[error("Invalid snapshot state: {0}")]
    InvalidState(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::PlayerId;

    #[test]
    fn test_snapshot_choice_tracking() {
        // Test the choice tracking logic without a full GameState
        let choices = [
            GameAction::ChoicePoint {
                player_id: PlayerId::new(0),
                choice_id: 1,
                choice: None,
            },
            GameAction::ChoicePoint {
                player_id: PlayerId::new(0),
                choice_id: 2,
                choice: None,
            },
        ];

        assert_eq!(choices.len(), 2);
        assert!(!choices.is_empty());
    }

    #[test]
    fn test_choice_point_serialization() {
        // Test that GameAction::ChoicePoint can be serialized/deserialized
        let choice = GameAction::ChoicePoint {
            player_id: PlayerId::new(1),
            choice_id: 42,
            choice: None,
        };

        let json = serde_json::to_string(&choice).unwrap();
        let deserialized: GameAction = serde_json::from_str(&json).unwrap();

        if let GameAction::ChoicePoint {
            player_id,
            choice_id,
            choice,
        } = deserialized
        {
            assert_eq!(player_id, PlayerId::new(1));
            assert_eq!(choice_id, 42);
            assert!(choice.is_none());
        } else {
            panic!("Failed to deserialize ChoicePoint");
        }
    }
}
