//! Rich input controller that parses textual commands
//!
//! This controller accepts rich text commands like "play swamp" or "cast black_knight"
//! and converts them to numeric choices based on available options.
//!
//! ## Command Syntax
//!
//! - **Verbs**: Play, Cast, Equip, Activate, Attack, Block, Discard, Pass (case-insensitive)
//! - **Card names**: Case-insensitive, spaces/underscores equivalent, prefix matching allowed
//! - **Quotes**: Optional for card names at end of command
//! - **Examples**:
//!   - `play swamp` - Play a land
//!   - `cast "Black Knight"` - Cast a spell
//!   - `equip accorder` - Activate Equipment's Equip ability
//!   - `activate forest` - Activate mana ability (first ability if multiple)
//!   - `activate forest[2]` - Activate second ability (1-indexed)
//!   - `attack serra` - Attack with Serra Angel
//!
//! ## Wildcard Separator
//!
//! Use `*` to skip choices until the next command matches:
//! - `play mountain;*;cast fireball` - Play mountain, then pass priority until "cast fireball" is available
//! - `equip accorder;*;attack grizzly` - Equip, then pass until attack phase
//!
//! This allows flexible scripts that don't need to specify every single priority pass.
//!
//! ## Error Handling
//!
//! - **Normal mode**: Commands MUST match an available action or be an explicit pass ("pass", "p", "0").
//!   If a command doesn't match, the controller returns an error with available actions.
//! - **Wildcard mode**: Non-matching commands cause the controller to pass priority and wait for a match.
//!   No error is raised - the controller keeps waiting until the command becomes available.
//!
//! ## Blocking Syntax
//!
//! Comma-separated clauses: `BlackKnight blocks WhiteKnight, SerraAngel blocks RoyalAssassin`

use crate::core::{CardId, ManaCost, PlayerId, SpellAbility};
use crate::game::controller::{ChoiceResult, GameStateView, PlayerController};
use smallvec::SmallVec;

/// Controller that parses rich text commands
pub struct RichInputController {
    player_id: PlayerId,
    /// Script of text commands (consumed from front)
    commands: Vec<String>,
    /// Current index in the command queue
    current_index: usize,
    /// Whether we're in wildcard mode (waiting for a specific command to match)
    wildcard_mode: bool,
}

impl RichInputController {
    /// Create a new rich input controller
    ///
    /// # Arguments
    /// * `player_id` - The player ID this controller manages
    /// * `commands` - Vector of text commands to execute
    pub fn new(player_id: PlayerId, commands: Vec<String>) -> Self {
        RichInputController {
            player_id,
            commands,
            current_index: 0,
            wildcard_mode: false,
        }
    }

    /// Peek at the current command without consuming it
    fn peek_command(&self) -> Option<&str> {
        if self.current_index < self.commands.len() {
            Some(&self.commands[self.current_index])
        } else {
            None
        }
    }

    /// Check if the current command (at current_index) is a wildcard
    fn current_is_wildcard(&self) -> bool {
        if self.current_index < self.commands.len() {
            self.commands[self.current_index].trim() == "*"
        } else {
            false
        }
    }

    /// Get the next command from the script and advance the index
    /// Automatically skips wildcard separators and enters wildcard mode
    fn next_command(&mut self) -> Option<String> {
        if self.current_index < self.commands.len() {
            let cmd = self.commands[self.current_index].clone();
            self.current_index += 1;

            // Check if this was a wildcard separator
            if cmd.trim() == "*" {
                // Enter wildcard mode and get the next actual command
                self.wildcard_mode = true;
                return self.next_command();
            }

            // Check if current command (after advancing) is a wildcard - if so, enter wildcard mode
            if self.current_is_wildcard() {
                // Consume the wildcard
                self.current_index += 1;
                self.wildcard_mode = true;
            } else if !self.wildcard_mode {
                // Only reset to false if we weren't already in wildcard mode
                // (prevents recursive next_command calls from resetting the flag)
                self.wildcard_mode = false;
            }

            Some(cmd)
        } else {
            None
        }
    }

