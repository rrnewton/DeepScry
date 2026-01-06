//! Deterministic state hashing for debugging snapshot/resume and network sync
//!
//! This module provides functionality to compute a deterministic hash of game state,
//! excluding metadata and ephemeral fields. Supports multiple hash modes:
//!
//! - **Replay**: For snapshot/resume debugging (excludes metadata, lands_played_this_turn)
//! - **UndoTest**: For undo verification (excludes only true metadata)
//! - **Network**: For network sync verification (excludes hidden information)
//!
//! The network hash is designed to produce identical results on server and client
//! even though the client doesn't know opponent's hand contents or library order.

use crate::game::GameState;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Fields to exclude when computing deterministic state hash FOR SNAPSHOT/REPLAY
///
/// These fields are metadata or ephemeral state that doesn't affect gameplay:
/// - choice_id: Global counter
/// - undo_log: Not gameplay state
/// - logger: Presentation layer
/// - show_choice_menu, output_mode, etc: Display settings
/// - lands_played_this_turn: Turn-scoped counter (resets on rewind in replay)
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

/// Fields to exclude for UNDO TESTING (stricter - only metadata)
///
/// For undo testing, we want to verify that ALL gameplay state is correctly restored,
/// including fields like lands_played_this_turn that may differ in replay scenarios.
const EXCLUDED_FIELDS_UNDO_TEST: &[&str] = &[
    "choice_id",           // Global counter, not gameplay state
    "undo_log",            // The undo log itself shouldn't be compared
    "logger",              // Presentation layer
    "token_definitions",   // Loaded definitions cache, not gameplay state
    "show_choice_menu",    // Display setting
    "output_mode",         // Display setting
    "output_format",       // Display setting
    "numeric_choices",     // Display setting
    "step_header_printed", // Display state
    "mana_state_version",  // Cache invalidation counter for ManaEngine memoization
];

/// Fields to exclude for NETWORK hash (excludes hidden information)
///
/// Network hashes must produce identical results on server and all clients,
/// even though clients don't know opponent's hand contents or library order.
/// This means we exclude:
/// - All metadata fields (same as undo test)
/// - RNG state (server-only)
/// - Hand contents (private - but SIZE is public)
/// - Library contents (private - but SIZE is public)
const EXCLUDED_FIELDS_NETWORK: &[&str] = &[
    // Metadata (same as undo test)
    "choice_id",
    "undo_log",
    "logger",
    "token_definitions",
    "show_choice_menu",
    "output_mode",
    "output_format",
    "numeric_choices",
    "step_header_printed",
    "mana_state_version",
    "lands_played_this_turn", // Turn-scoped counter
    // Hidden information
    "rng", // Server-only RNG state
           // Note: "hand" and "library" are handled specially - we keep SIZE but not contents
];

/// Hash mode determines which fields are excluded and how zones are handled
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HashMode {
    /// For snapshot/resume debugging (excludes metadata, lands_played_this_turn)
    Replay,
    /// For undo verification (excludes only true metadata)
    UndoTest,
    /// For network sync (excludes hidden information: hand/library contents, RNG)
    Network,
}

/// Compute a deterministic hash of game state
///
/// This serializes the game state to JSON, strips metadata fields,
/// then computes a hash of the cleaned state. Two game states with
/// the same gameplay-relevant state will produce the same hash.
#[allow(clippy::collection_is_never_read)] // False positive: canonical is used via .hash()
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

/// Compute a deterministic hash of game state FOR UNDO TESTING
///
/// This is stricter than compute_state_hash() - it only excludes true metadata,
/// not gameplay state that should be identical after undo/redo.
///
/// Use this in undo tests to verify that ALL gameplay state is correctly restored.
#[allow(clippy::collection_is_never_read)] // False positive: canonical is used via .hash()
pub fn compute_undo_test_hash(game: &GameState) -> u64 {
    // Serialize to JSON
    let json_value = match serde_json::to_value(game) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("Warning: Failed to serialize game state for undo test hashing: {}", e);
            return 0;
        }
    };

    // Strip only true metadata (not gameplay state like lands_played_this_turn)
    let cleaned = strip_metadata_for_undo_test(json_value);

    // Convert to canonical string representation
    let canonical = match serde_json::to_string(&cleaned) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Warning: Failed to canonicalize cleaned state for undo test: {}", e);
            return 0;
        }
    };

    // Hash the canonical string
    let mut hasher = DefaultHasher::new();
    canonical.hash(&mut hasher);
    hasher.finish()
}

