//! Shared command parsing utilities for rich text input controllers
//!
//! This module provides common parsing functionality used by both native and WASM
//! rich input controllers for parsing textual game commands.
//!
//! ## Supported Commands
//!
//! - **Verbs**: Play, Cast, Equip, Activate, Attack, Block, Discard, Pass (case-insensitive)
//! - **Card names**: Case-insensitive, spaces/underscores equivalent, prefix matching allowed
//! - **Numeric choices**: 0 = pass, 1-N = select from available options
//! - **Examples**:
//!   - `play swamp` - Play a land
//!   - `cast "Black Knight"` - Cast a spell
//!   - `equip accorder` - Activate Equipment's Equip ability
//!   - `activate forest` - Activate mana ability (first ability if multiple)
//!   - `activate forest[2]` - Activate second ability (1-indexed)

use crate::core::SpellAbility;
use crate::game::controller::{sort_spell_abilities, GameStateView};

/// Normalize a string for comparison
///
/// - Converts to lowercase
/// - Removes spaces, underscores, and trailing punctuation
/// - Allows prefix matching for card names
///
/// # Examples
/// ```ignore
/// assert_eq!(normalize("Black Knight"), "blackknight");
/// assert_eq!(normalize("Serra_Angel"), "serraangel");
/// assert_eq!(normalize("mountain."), "mountain");
/// ```
pub fn normalize(s: &str) -> String {
    // First, remove whitespace and underscores, then lowercase
    let normalized: String = s
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '_')
        .collect::<String>()
        .to_lowercase();

    // Strip trailing punctuation (periods, commas, etc.)
    normalized
        .trim_end_matches(|c: char| c.is_ascii_punctuation())
        .to_string()
}

/// Check if a card name matches a pattern (prefix matching)
///
/// Both the card name and pattern are normalized before comparison.
///
/// # Examples
/// ```ignore
/// assert!(card_matches("Black Knight", "black"));
/// assert!(card_matches("Serra Angel", "serra"));
/// assert!(!card_matches("Black Knight", "white"));
/// ```
pub fn card_matches(card_name: &str, pattern: &str) -> bool {
    let normalized_card = normalize(card_name);
    let normalized_pattern = normalize(pattern);
    normalized_card.starts_with(&normalized_pattern)
}