    /// Normalize a string for comparison
    ///
    /// - Converts to lowercase
    /// - Removes spaces and underscores
    /// - Removes non-alphanumeric characters (except for basic punctuation)
    pub fn normalize(s: &str) -> String {
        s.chars()
            .filter(|c| !c.is_whitespace() && *c != '_')
            .collect::<String>()
            .to_lowercase()
    }

    /// Check if a card name matches a pattern (prefix matching)
    pub fn card_matches(card_name: &str, pattern: &str) -> bool {
        let normalized_card = Self::normalize(card_name);
        let normalized_pattern = Self::normalize(pattern);
        normalized_card.starts_with(&normalized_pattern)
    }

    /// Parse a spell ability choice command
    ///
    /// Examples: "play swamp", "cast lightning bolt", "0", "pass"
    pub fn parse_spell_ability_choice(
        command: &str,
        view: &GameStateView,
        available: &[SpellAbility],
    ) -> Option<SpellAbility> {
        let cmd = command.trim().to_lowercase();

        // Handle numeric choice (matching menu display format from format_choice_menu)
        // [0] = Pass priority (return None)
        // [1] to [N] = available[0] to available[N-1] (menu indices shifted by 1)
        // Out of bounds values (idx > available.len()) also pass priority
        if let Ok(idx) = cmd.parse::<usize>() {
            if idx == 0 {
                return None; // [0] = Pass priority
            } else if idx <= available.len() {
                return Some(available[idx - 1].clone()); // [1] = available[0], [2] = available[1], etc.
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
                        if Self::card_matches(&card_name, card_pattern) {
                            return Some(ability.clone());
                        }
                    }
                }
            }
        } else if let Some(card_pattern) = cmd.strip_prefix("cast ") {
            // Find matching CastSpell ability
            for ability in available {
                if let SpellAbility::CastSpell { card_id } = ability {
                    if let Some(card_name) = view.card_name(*card_id) {
                        if Self::card_matches(&card_name, card_pattern) {
                            return Some(ability.clone());
                        }
                    }
                }
            }
        } else if let Some(card_pattern) = cmd.strip_prefix("equip ") {
            // Find matching ActivateAbility for Equipment
            // Format: "equip [card_name]" activates the Equip ability on that Equipment
            for ability in available {
                if let SpellAbility::ActivateAbility { card_id, .. } = ability {
                    if let Some(card_name) = view.card_name(*card_id) {
                        if Self::card_matches(&card_name, card_pattern) {
                            // TODO: Verify this is actually an Equip ability
                            // For now, just match by card name
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
                        if Self::card_matches(&card_name, pattern_part) {
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

        // Command not recognized or no match found - pass priority
        None
    }
}

impl PlayerController for RichInputController {
    fn player_id(&self) -> PlayerId {
        self.player_id
    }

    fn choose_spell_ability_to_play(
        &mut self,
        view: &GameStateView,
        available: &[SpellAbility],
    ) -> ChoiceResult<Option<SpellAbility>> {
        // Peek at the next command without consuming it
        if let Some(command_str) = self.peek_command() {
            let command = command_str.to_string();

            // Check if this is a wildcard separator - if so, consume it and enter wildcard mode
            if command.trim() == "*" {
                // Consume the wildcard and enter wildcard mode
                self.current_index += 1;
                self.wildcard_mode = true;
                // Now recursively call to process the next actual command
                return self.choose_spell_ability_to_play(view, available);
            }

            // Try to parse it
            let result = Self::parse_spell_ability_choice(&command, view, available);

            // Check if this is an explicit pass command
            let cmd_trimmed = command.trim().to_lowercase();
            let is_explicit_pass = cmd_trimmed == "pass" || cmd_trimmed == "p" || cmd_trimmed == "0";

            // In wildcard mode, only advance if we found a match or explicit pass
            if self.wildcard_mode {
                if result.is_some() || is_explicit_pass {
                    // Found a match or explicit pass! Exit wildcard mode first, then consume
                    self.wildcard_mode = false;
                    self.next_command();
                    ChoiceResult::Ok(result)
                } else {
                    // No match - pass priority and stay in wildcard mode
                    ChoiceResult::Ok(None)
                }
            } else {
                // Normal mode - command MUST match or error
                if result.is_some() || is_explicit_pass {
                    // Valid command or explicit pass - consume and execute
                    self.next_command();
                    ChoiceResult::Ok(result)
                } else {
                    // Command didn't match any available action - ERROR
                    self.next_command(); // Consume the bad command to avoid infinite loop
                    ChoiceResult::Error(format!(
                        "Command '{}' did not match any available action. Available actions: {:?}",
                        command,
                        available
                            .iter()
                            .filter_map(|a| match a {
                                SpellAbility::PlayLand { card_id } => {
                                    view.card_name(*card_id).map(|n| format!("play {}", n))
                                }
                                SpellAbility::CastSpell { card_id } => {
                                    view.card_name(*card_id).map(|n| format!("cast {}", n))
                                }
                                SpellAbility::ActivateAbility { card_id, .. } => {
                                    view.card_name(*card_id).map(|n| format!("activate {}", n))
                                }
                            })
                            .collect::<Vec<_>>()
                    ))
                }
            }
        } else {
            // No more commands - pass priority
            ChoiceResult::Ok(None)
        }
    }

    fn choose_targets(
        &mut self,
        _view: &GameStateView,
        _spell: CardId,
        valid_targets: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        if valid_targets.is_empty() {
            return ChoiceResult::Ok(SmallVec::new());
        }

        if valid_targets.len() == 1 {
            // Only one target - no choice needed
            let mut targets = SmallVec::new();
            targets.push(valid_targets[0]);
            return ChoiceResult::Ok(targets);
        }

        // For now, just take the first target
        // TODO: Implement rich syntax for target selection
        let mut targets = SmallVec::new();
        targets.push(valid_targets[0]);
        ChoiceResult::Ok(targets)
    }

    fn choose_mana_sources_to_pay(
        &mut self,
        _view: &GameStateView,
        cost: &ManaCost,
        available_sources: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        // Simple greedy approach: take first N sources
        let mut sources = SmallVec::new();
        let needed = cost.cmc() as usize;

        for &source_id in available_sources.iter().take(needed) {
            sources.push(source_id);
        }

        ChoiceResult::Ok(sources)
    }

    fn choose_attackers(
        &mut self,
        view: &GameStateView,
        available_creatures: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        if available_creatures.is_empty() {
            return ChoiceResult::Ok(SmallVec::new());
        }

        if let Some(command) = self.next_command() {
            let cmd = command.trim().to_lowercase();

            // Handle numeric choice (legacy format)
            if let Ok(num) = cmd.parse::<usize>() {
                let num_attackers = num.min(available_creatures.len());
                return ChoiceResult::Ok(available_creatures.iter().take(num_attackers).copied().collect());
            }

            // Parse "attack X" commands
            let mut attackers = SmallVec::new();
            for clause in command.split(';') {
                let clause = clause.trim().to_lowercase();
                if let Some(card_pattern) = clause.strip_prefix("attack ") {
                    for &creature_id in available_creatures {
                        if let Some(card_name) = view.card_name(creature_id) {
                            if Self::card_matches(&card_name, card_pattern) && !attackers.contains(&creature_id) {
                                attackers.push(creature_id);
                            }
                        }
                    }
                } else if clause == "done" {
                    break;
                }
            }

            ChoiceResult::Ok(attackers)
        } else {
            // No more commands - don't attack
            ChoiceResult::Ok(SmallVec::new())
        }
    }

    fn choose_blockers(
        &mut self,
        view: &GameStateView,
        available_blockers: &[CardId],
        attackers: &[CardId],
    ) -> ChoiceResult<SmallVec<[(CardId, CardId); 8]>> {
        if available_blockers.is_empty() || attackers.is_empty() {
            return ChoiceResult::Ok(SmallVec::new());
        }

        if let Some(command) = self.next_command() {
            let cmd = command.trim().to_lowercase();

            // Handle numeric choice (legacy format)
            if let Ok(num) = cmd.parse::<usize>() {
                let num_blockers = num.min(available_blockers.len());
                let mut blocks = SmallVec::new();
                for &blocker_id in available_blockers.iter().take(num_blockers) {
                    blocks.push((blocker_id, attackers[0]));
                }
                return ChoiceResult::Ok(blocks);
            }

            // Parse "X blocks Y" commands
            let mut blocks = SmallVec::new();
            for clause in command.split(';') {
                let clause = clause.trim().to_lowercase();
                if clause.contains(" blocks ") {
                    if let Some(blocks_pos) = clause.find(" blocks ") {
                        let blocker_pattern = &clause[..blocks_pos];
                        let attacker_pattern = &clause[blocks_pos + 8..];

                        // Find matching blocker
                        let mut blocker_id = None;
                        for &creature_id in available_blockers {
                            if let Some(card_name) = view.card_name(creature_id) {
                                if Self::card_matches(&card_name, blocker_pattern) {
                                    blocker_id = Some(creature_id);
                                    break;
                                }
                            }
                        }

                        // Find matching attacker
                        let mut attacker_id = None;
                        for &creature_id in attackers {
                            if let Some(card_name) = view.card_name(creature_id) {
                                if Self::card_matches(&card_name, attacker_pattern) {
                                    attacker_id = Some(creature_id);
                                    break;
                                }
                            }
                        }

                        if let (Some(blocker), Some(attacker)) = (blocker_id, attacker_id) {
                            blocks.push((blocker, attacker));
                        }
                    }
                } else if clause == "done" {
                    break;
                }
            }

            ChoiceResult::Ok(blocks)
        } else {
            // No more commands - don't block
            ChoiceResult::Ok(SmallVec::new())
        }
    }

    fn choose_damage_assignment_order(
        &mut self,
        _view: &GameStateView,
        _attacker: CardId,
        blockers: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        // Keep original order (no reordering via rich input yet)
        ChoiceResult::Ok(blockers.iter().copied().collect())
    }

    fn choose_cards_to_discard(
        &mut self,
        _view: &GameStateView,
        hand: &[CardId],
        count: usize,
    ) -> ChoiceResult<SmallVec<[CardId; 7]>> {
        // Simple: discard first N cards
        // TODO: Implement rich syntax for discard selection
        ChoiceResult::Ok(hand.iter().take(count).copied().collect())
    }

    fn on_priority_passed(&mut self, _view: &GameStateView) {
        // No action needed
    }

    fn on_game_end(&mut self, _view: &GameStateView, _won: bool) {
        // No action needed
    }

    fn get_controller_type(&self) -> crate::game::snapshot::ControllerType {
        crate::game::snapshot::ControllerType::Tui
    }

    fn get_snapshot_state(&self) -> Option<serde_json::Value> {
        // Serialize the controller state
        serde_json::to_value(self).ok()
    }

    fn has_more_choices(&self) -> bool {
        self.current_index < self.commands.len()
    }
}

// Implement serialization for snapshots
impl serde::Serialize for RichInputController {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("RichInputController", 4)?;
        state.serialize_field("player_id", &self.player_id)?;
        state.serialize_field("commands", &self.commands)?;
        state.serialize_field("current_index", &self.current_index)?;
        state.serialize_field("wildcard_mode", &self.wildcard_mode)?;
        state.end()
    }
}

impl<'de> serde::Deserialize<'de> for RichInputController {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        struct RichInputControllerData {
            player_id: PlayerId,
            commands: Vec<String>,
            current_index: usize,
            #[serde(default)]
            wildcard_mode: bool,
        }

        let data = RichInputControllerData::deserialize(deserializer)?;
        Ok(RichInputController {
            player_id: data.player_id,
            commands: data.commands,
            current_index: data.current_index,
            wildcard_mode: data.wildcard_mode,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::EntityId;
    use crate::game::GameState;

    #[test]
    fn test_normalize() {
        assert_eq!(RichInputController::normalize("Black Knight"), "blackknight");
        assert_eq!(RichInputController::normalize("Serra_Angel"), "serraangel");
        assert_eq!(RichInputController::normalize("Royal  Assassin"), "royalassassin");
    }

    #[test]
    fn test_card_matches() {
        assert!(RichInputController::card_matches("Black Knight", "black"));
        assert!(RichInputController::card_matches("Black Knight", "blackkn"));
        assert!(RichInputController::card_matches("Serra Angel", "serra"));
        assert!(!RichInputController::card_matches("Black Knight", "white"));
    }

    #[test]
    fn test_numeric_choice() {
        let player_id = EntityId::new(1);
        // Choice "1" selects first ability (available[0]), matching menu format where [0] = Pass
        let mut controller = RichInputController::new(player_id, vec!["1".to_string()]);
        let game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
        let view = GameStateView::new(&game, player_id);

        let abilities = vec![SpellAbility::PlayLand {
            card_id: EntityId::new(10),
        }];

        let choice = controller.choose_spell_ability_to_play(&view, &abilities);
        assert!(choice.unwrap().is_some());
    }

    #[test]
    fn test_numeric_choice_pass() {
        let player_id = EntityId::new(1);
        // Choice "0" means pass priority
        let mut controller = RichInputController::new(player_id, vec!["0".to_string()]);
        let game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
        let view = GameStateView::new(&game, player_id);

        let abilities = vec![SpellAbility::PlayLand {
            card_id: EntityId::new(10),
        }];

        let choice = controller.choose_spell_ability_to_play(&view, &abilities);
        assert!(choice.unwrap().is_none()); // Should pass (return None)
    }

    #[test]
    fn test_pass_command() {
        let player_id = EntityId::new(1);
        let mut controller = RichInputController::new(player_id, vec!["pass".to_string()]);
        let game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
        let view = GameStateView::new(&game, player_id);

        let choice = controller.choose_spell_ability_to_play(&view, &[]);
        assert!(choice.unwrap().is_none());
    }

    #[test]
    fn test_equip_command() {
        let player_id = EntityId::new(1);
        // Without actual card names in the test view, "equip accorder" won't match and should error
        let mut controller = RichInputController::new(player_id, vec!["equip accorder".to_string()]);
        let game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
        let view = GameStateView::new(&game, player_id);

        let equipment_id = EntityId::new(10);
        let abilities = vec![SpellAbility::ActivateAbility {
            card_id: equipment_id,
            ability_index: 0,
        }];

        let choice = controller.choose_spell_ability_to_play(&view, &abilities);
        // Without actual card data, this should error
        assert!(matches!(choice, ChoiceResult::Error(_)));
    }

    #[test]
    fn test_activate_command() {
        let player_id = EntityId::new(1);
        // Without actual card names, "activate forest" won't match and should error
        let mut controller = RichInputController::new(player_id, vec!["activate forest".to_string()]);
        let game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
        let view = GameStateView::new(&game, player_id);

        let land_id = EntityId::new(20);
        let abilities = vec![SpellAbility::ActivateAbility {
            card_id: land_id,
            ability_index: 0,
        }];

        let choice = controller.choose_spell_ability_to_play(&view, &abilities);
        // Without actual card data, this should error
        assert!(matches!(choice, ChoiceResult::Error(_)));
    }

    #[test]
    fn test_activate_command_with_index() {
        let player_id = EntityId::new(1);
        // Without actual card names, indexed activation should also error
        let mut controller = RichInputController::new(player_id, vec!["activate forest[2]".to_string()]);
        let game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
        let view = GameStateView::new(&game, player_id);

        let land_id = EntityId::new(20);
        let abilities = vec![
            SpellAbility::ActivateAbility {
                card_id: land_id,
                ability_index: 0,
            },
            SpellAbility::ActivateAbility {
                card_id: land_id,
                ability_index: 1,
            },
        ];

        let choice = controller.choose_spell_ability_to_play(&view, &abilities);
        // Without actual card data, this should error
        assert!(matches!(choice, ChoiceResult::Error(_)));
    }

    #[test]
    fn test_command_error_on_no_match() {
        let player_id = EntityId::new(1);
        // Command "cast fireball" should error when fireball is not available
        let mut controller = RichInputController::new(player_id, vec!["cast fireball".to_string()]);
        let game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
        let view = GameStateView::new(&game, player_id);

        let land_id = EntityId::new(10);
        let abilities = vec![SpellAbility::PlayLand { card_id: land_id }];

        let result = controller.choose_spell_ability_to_play(&view, &abilities);
        // Should return an error because "cast fireball" doesn't match "play land"
        assert!(matches!(result, ChoiceResult::Error(_)));
        if let ChoiceResult::Error(msg) = result {
            assert!(msg.contains("cast fireball"));
            assert!(msg.contains("did not match"));
        }
    }

    #[test]
    fn test_wildcard_mode_no_error_on_no_match() {
        let player_id = EntityId::new(1);
        // In wildcard mode, "cast fireball" should pass priority (not error) when not available
        let mut controller = RichInputController::new(
            player_id,
            vec!["pass".to_string(), "*".to_string(), "cast fireball".to_string()],
        );
        let game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
        let view = GameStateView::new(&game, player_id);

        let land_id = EntityId::new(10);
        let abilities = vec![SpellAbility::PlayLand { card_id: land_id }];

        // First choice: "pass" - should work
        let result1 = controller.choose_spell_ability_to_play(&view, &abilities);
        assert!(matches!(result1, ChoiceResult::Ok(_)));
        if let ChoiceResult::Ok(choice) = result1 {
            assert!(choice.is_none());
        }

        // Now in wildcard mode, waiting for "cast fireball"
        // Should pass priority without error
        let result2 = controller.choose_spell_ability_to_play(&view, &abilities);
        assert!(matches!(result2, ChoiceResult::Ok(_)));
        if let ChoiceResult::Ok(choice) = result2 {
            assert!(choice.is_none()); // Passes priority, waiting for fireball
        }
    }

    #[test]
    fn test_wildcard_at_beginning() {
        let player_id = EntityId::new(1);
        // Wildcard at the start means immediately enter wildcard mode
        let mut controller = RichInputController::new(player_id, vec!["*".to_string(), "cast fireball".to_string()]);
        let game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
        let view = GameStateView::new(&game, player_id);

        let land_id = EntityId::new(10);
        let abilities = vec![SpellAbility::PlayLand { card_id: land_id }];

        // First choice: wildcard mode active, "cast fireball" doesn't match
        // Should pass priority without error
        let result = controller.choose_spell_ability_to_play(&view, &abilities);
        assert!(matches!(result, ChoiceResult::Ok(_)));
        if let ChoiceResult::Ok(choice) = result {
            assert!(choice.is_none()); // Passes priority, waiting for fireball
        }

        // Controller should still be in wildcard mode
        assert!(controller.wildcard_mode);
    }

    #[test]
    fn test_first_command_must_match_strictly() {
        let player_id = EntityId::new(1);
        // First command is NOT a wildcard, so it must match strictly
        let mut controller = RichInputController::new(player_id, vec!["cast fireball".to_string()]);
        let game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
        let view = GameStateView::new(&game, player_id);

        let land_id = EntityId::new(10);
        let abilities = vec![SpellAbility::PlayLand { card_id: land_id }];

        // Should ERROR because "cast fireball" doesn't match and we're not in wildcard mode
        let result = controller.choose_spell_ability_to_play(&view, &abilities);
        assert!(matches!(result, ChoiceResult::Error(_)));
        if let ChoiceResult::Error(msg) = result {
            assert!(msg.contains("cast fireball"));
            assert!(msg.contains("did not match"));
        }
    }

    #[test]
    fn test_wildcard_at_beginning_then_match() {
        let player_id = EntityId::new(1);
        // Start with wildcard, then command matches available action
        let mut controller = RichInputController::new(
            player_id,
            vec!["*".to_string(), "0".to_string()], // 0 = pass
        );
        let game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
        let view = GameStateView::new(&game, player_id);

        let land_id = EntityId::new(10);
        let abilities = vec![SpellAbility::PlayLand { card_id: land_id }];

        // First choice: wildcard mode, "0" (pass) always matches
        // Should execute the pass and exit wildcard mode
        let result = controller.choose_spell_ability_to_play(&view, &abilities);
        assert!(matches!(result, ChoiceResult::Ok(_)));
        if let ChoiceResult::Ok(choice) = result {
            assert!(choice.is_none()); // Pass priority
        }

        // Should have exited wildcard mode after match
        assert!(!controller.wildcard_mode);

        // Should have consumed both wildcard and the "0" command
        assert_eq!(controller.current_index, 2);
    }
}
