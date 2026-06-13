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
//! - **Semantic anti-overfitting**: `PASS_UNTIL turn=N,phase=PHASE` — pass priority until a
//!   specific turn+phase combination, indifferent to what triggers fire in between.
//! - **Examples**:
//!   - `play swamp` - Play a land
//!   - `cast "Black Knight"` - Cast a spell
//!   - `equip accorder` - Activate Equipment's Equip ability
//!   - `activate forest` - Activate mana ability (first ability if multiple)
//!   - `activate forest[2]` - Activate second ability (1-indexed)
//!   - `PASS_UNTIL turn=3,phase=MAIN2` - Pass until turn 3 post-combat main phase
//!   - `PASS_UNTIL phase=COMBAT` - Pass until combat phase this (or the next) turn

use crate::core::SpellAbility;
use crate::game::controller::{sort_spell_abilities, GameStateView};
use crate::game::Step;

// ---------------------------------------------------------------------------
// PASS_UNTIL: semantic anti-overfitting primitive
// ---------------------------------------------------------------------------

/// The parsed target of a `PASS_UNTIL` command.
///
/// A `PASS_UNTIL` command causes the controller to pass priority on every
/// callback until the game reaches the specified turn+phase combination.
/// Once the condition is satisfied, the command is consumed and the
/// controller resumes normal script execution.
///
/// Only public game information (turn number and current step) is used —
/// this is information-independent and network-deterministic.
///
/// # Syntax
///
/// ```text
/// PASS_UNTIL turn=N,phase=PHASE
/// PASS_UNTIL turn=N phase=PHASE   (spaces as delimiter)
/// PASS_UNTIL phase=PHASE          (omit turn to match any future turn)
/// ```
///
/// `PHASE` is matched against [`Step::from_script_name`] (case-insensitive,
/// whitespace-stripped). Both `turn=` and `phase=` accept any ordering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PassUntilCondition {
    /// If `Some(n)`, pass until this turn number (1-based). If `None`, match
    /// any turn (useful for "pass until the next MAIN2 of any turn").
    pub turn: Option<u32>,
    /// The step to wait for. `None` means "any step of the target turn".
    pub step: Option<Step>,
}

impl PassUntilCondition {
    /// Returns `true` when the current game state satisfies this condition.
    ///
    /// The condition is met when BOTH:
    ///  - `turn` is `None`, OR the current turn number >= the target turn
    ///  - `step` is `None`, OR the current step == the target step
    ///
    /// We use `>=` for turn so that a condition for turn 3 is also satisfied
    /// if turns are skipped somehow (extra-turn effects, etc.).
    #[must_use]
    pub fn is_satisfied(&self, turn_number: u32, current_step: Step) -> bool {
        let turn_ok = match self.turn {
            None => true,
            Some(t) => turn_number >= t,
        };
        let step_ok = match self.step {
            None => true,
            Some(s) => current_step == s,
        };
        turn_ok && step_ok
    }
}