/// Recursively strip metadata fields from JSON value (for snapshot/replay)
///
/// Note: Wildcard is intentional - serde_json::Value primitives (Null/Bool/Number/String)
/// pass through unchanged; only Object/Array are recursively processed.
#[allow(clippy::wildcard_enum_match_arm)]
fn strip_metadata(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(mut map) => {
            // Remove excluded fields
            for field in EXCLUDED_FIELDS {
                map.remove(*field);
            }

            // Also remove lands_played_this_turn which can differ after rewind in replay
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

/// Recursively strip metadata fields from JSON value (for undo testing - stricter)
///
/// Note: Wildcard is intentional - serde_json::Value primitives (Null/Bool/Number/String)
/// pass through unchanged; only Object/Array are recursively processed.
#[allow(clippy::wildcard_enum_match_arm)]
fn strip_metadata_for_undo_test(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(mut map) => {
            // Remove ONLY true metadata (not gameplay state)
            for field in EXCLUDED_FIELDS_UNDO_TEST {
                map.remove(*field);
            }

            // Do NOT remove lands_played_this_turn - it's gameplay state that should be restored

            // Recursively clean nested objects
            for (_, v) in map.iter_mut() {
                *v = strip_metadata_for_undo_test(v.clone());
            }

            serde_json::Value::Object(map)
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.into_iter().map(strip_metadata_for_undo_test).collect())
        }
        other => other,
    }
}

/// Format a hash for display (shows first 8 hex digits)
pub fn format_hash(hash: u64) -> String {
    format!("{:08x}", (hash >> 32) as u32)
}

