//! WASM Rich Input Controller
//!
//! This controller combines the RichTextInputController's text command parsing
//! with the WasmHumanController's NeedInput pattern for browser-based testing.
//!
//! It allows scripted games using commands like "play swamp", "cast lightning bolt", etc.
//! but operates in the event-driven pattern required for WASM/browser gameplay with
//! the rewind/replay mechanism.
//!
//! ## Design
//!
//! Unlike RichTextInputController which parses commands synchronously, this controller:
//! 1. Parses the command script at creation time
//! 2. Returns `NeedInput` when a choice is needed (triggering rewind/replay)
//! 3. Uses the pending_choice mechanism to provide the next scripted choice
//!
//! This allows E2E testing of the browser TUI's rewind/replay mechanism using
//! deterministic scripts.
//!
//! ## Usage
//!
//! ```rust,ignore
//! let commands = vec![
//!     "play swamp".to_string(),
//!     "*".to_string(),
//!     "play badlands".to_string(),
//!     "*".to_string(),
//!     "cast black knight".to_string(),
//! ];
//! let controller = WasmRichInputController::new(player_id, commands);
//! ```

use crate::core::{CardId, ManaCost, PlayerId, SpellAbility};
use crate::game::controller::{ChoiceContext, ChoiceResult, GameStateView, PlayerController};
use crate::game::snapshot::ControllerType;
use smallvec::SmallVec;

use super::human_controller::PendingChoice;

// Inline parsing helpers (from rich_input_controller, avoiding feature gate dependency)

/// Normalize a string for comparison
fn normalize(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace() && *c != '_')
        .collect::<String>()
        .to_lowercase()
}

/// Check if a card name matches a pattern (prefix matching)
fn card_matches(card_name: &str, pattern: &str) -> bool {
    let normalized_card = normalize(card_name);
    let normalized_pattern = normalize(pattern);
    normalized_card.starts_with(&normalized_pattern)
}