/// Parse a `PASS_UNTIL` command string into a [`PassUntilCondition`].
///
/// Accepts `PASS_UNTIL` (case-insensitive) as the verb, followed by
/// one or more `key=value` pairs separated by commas or whitespace:
///
/// - `turn=N`   — target turn number (positive integer)
/// - `phase=P`  — target step name (parsed via [`Step::from_script_name`])
///
/// Returns `None` if the command does not start with `PASS_UNTIL`, or
/// `Err(String)` with a diagnostic if the verb matches but the arguments
/// are malformed.
///
/// # Examples
/// ```ignore
/// assert!(parse_pass_until("PASS_UNTIL turn=3,phase=MAIN2").is_some());
/// assert!(parse_pass_until("pass_until phase=COMBAT").is_some());
/// assert!(parse_pass_until("cast bolt").is_none());
/// ```
pub fn parse_pass_until(command: &str) -> Option<Result<PassUntilCondition, String>> {
    let trimmed = command.trim();
    // Case-insensitive prefix check
    let rest = trimmed
        .to_lowercase()
        .strip_prefix("pass_until")
        .map(|_| &trimmed[10..])?; // keep original case for value parsing

    // Split the remainder on commas and whitespace tokens
    // e.g. " turn=3,phase=MAIN2" → ["turn=3", "phase=MAIN2"]
    let tokens: Vec<&str> = rest
        .split(|c: char| c == ',' || c.is_ascii_whitespace())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();

    let mut turn: Option<u32> = None;
    let mut step: Option<Step> = None;

    for token in &tokens {
        // Each token must be key=value
        let mut parts = token.splitn(2, '=');
        let key = match parts.next() {
            Some(k) => k.trim().to_lowercase(),
            None => return Some(Err(format!("PASS_UNTIL: malformed token '{token}'"))),
        };
        let value = match parts.next() {
            Some(v) => v.trim(),
            None => return Some(Err(format!("PASS_UNTIL: token '{token}' is missing '=' separator"))),
        };

        match key.as_str() {
            "turn" => {
                let n: u32 = match value.parse() {
                    Ok(n) => n,
                    Err(_) => {
                        return Some(Err(format!(
                            "PASS_UNTIL: 'turn' value '{value}' is not a valid positive integer"
                        )))
                    }
                };
                turn = Some(n);
            }
            "phase" | "step" => {
                let s = match Step::from_script_name(value) {
                    Some(s) => s,
                    None => {
                        return Some(Err(format!(
                            "PASS_UNTIL: unknown phase/step name '{value}'. \
                             Valid names: untap, upkeep, draw, main1, beginCombat, \
                             declareAttackers, declareBlockers, combatDamage, endCombat, \
                             main2, end, cleanup"
                        )))
                    }
                };
                step = Some(s);
            }
            other => {
                return Some(Err(format!(
                    "PASS_UNTIL: unknown key '{other}' (expected 'turn' or 'phase')"
                )));
            }
        }
    }

    Some(Ok(PassUntilCondition { turn, step }))
}

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
        // Find matching PlayLand or PlayLandFromLibrary ability
        for ability in available {
            match ability {
                SpellAbility::PlayLand { card_id } | SpellAbility::PlayLandFromLibrary { card_id } => {
                    if let Some(card_name) = view.card_name(*card_id) {
                        if card_matches(&card_name, card_pattern) {
                            return Some(ability.clone());
                        }
                    }
                }
                SpellAbility::CastSpell { .. }
                | SpellAbility::ActivateAbility { .. }
                | SpellAbility::CastFromExile { .. }
                | SpellAbility::CastFromCommand { .. }
                | SpellAbility::Cycle { .. }
                | SpellAbility::CastFromGraveyard { .. }
                | SpellAbility::CastAdventure { .. }
                | SpellAbility::CastFromHandWithAltCost { .. }
                | SpellAbility::CastFromLibrary { .. } => {}
            }
        }
    } else if let Some(card_pattern) = cmd.strip_prefix("cast ") {
        // Find matching CastSpell or CastFromCommand ability
        for ability in available {
            match ability {
                SpellAbility::CastSpell { card_id }
                | SpellAbility::CastFromCommand { card_id, .. }
                | SpellAbility::CastFromGraveyard { card_id, .. } => {
                    if let Some(card_name) = view.card_name(*card_id) {
                        if card_matches(&card_name, card_pattern) {
                            return Some(ability.clone());
                        }
                    }
                }
                // Adventure cast matches against the ADVENTURE-face name
                // (e.g. "cast Stomp (Adventure)"), keeping it distinct from the
                // creature-half cast that matches the creature name. The
                // " (Adventure)" disambiguator suffix is stripped before the
                // name comparison.
                SpellAbility::CastAdventure { card_id } => {
                    // `card_pattern` is already lowercased; strip the lowercase
                    // " (adventure)" disambiguator suffix before name matching.
                    let adv_pattern = card_pattern
                        .trim_end()
                        .strip_suffix("(adventure)")
                        .unwrap_or(card_pattern)
                        .trim_end();
                    if let Some(adv_name) = view.adventure_name(*card_id) {
                        if card_matches(&adv_name, adv_pattern) {
                            return Some(ability.clone());
                        }
                    }
                }
                // A `cast <name>` command also matches a from-exile cast of a
                // card by that name (Adventure creature half from exile, Suspend,
                // Airbend, ...) so fixed-input scripts can drive it. The exile
                // form is matched after the in-hand forms above, so a same-named
                // hand card is preferred when both are offered.
                SpellAbility::CastFromExile { card_id, .. } => {
                    if let Some(card_name) = view.card_name(*card_id) {
                        if card_matches(&card_name, card_pattern) {
                            return Some(ability.clone());
                        }
                    }
                }
                SpellAbility::CastFromHandWithAltCost { card_id, .. } | SpellAbility::CastFromLibrary { card_id } => {
                    if let Some(card_name) = view.card_name(*card_id) {
                        if card_matches(&card_name, card_pattern) {
                            return Some(ability.clone());
                        }
                    }
                }
                SpellAbility::PlayLand { .. }
                | SpellAbility::ActivateAbility { .. }
                | SpellAbility::Cycle { .. }
                | SpellAbility::PlayLandFromLibrary { .. } => {}
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

    // -----------------------------------------------------------------------
    // PASS_UNTIL parsing tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_pass_until_not_matched() {
        assert!(parse_pass_until("cast bolt").is_none());
        assert!(parse_pass_until("play swamp").is_none());
        assert!(parse_pass_until("pass").is_none());
        assert!(parse_pass_until("0").is_none());
    }

    #[test]
    fn test_parse_pass_until_turn_and_phase() {
        let result = parse_pass_until("PASS_UNTIL turn=3,phase=MAIN2")
            .expect("should match")
            .expect("should parse ok");
        assert_eq!(result.turn, Some(3));
        assert_eq!(result.step, Some(Step::Main2));
    }

    #[test]
    fn test_parse_pass_until_case_insensitive() {
        let result = parse_pass_until("pass_until turn=1,phase=main1")
            .expect("should match")
            .expect("should parse ok");
        assert_eq!(result.turn, Some(1));
        assert_eq!(result.step, Some(Step::Main1));
    }

    #[test]
    fn test_parse_pass_until_phase_only() {
        let result = parse_pass_until("PASS_UNTIL phase=COMBAT")
            .expect("should match")
            .expect("should parse ok");
        assert_eq!(result.turn, None);
        assert_eq!(result.step, Some(Step::BeginCombat));
    }

    #[test]
    fn test_parse_pass_until_turn_only() {
        let result = parse_pass_until("PASS_UNTIL turn=5")
            .expect("should match")
            .expect("should parse ok");
        assert_eq!(result.turn, Some(5));
        assert_eq!(result.step, None);
    }

    #[test]
    fn test_parse_pass_until_space_separator() {
        // Spaces as delimiter (alternative to commas)
        let result = parse_pass_until("PASS_UNTIL turn=2 phase=MAIN2")
            .expect("should match")
            .expect("should parse ok");
        assert_eq!(result.turn, Some(2));
        assert_eq!(result.step, Some(Step::Main2));
    }

    #[test]
    fn test_parse_pass_until_various_phase_names() {
        let cases = [
            ("PASS_UNTIL phase=beginCombat", Step::BeginCombat),
            ("PASS_UNTIL phase=declareAttackers", Step::DeclareAttackers),
            ("PASS_UNTIL phase=declareBlockers", Step::DeclareBlockers),
            ("PASS_UNTIL phase=combatDamage", Step::CombatDamage),
            ("PASS_UNTIL phase=endCombat", Step::EndCombat),
            ("PASS_UNTIL phase=end", Step::End),
            ("PASS_UNTIL phase=cleanup", Step::Cleanup),
            ("PASS_UNTIL phase=upkeep", Step::Upkeep),
            ("PASS_UNTIL phase=draw", Step::Draw),
        ];
        for (cmd, expected) in &cases {
            let result = parse_pass_until(cmd)
                .unwrap_or_else(|| panic!("no match for '{cmd}'"))
                .unwrap_or_else(|e| panic!("parse error for '{cmd}': {e}"));
            assert_eq!(result.step, Some(*expected), "phase mismatch for '{cmd}'");
        }
    }

    #[test]
    fn test_parse_pass_until_bad_turn_value() {
        let result = parse_pass_until("PASS_UNTIL turn=abc")
            .expect("should match")
            .expect_err("should fail to parse");
        assert!(result.contains("turn"), "error should mention 'turn': {result}");
    }

    #[test]
    fn test_parse_pass_until_unknown_phase() {
        let result = parse_pass_until("PASS_UNTIL phase=NOTAPHASE")
            .expect("should match")
            .expect_err("should fail to parse");
        assert!(
            result.contains("NOTAPHASE"),
            "error should mention the bad name: {result}"
        );
    }

    #[test]
    fn test_parse_pass_until_unknown_key() {
        let result = parse_pass_until("PASS_UNTIL foo=bar")
            .expect("should match")
            .expect_err("should fail to parse");
        assert!(result.contains("foo"), "error should mention bad key: {result}");
    }

    #[test]
    fn test_pass_until_condition_is_satisfied() {
        let cond = PassUntilCondition {
            turn: Some(3),
            step: Some(Step::Main2),
        };
        // Not satisfied yet
        assert!(!cond.is_satisfied(1, Step::Main1));
        assert!(!cond.is_satisfied(2, Step::Main2));
        assert!(!cond.is_satisfied(3, Step::Main1));
        // Satisfied
        assert!(cond.is_satisfied(3, Step::Main2));
        // Over-shot turn is also satisfied
        assert!(cond.is_satisfied(4, Step::Main2));
    }

    #[test]
    fn test_pass_until_condition_phase_only() {
        let cond = PassUntilCondition {
            turn: None,
            step: Some(Step::BeginCombat),
        };
        assert!(!cond.is_satisfied(1, Step::Main1));
        assert!(cond.is_satisfied(1, Step::BeginCombat));
        assert!(cond.is_satisfied(99, Step::BeginCombat));
    }

    #[test]
    fn test_pass_until_condition_turn_only() {
        let cond = PassUntilCondition {
            turn: Some(3),
            step: None,
        };
        assert!(!cond.is_satisfied(2, Step::Main2));
        // Any step on turn 3+ satisfies
        assert!(cond.is_satisfied(3, Step::Untap));
        assert!(cond.is_satisfied(4, Step::End));
    }
}
