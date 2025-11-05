//! Deterministic state hashing for debugging snapshot/resume
//!
//! This module provides functionality to compute a deterministic hash of game state,
//! excluding metadata and ephemeral fields. Useful for tracking exactly when game
//! states diverge during stop-go replay.

use crate::game::GameState;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Fields to exclude when computing deterministic state hash
///
/// These fields are metadata or ephemeral state that doesn't affect gameplay:
/// - choice_id: Global counter
/// - undo_log: Not gameplay state
/// - logger: Presentation layer
/// - show_choice_menu, output_mode, etc: Display settings
/// - lands_played_this_turn: Turn-scoped counter (resets on rewind)
const EXCLUDED_FIELDS: &[&str] = &[
    "choice_id",
    "undo_log",
    "logger",
    "show_choice_menu",
    "output_mode",
    "output_format",
    "numeric_choices",
    "step_header_printed",
];

/// Compute a deterministic hash of game state
///
/// This serializes the game state to JSON, strips metadata fields,
/// then computes a hash of the cleaned state. Two game states with
/// the same gameplay-relevant state will produce the same hash.
pub fn compute_state_hash(game: &GameState) -> u64 {
    // Serialize to JSON
    let json_value = match serde_json::to_value(game) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("Warning: Failed to serialize game state for hashing: {}", e);
            return 0;
        }
    };

    // Strip metadata
    let cleaned = strip_metadata(json_value);

    // Convert to canonical string representation
    let canonical = match serde_json::to_string(&cleaned) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Warning: Failed to canonicalize cleaned state: {}", e);
            return 0;
        }
    };

    // Hash the canonical string
    let mut hasher = DefaultHasher::new();
    canonical.hash(&mut hasher);
    hasher.finish()
}

/// Recursively strip metadata fields from JSON value
fn strip_metadata(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(mut map) => {
            // Remove excluded fields
            for field in EXCLUDED_FIELDS {
                map.remove(*field);
            }

            // Also remove lands_played_this_turn which can differ after rewind
            map.remove("lands_played_this_turn");

            // Recursively clean nested objects
            for (_, v) in map.iter_mut() {
                *v = strip_metadata(v.clone());
            }

            serde_json::Value::Object(map)
        }
        serde_json::Value::Array(arr) => serde_json::Value::Array(arr.into_iter().map(strip_metadata).collect()),
        other => other,
    }
}

/// Format a hash for display (shows first 8 hex digits)
pub fn format_hash(hash: u64) -> String {
    format!("{:08x}", (hash >> 32) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_metadata() {
        let json = serde_json::json!({
            "turn_number": 5,
            "choice_id": 42,
            "undo_log": ["action1", "action2"],
            "player": {
                "life": 20,
                "lands_played_this_turn": 1
            }
        });

        let cleaned = strip_metadata(json);

        assert_eq!(
            cleaned,
            serde_json::json!({
                "turn_number": 5,
                "player": {
                    "life": 20
                }
            })
        );
    }

    #[test]
    fn test_deterministic_hash() {
        // Same JSON should produce same hash
        let json1 = serde_json::json!({"life": 20, "turn": 5});
        let json2 = serde_json::json!({"life": 20, "turn": 5});

        let mut hasher1 = DefaultHasher::new();
        serde_json::to_string(&json1).unwrap().hash(&mut hasher1);
        let hash1 = hasher1.finish();

        let mut hasher2 = DefaultHasher::new();
        serde_json::to_string(&json2).unwrap().hash(&mut hasher2);
        let hash2 = hasher2.finish();

        assert_eq!(hash1, hash2);
    }
}