/// Parse a spell ability choice command
///
/// Examples: "play swamp", "cast lightning bolt", "0", "pass"
fn parse_spell_ability_choice(
    command: &str,
    view: &GameStateView,
    available: &[SpellAbility],
) -> Option<SpellAbility> {
    let cmd = command.trim().to_lowercase();

    // Handle numeric choice
    if let Ok(idx) = cmd.parse::<usize>() {
        if idx == 0 {
            return None; // [0] = Pass priority
        } else if idx <= available.len() {
            return Some(available[idx - 1].clone());
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
        for ability in available {
            if let SpellAbility::CastSpell { card_id } = ability {
                if let Some(card_name) = view.card_name(*card_id) {
                    if card_matches(&card_name, card_pattern) {
                        return Some(ability.clone());
                    }
                }
            }
        }
    } else if let Some(card_pattern) = cmd.strip_prefix("equip ") {
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
        // Check for indexed activation: "activate forest[2]"
        let (pattern_part, ability_num) = if let Some(bracket_pos) = card_pattern.find('[') {
            let pattern = &card_pattern[..bracket_pos];
            let num_str = &card_pattern[bracket_pos + 1..];
            if let Some(close_pos) = num_str.find(']') {
                let num = num_str[..close_pos].parse::<usize>().ok();
                (pattern, num)
            } else {
                (pattern, None)
            }
        } else {
            (card_pattern, None)
        };

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

        if !matches.is_empty() {
            if let Some(num) = ability_num {
                if num > 0 && num <= matches.len() {
                    return Some(matches[num - 1].clone());
                }
            } else {
                return Some(matches[0].clone());
            }
        }
    }

    None
}

/// WASM Rich Input Controller
///
/// Combines rich text command parsing with the WASM NeedInput pattern.
/// This allows scripted browser testing with the rewind/replay mechanism.
pub struct WasmRichInputController {
    player_id: PlayerId,
    /// Script of text commands
    commands: Vec<String>,
    /// Current index in the command queue
    current_index: usize,
    /// Whether we're in wildcard mode (waiting for a specific command to match)
    wildcard_mode: bool,
    /// Pending choice to return (set by set_pending_choice or auto from script)
    pending_choice: Option<PendingChoice>,
    /// Whether we've already requested input for the current command
    /// This prevents infinite NeedInput loops
    input_requested: bool,
}

impl WasmRichInputController {
    /// Create a new WASM rich input controller
    ///
    /// # Arguments
    /// * `player_id` - The player ID this controller manages
    /// * `script` - A semicolon-separated string of commands, or a vector of individual commands
    pub fn new(player_id: PlayerId, commands: Vec<String>) -> Self {
        Self {
            player_id,
            commands,
            current_index: 0,
            wildcard_mode: false,
            pending_choice: None,
            input_requested: false,
        }
    }

    /// Create from a semicolon-separated script string
    ///
    /// # Example
    /// ```rust,ignore
    /// let controller = WasmRichInputController::from_script(player_id, "play swamp; * ; cast bolt");
    /// ```
    pub fn from_script(player_id: PlayerId, script: &str) -> Self {
        let commands: Vec<String> = script
            .split(';')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        Self::new(player_id, commands)
    }

    /// Set a pending choice (for manual override or testing)
    pub fn set_pending_choice(&mut self, choice: PendingChoice) {
        self.pending_choice = Some(choice);
        self.input_requested = false;
    }

    /// Check if a pending choice is available
    pub fn has_pending_choice(&self) -> bool {
        self.pending_choice.is_some()
    }

    /// Check if script has more commands
    pub fn has_more_commands(&self) -> bool {
        self.current_index < self.commands.len()
    }

    /// Peek at the current command without consuming it
    fn peek_command(&self) -> Option<&str> {
        if self.current_index < self.commands.len() {
            Some(&self.commands[self.current_index])
        } else {
            None
        }
    }

    /// Check if the current command is a wildcard
    fn current_is_wildcard(&self) -> bool {
        self.peek_command().map(|c| c.trim() == "*").unwrap_or(false)
    }

    /// Consume the current command and advance
    fn consume_command(&mut self) -> Option<String> {
        if self.current_index < self.commands.len() {
            let cmd = self.commands[self.current_index].clone();
            self.current_index += 1;

            // Check if this was a wildcard separator
            if cmd.trim() == "*" {
                self.wildcard_mode = true;
                return self.consume_command(); // Recurse to get actual command
            }

            // Check if next command is wildcard
            if self.current_is_wildcard() {
                self.current_index += 1; // Consume the wildcard
                self.wildcard_mode = true;
            }

            Some(cmd)
        } else {
            None
        }
    }

    /// Try to parse and match a command against available spell abilities
    fn try_match_command(
        &self,
        command: &str,
        view: &GameStateView,
        available: &[SpellAbility],
    ) -> Option<Option<SpellAbility>> {
        // Use inlined parsing logic
        let parsed = parse_spell_ability_choice(command, view, available);

        // Check if this is an explicit pass command
        let cmd_trimmed = command.trim().to_lowercase();
        let is_explicit_pass = cmd_trimmed == "pass" || cmd_trimmed == "p" || cmd_trimmed == "0";

        if parsed.is_some() || is_explicit_pass {
            Some(parsed) // Return Some(Some(ability)) or Some(None) for pass
        } else {
            None // No match
        }
    }
}

impl PlayerController for WasmRichInputController {
    fn player_id(&self) -> PlayerId {
        self.player_id
    }

    fn choose_spell_ability_to_play(
        &mut self,
        view: &GameStateView,
        available: &[SpellAbility],
    ) -> ChoiceResult<Option<SpellAbility>> {
        // First check for a pending choice (from previous NeedInput cycle)
        if let Some(PendingChoice::SpellAbility(choice_idx)) = self.pending_choice.take() {
            self.input_requested = false;
            return match choice_idx {
                None => ChoiceResult::Ok(None),
                Some(0) => ChoiceResult::Ok(None),
                Some(idx) => {
                    let ability_idx = idx - 1;
                    if ability_idx < available.len() {
                        ChoiceResult::Ok(Some(available[ability_idx].clone()))
                    } else {
                        ChoiceResult::Ok(None)
                    }
                }
            };
        }

        // Handle wildcard at beginning
        while self.current_is_wildcard() {
            self.current_index += 1;
            self.wildcard_mode = true;
        }

        // Try to parse the current command
        if let Some(command) = self.peek_command() {
            let command = command.to_string();

            // Try to match the command against available actions
            if let Some(result) = self.try_match_command(&command, view, available) {
                // Match found! Consume the command and return the result
                self.consume_command();
                self.wildcard_mode = false;
                self.input_requested = false;
                return ChoiceResult::Ok(result);
            }

            // No match
            if self.wildcard_mode {
                // In wildcard mode: pass priority silently and wait
                self.input_requested = false;
                return ChoiceResult::Ok(None);
            } else {
                // Not in wildcard mode and command doesn't match
                // Return NeedInput to allow rewind/replay testing
                // But only if we haven't already requested input
                if !self.input_requested {
                    self.input_requested = true;
                    // Format available actions for display
                    let formatted: Vec<String> = std::iter::once("Pass (do nothing)".to_string())
                        .chain(available.iter().map(|ability| match ability {
                            SpellAbility::PlayLand { card_id } => {
                                let name = view.card_name(*card_id).unwrap_or_else(|| "Unknown".to_string());
                                format!("Play land: {}", name)
                            }
                            SpellAbility::CastSpell { card_id } => {
                                let name = view.card_name(*card_id).unwrap_or_else(|| "Unknown".to_string());
                                format!("Cast: {}", name)
                            }
                            SpellAbility::ActivateAbility { card_id, .. } => {
                                let name = view.card_name(*card_id).unwrap_or_else(|| "Unknown".to_string());
                                format!("Activate: {}", name)
                            }
                        }))
                        .collect();

                    return ChoiceResult::NeedInput(ChoiceContext::SpellAbility {
                        available: available.to_vec(),
                        formatted_choices: formatted,
                    });
                } else {
                    // Already requested input but no pending choice - error
                    self.consume_command(); // Consume bad command
                    return ChoiceResult::Error(format!("Command '{}' did not match any available action", command));
                }
            }
        }

        // No more commands - pass priority
        self.input_requested = false;
        ChoiceResult::Ok(None)
    }

    fn choose_targets(
        &mut self,
        _view: &GameStateView,
        _spell: CardId,
        valid_targets: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        // Check for pending choice
        if let Some(PendingChoice::Targets(indices)) = self.pending_choice.take() {
            let targets: SmallVec<[CardId; 4]> = indices
                .into_iter()
                .filter_map(|i| valid_targets.get(i).copied())
                .collect();
            return ChoiceResult::Ok(targets);
        }

        // Auto-select first valid target for simplicity
        if valid_targets.is_empty() {
            ChoiceResult::Ok(SmallVec::new())
        } else {
            ChoiceResult::Ok(smallvec::smallvec![valid_targets[0]])
        }
    }

    fn choose_mana_sources_to_pay(
        &mut self,
        _view: &GameStateView,
        _cost: &ManaCost,
        available_sources: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        // Check for pending choice
        if let Some(PendingChoice::ManaSources(indices)) = self.pending_choice.take() {
            let sources: SmallVec<[CardId; 8]> = indices
                .into_iter()
                .filter_map(|i| available_sources.get(i).copied())
                .collect();
            return ChoiceResult::Ok(sources);
        }

        // Auto-select all available sources
        ChoiceResult::Ok(available_sources.iter().copied().collect())
    }

    fn choose_attackers(
        &mut self,
        _view: &GameStateView,
        _available_creatures: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        // Check for pending choice
        if let Some(PendingChoice::Attackers(indices)) = self.pending_choice.take() {
            let attackers: SmallVec<[CardId; 8]> = indices
                .into_iter()
                .filter_map(|i| _available_creatures.get(i).copied())
                .collect();
            return ChoiceResult::Ok(attackers);
        }

        // Don't attack by default
        ChoiceResult::Ok(SmallVec::new())
    }

    fn choose_blockers(
        &mut self,
        _view: &GameStateView,
        _available_blockers: &[CardId],
        _attackers: &[CardId],
    ) -> ChoiceResult<SmallVec<[(CardId, CardId); 8]>> {
        // Check for pending choice
        if let Some(PendingChoice::Blockers(pairs)) = self.pending_choice.take() {
            let blocks: SmallVec<[(CardId, CardId); 8]> = pairs
                .into_iter()
                .filter_map(|(blocker_idx, attacker_idx)| {
                    let blocker = _available_blockers.get(blocker_idx).copied()?;
                    let attacker = _attackers.get(attacker_idx).copied()?;
                    Some((blocker, attacker))
                })
                .collect();
            return ChoiceResult::Ok(blocks);
        }

        // Don't block by default
        ChoiceResult::Ok(SmallVec::new())
    }

    fn choose_damage_assignment_order(
        &mut self,
        _view: &GameStateView,
        _attacker: CardId,
        blockers: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        // Check for pending choice
        if let Some(PendingChoice::DamageOrder(indices)) = self.pending_choice.take() {
            let order: SmallVec<[CardId; 4]> = indices.into_iter().filter_map(|i| blockers.get(i).copied()).collect();
            return ChoiceResult::Ok(order);
        }

        // Return blockers in order
        ChoiceResult::Ok(blockers.iter().copied().collect())
    }

    fn choose_cards_to_discard(
        &mut self,
        _view: &GameStateView,
        hand: &[CardId],
        count: usize,
    ) -> ChoiceResult<SmallVec<[CardId; 7]>> {
        // Check for pending choice
        if let Some(PendingChoice::Discard(indices)) = self.pending_choice.take() {
            let discards: SmallVec<[CardId; 7]> = indices.into_iter().filter_map(|i| hand.get(i).copied()).collect();
            return ChoiceResult::Ok(discards);
        }

        // Discard first N cards
        ChoiceResult::Ok(hand.iter().take(count).copied().collect())
    }

    fn choose_from_library(&mut self, _view: &GameStateView, valid_cards: &[CardId]) -> ChoiceResult<Option<CardId>> {
        // Check for pending choice
        if let Some(PendingChoice::LibrarySearch(choice)) = self.pending_choice.take() {
            return match choice {
                None => ChoiceResult::Ok(None),
                Some(idx) => {
                    if idx < valid_cards.len() {
                        ChoiceResult::Ok(Some(valid_cards[idx]))
                    } else {
                        ChoiceResult::Ok(None)
                    }
                }
            };
        }

        // Select first valid card
        ChoiceResult::Ok(valid_cards.first().copied())
    }

    fn on_priority_passed(&mut self, _view: &GameStateView) {}
    fn on_game_end(&mut self, _view: &GameStateView, _won: bool) {}

    fn get_controller_type(&self) -> ControllerType {
        ControllerType::Tui
    }

    fn has_more_choices(&self) -> bool {
        self.current_index < self.commands.len() || self.pending_choice.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::EntityId;
    use crate::game::GameState;

    #[test]
    fn test_from_script() {
        let player_id = EntityId::new(1);
        let controller = WasmRichInputController::from_script(player_id, "play swamp; * ; cast bolt");

        assert_eq!(controller.commands.len(), 3);
        assert_eq!(controller.commands[0], "play swamp");
        assert_eq!(controller.commands[1], "*");
        assert_eq!(controller.commands[2], "cast bolt");
    }

    #[test]
    fn test_pending_choice() {
        let player_id = EntityId::new(1);
        let mut controller = WasmRichInputController::new(player_id, vec![]);

        assert!(!controller.has_pending_choice());

        controller.set_pending_choice(PendingChoice::SpellAbility(None));
        assert!(controller.has_pending_choice());

        let game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let view = GameStateView::new(&game, player_id);

        let result = controller.choose_spell_ability_to_play(&view, &[]);
        assert!(matches!(result, ChoiceResult::Ok(None)));
        assert!(!controller.has_pending_choice());
    }

    #[test]
    fn test_pass_command() {
        let player_id = EntityId::new(1);
        let mut controller = WasmRichInputController::new(player_id, vec!["pass".to_string()]);

        let game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let view = GameStateView::new(&game, player_id);

        let result = controller.choose_spell_ability_to_play(&view, &[]);
        assert!(matches!(result, ChoiceResult::Ok(None)));
    }

    #[test]
    fn test_wildcard_mode() {
        let player_id = EntityId::new(1);
        let mut controller = WasmRichInputController::new(player_id, vec!["*".to_string(), "cast bolt".to_string()]);

        let game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let view = GameStateView::new(&game, player_id);

        // In wildcard mode, should pass silently when command doesn't match
        let result = controller.choose_spell_ability_to_play(&view, &[]);
        assert!(matches!(result, ChoiceResult::Ok(None)));
        assert!(controller.wildcard_mode);
    }

    #[test]
    fn test_no_commands_passes() {
        let player_id = EntityId::new(1);
        let mut controller = WasmRichInputController::new(player_id, vec![]);

        let game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let view = GameStateView::new(&game, player_id);

        let result = controller.choose_spell_ability_to_play(&view, &[]);
        assert!(matches!(result, ChoiceResult::Ok(None)));
    }
}