/// Parse a spell ability choice command
///
/// Parses commands like:
/// - `play swamp` - Play a land card matching "swamp"
/// - `cast lightning bolt` - Cast a spell matching "lightning bolt"
/// - `equip accorder` - Equip ability on equipment matching "accorder"
/// - `activate forest` - First activate ability on permanent matching "forest"
/// - `activate forest[2]` - Second activate ability on permanent matching "forest"
/// - `0` or `pass` or `p` - Pass priority
/// - `1`, `2`, etc. - Select by menu index (1-indexed, 0 = pass)
///
/// Returns `Some(ability)` if a matching ability is found, `None` for pass/no match.
pub fn parse_spell_ability_choice(
    command: &str,
    view: &GameStateView,
    available: &[SpellAbility],
) -> Option<SpellAbility> {
    let cmd = command.trim().to_lowercase();

    // Handle numeric choice (matching menu display format from format_choice_menu)
    // format_choice_menu sorts abilities: PlayLand, CastSpell, ActivateAbility
    // [0] = Pass priority (return None)
    // [1] to [N] = sorted[0] to sorted[N-1] (menu indices shifted by 1)
    // Out of bounds values (idx > available.len()) also pass priority
    if let Ok(idx) = cmd.parse::<usize>() {
        if idx == 0 {
            return None; // [0] = Pass priority
        } else if idx <= available.len() {
            // Sort to match format_choice_menu display order
            let sorted = sort_spell_abilities(available);
            return Some(sorted[idx - 1].clone()); // [1] = sorted[0], etc.
        } else {
            return None; // Out of bounds = pass priority
        }
    }

    // Handle "pass" or "p"
    if cmd == "pass" || cmd == "p" {
        return None;
    }

    // Parse verb + card name
    if let Some(card_pattern) = cmd.strip_prefix("play ") {
        // Find matching PlayLand ability
        for ability in available {
            if let SpellAbility::PlayLand { card_id } = ability {
                if let Some(card_name) = view.card_name(*card_id) {
                    if card_matches(&card_name, card_pattern) {
                        return Some(ability.clone());
                    }
                }
            }
        }
    } else if let Some(card_pattern) = cmd.strip_prefix("cast ") {
        // Find matching CastSpell or CastFromCommand ability
        for ability in available {
            match ability {
                SpellAbility::CastSpell { card_id } | SpellAbility::CastFromCommand { card_id, .. } => {
                    if let Some(card_name) = view.card_name(*card_id) {
                        if card_matches(&card_name, card_pattern) {
                            return Some(ability.clone());
                        }
                    }
                }
                SpellAbility::PlayLand { .. }
                | SpellAbility::ActivateAbility { .. }
                | SpellAbility::CastFromExile { .. }
                | SpellAbility::Cycle { .. } => {}
            }
        }
    } else if let Some(card_pattern) = cmd.strip_prefix("equip ") {
        // Find matching ActivateAbility for Equipment
        for ability in available {
            if let SpellAbility::ActivateAbility { card_id, .. } = ability {
                if let Some(card_name) = view.card_name(*card_id) {
                    if card_matches(&card_name, card_pattern) {
                        return Some(ability.clone());
                    }
                }
            }
        }
    } else if let Some(card_pattern) = cmd.strip_prefix("activate ") {
        // Find matching ActivateAbility
        // Format: "activate [card_name]" or "activate [card_name][N]"
        // N is 1-indexed (matching ability_index + 1)

        // Check for indexed activation: "activate forest[2]"
        let (pattern_part, ability_num) = if let Some(bracket_pos) = card_pattern.find('[') {
            let pattern = &card_pattern[..bracket_pos];
            let num_str = &card_pattern[bracket_pos + 1..];
            // Extract number before closing bracket
            if let Some(close_pos) = num_str.find(']') {
                let num = num_str[..close_pos].parse::<usize>().ok();
                (pattern, num)
            } else {
                (pattern, None)
            }
        } else {
            (card_pattern, None)
        };

        // Find all matching abilities
        let mut matches: Vec<&SpellAbility> = Vec::new();
        for ability in available {
            if let SpellAbility::ActivateAbility { card_id, .. } = ability {
                if let Some(card_name) = view.card_name(*card_id) {
                    if card_matches(&card_name, pattern_part) {
                        matches.push(ability);
                    }
                }
            }
        }

        // Select the right match
        if !matches.is_empty() {
            if let Some(num) = ability_num {
                // User specified which ability: 1-indexed
                if num > 0 && num <= matches.len() {
                    return Some(matches[num - 1].clone());
                }
            } else {
                // No number specified - take first match (most common case)
                return Some(matches[0].clone());
            }
        }
    }

    // Command not recognized or no match found
    None
}

/// Check if a command is an explicit pass command
///
/// Returns true for "pass", "p", or "0".
pub fn is_explicit_pass(command: &str) -> bool {
    let cmd_trimmed = command.trim().to_lowercase();
    cmd_trimmed == "pass" || cmd_trimmed == "p" || cmd_trimmed == "0"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize() {
        assert_eq!(normalize("Black Knight"), "blackknight");
        assert_eq!(normalize("Serra_Angel"), "serraangel");
        assert_eq!(normalize("Royal  Assassin"), "royalassassin");
        assert_eq!(normalize("  Lightning Bolt  "), "lightningbolt");
        // Trailing punctuation should be stripped
        assert_eq!(normalize("mountain."), "mountain");
        assert_eq!(normalize("Mountain,"), "mountain");
        assert_eq!(normalize("lightning bolt!"), "lightningbolt");
    }

    #[test]
    fn test_card_matches() {
        assert!(card_matches("Black Knight", "black"));
        assert!(card_matches("Black Knight", "blackkn"));
        assert!(card_matches("Black Knight", "blackknight"));
        assert!(card_matches("Serra Angel", "serra"));
        assert!(card_matches("Serra_Angel", "serra"));
        assert!(!card_matches("Black Knight", "white"));
        assert!(!card_matches("Black Knight", "serra"));
    }

    #[test]
    fn test_is_explicit_pass() {
        assert!(is_explicit_pass("pass"));
        assert!(is_explicit_pass("PASS"));
        assert!(is_explicit_pass("p"));
        assert!(is_explicit_pass("P"));
        assert!(is_explicit_pass("0"));
        assert!(is_explicit_pass("  pass  "));
        assert!(!is_explicit_pass("1"));
        assert!(!is_explicit_pass("play swamp"));
        assert!(!is_explicit_pass("cast bolt"));
    }
}