/// Compute a state hash with configurable mode
///
/// This is the unified hash function that supports all modes:
/// - Replay: For snapshot/resume (same as compute_state_hash)
/// - UndoTest: For undo verification (same as compute_undo_test_hash)
/// - Network: For network sync (excludes hidden info, keeps zone sizes)
#[allow(clippy::collection_is_never_read)] // False positive: canonical is used via .hash()
pub fn compute_state_hash_with_mode(game: &GameState, mode: HashMode) -> u64 {
    // Serialize to JSON
    let json_value = match serde_json::to_value(game) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("Warning: Failed to serialize game state for hashing: {}", e);
            return 0;
        }
    };

    // Strip fields based on mode
    let cleaned = strip_fields_for_mode(json_value, mode);

    // For network mode, inject zone sizes (since we stripped contents)
    let final_value = if mode == HashMode::Network {
        inject_zone_sizes(cleaned, game)
    } else {
        cleaned
    };

    // Convert to canonical string representation
    let canonical = match serde_json::to_string(&final_value) {
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

/// Compute a network-safe state hash (excludes hidden information)
///
/// This hash includes only PUBLIC information that both server and client know:
/// - Battlefield state (all cards, tapped status, counters, attachments)
/// - Stack contents
/// - Graveyard contents (public zone)
/// - Exile contents
/// - Life totals
/// - Turn/step info
/// - Hand SIZES (not contents)
/// - Library SIZES (not contents or order)
///
/// Excluded (hidden info):
/// - Hand contents
/// - Library order and contents
/// - RNG state
pub fn compute_network_state_hash(game: &GameState) -> u64 {
    compute_state_hash_with_mode(game, HashMode::Network)
}

/// Compute a network-safe state hash from a GameStateView
///
/// This function computes the same hash as `compute_network_state_hash(game)`
/// but works with a GameStateView, which is what controllers have access to.
///
/// The hash includes only PUBLIC information visible to both server and client:
/// - Turn number, active player, current step/phase
/// - Life totals for all players
/// - Hand SIZES (not contents)
/// - Library SIZES (not contents)
/// - Graveyard contents (public zone)
/// - Battlefield cards with tap status and controller
/// - Stack contents
/// - Action count (undo log length)
///
/// This is designed to produce identical results on server and all clients.
pub fn compute_view_hash(view: &crate::game::controller::GameStateView) -> u64 {
    use crate::core::PlayerId;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();

    // ═══ Game Metadata ═══
    view.turn_number().hash(&mut hasher);
    view.active_player().as_u32().hash(&mut hasher);
    // Use discriminant for step since it may not implement Hash
    std::mem::discriminant(&view.current_step()).hash(&mut hasher);
    view.action_count().hash(&mut hasher);

    // ═══ Player State (2 players) ═══
    for player_idx in 0..2u32 {
        let player_id = PlayerId::new(player_idx);

        // Life total (public)
        view.player_life(player_id).hash(&mut hasher);

        // Hand SIZE only (contents are private)
        // Use player_hand_size() to correctly include hidden cards
        // (opponent's draws we don't have reveals for in network mode)
        view.player_hand_size(player_id).hash(&mut hasher);

        // Library SIZE only (contents are private)
        // Use player_library_size() to correctly handle remote libraries
        // (client shadow state where cards vec is empty but size is tracked)
        view.player_library_size(player_id).hash(&mut hasher);

        // Graveyard contents (public zone - include card IDs in order)
        // Use player_graveyard_size() to correctly include hidden discards
        // (opponent's discards we don't know about in network mode)
        let graveyard = view.player_graveyard(player_id);
        view.player_graveyard_size(player_id).hash(&mut hasher);
        for &card_id in graveyard {
            card_id.as_u32().hash(&mut hasher);
        }
    }

    // ═══ Battlefield (public zone) ═══
    // Sort by CardId for deterministic ordering
    let mut battlefield_cards: Vec<_> = view.battlefield().to_vec();
    battlefield_cards.sort_by_key(|id| id.as_u32());

    battlefield_cards.len().hash(&mut hasher);
    for card_id in battlefield_cards {
        card_id.as_u32().hash(&mut hasher);
        // Include tap status (public information)
        view.is_tapped(card_id).hash(&mut hasher);
        // Include controller if we can get it from the card
        if let Some(card) = view.get_card(card_id) {
            card.controller.as_u32().hash(&mut hasher);
        }
    }

    // ═══ Stack (public zone) ═══
    let stack = view.stack();
    stack.len().hash(&mut hasher);
    for &card_id in stack {
        card_id.as_u32().hash(&mut hasher);
    }

    hasher.finish()
}

/// Build a DebugSyncInfo from a GameStateView
///
/// Creates debug synchronization information for network sync debugging.
/// Used to populate the debug_info field in network messages.
#[cfg(feature = "network")]
pub fn build_debug_sync_info(
    view: &crate::game::controller::GameStateView,
    last_action_count: usize,
) -> crate::network::DebugSyncInfo {
    use crate::core::PlayerId;
    use crate::network::DebugSyncInfo;

    let p1 = PlayerId::new(0);
    let p2 = PlayerId::new(1);

    let last_actions: Vec<String> = if last_action_count > 0 {
        view.format_last_n_actions(last_action_count)
            .lines()
            .map(|s| s.to_string())
            .collect()
    } else {
        Vec::new()
    };

    DebugSyncInfo {
        turn: view.turn_number(),
        phase: format!("{:?}", view.current_step()),
        active_player: view.active_player(),
        priority_player: None, // Would need more context to determine
        life_totals: [view.player_life(p1), view.player_life(p2)],
        // Use player_hand_size() to correctly include hidden cards
        // (opponent's draws we don't have reveals for in network mode)
        hand_sizes: [view.player_hand_size(p1), view.player_hand_size(p2)],
        // Use player_library_size() to correctly handle remote libraries
        // (client shadow state where cards vec is empty but size is tracked)
        library_sizes: [view.player_library_size(p1), view.player_library_size(p2)],
        battlefield_count: view.battlefield().len(),
        stack_size: view.stack().len(),
        // Use player_graveyard_size() to correctly include hidden discards
        // (opponent's discards we don't know about in network mode)
        graveyard_sizes: [view.player_graveyard_size(p1), view.player_graveyard_size(p2)],
        last_actions,
    }
}

/// Recursively strip fields based on hash mode
fn strip_fields_for_mode(value: serde_json::Value, mode: HashMode) -> serde_json::Value {
    let excluded_fields: &[&str] = match mode {
        HashMode::Replay => EXCLUDED_FIELDS,
        HashMode::UndoTest => EXCLUDED_FIELDS_UNDO_TEST,
        HashMode::Network => EXCLUDED_FIELDS_NETWORK,
    };

    strip_fields_recursive(value, excluded_fields, mode)
}

/// Recursively strip specified fields from JSON value
///
/// Note: Wildcard is intentional - serde_json::Value primitives pass through unchanged.
#[allow(clippy::wildcard_enum_match_arm)]
fn strip_fields_recursive(value: serde_json::Value, excluded: &[&str], mode: HashMode) -> serde_json::Value {
    match value {
        serde_json::Value::Object(mut map) => {
            // Remove excluded fields
            for field in excluded {
                map.remove(*field);
            }

            // Mode-specific handling
            match mode {
                HashMode::Replay => {
                    // Also remove lands_played_this_turn which can differ after rewind
                    map.remove("lands_played_this_turn");
                }
                HashMode::UndoTest => {
                    // Keep lands_played_this_turn - it's gameplay state
                }
                HashMode::Network => {
                    // For network mode, we need to handle hand and library specially
                    // We want to keep their SIZE but not their contents
                    // The "cards" array inside hand/library zones is what we strip
                    // This is handled by inject_zone_sizes() after this function
                    if map.contains_key("zone_type") {
                        // This is a CardZone object
                        if let Some(serde_json::Value::String(zone_type)) = map.get("zone_type") {
                            if zone_type == "Hand" || zone_type == "Library" {
                                // Replace cards array with empty array
                                // (size will be injected separately)
                                map.insert("cards".to_string(), serde_json::Value::Array(vec![]));
                            }
                        }
                    }
                }
            }

            // Recursively clean nested objects
            for (_, v) in map.iter_mut() {
                *v = strip_fields_recursive(v.clone(), excluded, mode);
            }

            serde_json::Value::Object(map)
        }
        serde_json::Value::Array(arr) => serde_json::Value::Array(
            arr.into_iter()
                .map(|v| strip_fields_recursive(v, excluded, mode))
                .collect(),
        ),
        other => other,
    }
}

/// Inject zone sizes into the hash input (for network mode)
///
/// After stripping hand/library contents, we add back just the sizes
/// since those are public information per MTG rules.
fn inject_zone_sizes(mut value: serde_json::Value, game: &GameState) -> serde_json::Value {
    if let serde_json::Value::Object(ref mut map) = value {
        let mut zone_sizes = serde_json::Map::new();

        // Add hand and library sizes for each player
        // player_zones is Vec<(PlayerId, PlayerZones)>
        for (i, (_player_id, zones)) in game.player_zones.iter().enumerate() {
            zone_sizes.insert(
                format!("p{}_hand_size", i),
                serde_json::Value::Number(zones.hand.cards.len().into()),
            );
            zone_sizes.insert(
                format!("p{}_library_size", i),
                serde_json::Value::Number(zones.library.len().into()),
            );
        }

        map.insert("_network_zone_sizes".to_string(), serde_json::Value::Object(zone_sizes));
    }
    value
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

    #[test]
    fn test_network_mode_strips_hidden_info() {
        // Test that network mode strips hidden information
        let json = serde_json::json!({
            "turn_number": 5,
            "rng": "some_rng_state",
            "player_zones": [{
                "zone_type": "Hand",
                "owner": 0,
                "cards": [1, 2, 3]  // Should be stripped
            }, {
                "zone_type": "Library",
                "owner": 0,
                "cards": [4, 5, 6, 7]  // Should be stripped
            }, {
                "zone_type": "Graveyard",
                "owner": 0,
                "cards": [8, 9]  // Should NOT be stripped
            }]
        });

        let cleaned = strip_fields_recursive(json, EXCLUDED_FIELDS_NETWORK, HashMode::Network);

        // Check that RNG was removed
        assert!(cleaned.get("rng").is_none());

        // Check zones
        if let Some(serde_json::Value::Array(zones)) = cleaned.get("player_zones") {
            for zone in zones {
                let zone_type = zone.get("zone_type").and_then(|v| v.as_str()).unwrap_or("");
                let cards = zone.get("cards").and_then(|v| v.as_array());

                if zone_type == "Hand" || zone_type == "Library" {
                    // Should have empty cards array
                    assert!(
                        cards.map(|c| c.is_empty()).unwrap_or(false),
                        "Expected empty cards for {} zone",
                        zone_type
                    );
                } else if zone_type == "Graveyard" {
                    // Should still have cards
                    assert!(
                        cards.map(|c| !c.is_empty()).unwrap_or(false),
                        "Expected non-empty cards for Graveyard zone"
                    );
                }
            }
        }
    }

    #[test]
    fn test_hash_mode_enum() {
        // Verify all hash modes are distinct
        assert_ne!(HashMode::Replay, HashMode::UndoTest);
        assert_ne!(HashMode::UndoTest, HashMode::Network);
        assert_ne!(HashMode::Network, HashMode::Replay);
    }
}
