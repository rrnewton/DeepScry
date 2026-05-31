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
use crate::game::command_parsing::{card_matches, is_explicit_pass, parse_spell_ability_choice};
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
        // If there are no available abilities, just pass - no point trying to match commands
        // This can happen in network mode when server asks for priority with 0 abilities
        if available.is_empty() {
            return ChoiceResult::Ok(None);
        }

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
            let result = parse_spell_ability_choice(&command, view, available);

            // Check if this is an explicit pass command
            let explicit_pass = is_explicit_pass(&command);

            // Check if this is a combat command (attack/block) - these should be deferred
            // to choose_attackers/choose_blockers, so we pass priority here
            let cmd_lower = command.trim().to_lowercase();
            let is_combat_command = cmd_lower.starts_with("attack ") || cmd_lower.starts_with("block ");

            // In wildcard mode, only advance if we found a match or explicit pass
            if self.wildcard_mode {
                if result.is_some() || explicit_pass {
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
                if result.is_some() || explicit_pass {
                    // Valid command or explicit pass - consume and execute
                    self.next_command();
                    ChoiceResult::Ok(result)
                } else if is_combat_command {
                    // Combat command (attack/block) during priority - don't consume, pass priority
                    // The command will be handled later by choose_attackers/choose_blockers
                    ChoiceResult::Ok(None)
                } else {
                    // Command didn't match any available action - ERROR
                    self.next_command(); // Consume the bad command to avoid infinite loop
                    ChoiceResult::Error(format!(
                        "Command '{}' did not match any available action. Available actions: {:?}",
                        command,
                        available
                            .iter()
                            .map(|a| crate::game::controller::format_spell_ability_choice(view, a))
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
        min_targets: usize,
        max_targets: usize,
    ) -> ChoiceResult<SmallVec<[CardId; 4]>> {
        if valid_targets.is_empty() {
            return ChoiceResult::Ok(SmallVec::new());
        }

        // Best-effort: rich text target syntax for variable-count spells is a
        // deferred follow-up (mtg-tyvcn). For now take the first `min_targets`
        // (at least 1, capped at max) valid targets deterministically.
        let lo = min_targets.max(1);
        let count = lo.min(max_targets.max(1)).min(valid_targets.len());
        ChoiceResult::Ok(valid_targets.iter().take(count).copied().collect())
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
                            if card_matches(&card_name, card_pattern) && !attackers.contains(&creature_id) {
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
                                if card_matches(&card_name, blocker_pattern) {
                                    blocker_id = Some(creature_id);
                                    break;
                                }
                            }
                        }

                        // Find matching attacker
                        let mut attacker_id = None;
                        for &creature_id in attackers {
                            if let Some(card_name) = view.card_name(creature_id) {
                                if card_matches(&card_name, attacker_pattern) {
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

    fn choose_from_library(
        &mut self,
        view: &GameStateView,
        valid_cards: &[&crate::loader::CardDefinition],
    ) -> ChoiceResult<Option<usize>> {
        // RichInputController: Auto-select first valid card
        // TODO: Implement rich syntax for library search selection
        if valid_cards.is_empty() {
            view.logger()
                .controller_choice("RICHINPUT", "Library search: fail to find (no valid cards)");
            return ChoiceResult::Ok(None);
        }

        let card_def = valid_cards[0];
        view.logger().controller_choice(
            "RICHINPUT",
            &format!("Library search: found {} (auto-selected first)", card_def.name),
        );

        ChoiceResult::Ok(Some(0))
    }

    fn choose_permanents_to_sacrifice(
        &mut self,
        view: &GameStateView,
        valid_permanents: &[CardId],
        count: usize,
        card_type_description: &str,
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        // RichInputController: Auto-select first N permanents
        // TODO: Implement rich syntax for sacrifice selection
        let num_to_sacrifice = count.min(valid_permanents.len());
        let to_sacrifice: SmallVec<[CardId; 8]> = valid_permanents.iter().take(num_to_sacrifice).copied().collect();

        if !to_sacrifice.is_empty() {
            view.logger().controller_choice(
                "RICHINPUT",
                &format!(
                    "Sacrifice {}: auto-selected first {} permanents",
                    card_type_description, num_to_sacrifice
                ),
            );
        }

        ChoiceResult::Ok(to_sacrifice)
    }

    fn choose_permanents_to_not_untap(
        &mut self,
        _view: &GameStateView,
        _may_not_untap_permanents: &[CardId],
    ) -> ChoiceResult<SmallVec<[CardId; 8]>> {
        // Rich input controller always untaps everything (returns empty list = untap all)
        // TODO: Could add command syntax for controlling untap decisions
        ChoiceResult::Ok(SmallVec::new())
    }

    fn choose_modes(
        &mut self,
        _view: &GameStateView,
        _spell_id: CardId,
        mode_descriptions: &[String],
        mode_count: usize,
        _min_modes: usize,
        _can_repeat: bool,
    ) -> ChoiceResult<SmallVec<[usize; 4]>> {
        // Rich input controller: use next command as mode index, or default to first N modes
        // TODO: Could add command syntax like "mode 0 1" for selecting specific modes
        if let Some(command_str) = self.peek_command() {
            let command = command_str.to_string();
            self.current_index += 1;

            // Try to parse as mode index
            if let Ok(idx) = command.trim().parse::<usize>() {
                if idx < mode_descriptions.len() {
                    return ChoiceResult::Ok(smallvec::smallvec![idx]);
                }
            }
        }

        // Default: choose first N modes
        ChoiceResult::Ok((0..mode_count.min(mode_descriptions.len())).collect())
    }

    fn on_priority_passed(&mut self, _view: &GameStateView) {
        // No action needed
    }

    fn on_game_end(&mut self, _view: &GameStateView, _won: bool) {
        // No action needed
    }

    /// Choose from string options (used by network client)
    ///
    /// This implements the same command parsing logic as choose_spell_ability_to_play,
    /// but works with the simplified string-based options used by the network protocol.
    fn choose_from_options(&mut self, options: &[String]) -> usize {
        // Check if we have commands left
        if let Some(command_str) = self.peek_command() {
            let command = command_str.to_string();

            // Check if this is a wildcard separator
            if command.trim() == "*" {
                self.current_index += 1;
                self.wildcard_mode = true;
                return self.choose_from_options(options);
            }

            // Try to parse numeric choice first
            if let Ok(idx) = command.trim().parse::<usize>() {
                self.next_command();
                self.wildcard_mode = false;
                if idx < options.len() {
                    return idx;
                } else {
                    // Invalid index, default to 0
                    return 0;
                }
            }

            // Try to match command text against options
            // Check for pass commands (match "pass priority" option)
            let cmd_lower = command.trim().to_lowercase();
            if cmd_lower == "pass" || cmd_lower == "p" || cmd_lower == "0" {
                self.next_command();
                self.wildcard_mode = false;
                // Find "pass priority" option or return 0
                for (i, opt) in options.iter().enumerate() {
                    if opt.to_lowercase().contains("pass") {
                        return i;
                    }
                }
                return 0;
            }

            // Try to match by option text (e.g., "play island" matching "Play land: Island")
            for (i, opt) in options.iter().enumerate() {
                let opt_lower = opt.to_lowercase();
                // Check if command appears to match this option
                // Handle "play X" -> "Play land: X"
                if let Some(card_name) = cmd_lower.strip_prefix("play ") {
                    if opt_lower.contains("play land") && opt_lower.contains(card_name.trim()) {
                        self.next_command();
                        self.wildcard_mode = false;
                        return i;
                    }
                }
                // Handle "cast X" -> "Cast spell: X" or just "Cast: X"
                if let Some(card_name) = cmd_lower.strip_prefix("cast ") {
                    if (opt_lower.contains("cast spell") || opt_lower.contains("cast:"))
                        && opt_lower.contains(card_name.trim())
                    {
                        self.next_command();
                        self.wildcard_mode = false;
                        return i;
                    }
                }
            }

            // No match found
            if self.wildcard_mode {
                // In wildcard mode, pass priority and keep waiting
                return 0;
            } else {
                // Normal mode - consume command and default to 0
                log::warn!(
                    "Command '{}' did not match any option, defaulting to 0. Options: {:?}",
                    command,
                    options
                );
                self.next_command();
                return 0;
            }
        }

        // No more commands - default to 0 (usually "pass priority")
        0
    }

    fn get_controller_type(&self) -> crate::game::snapshot::ControllerType {
        crate::game::snapshot::ControllerType::Tui
    }

    fn wants_context(&self) -> bool {
        true
    }

    fn get_snapshot_state(&self) -> Option<serde_json::Value> {
        // RichInputController state is not needed for snapshot restoration
        // (choices are replayed from the choices files, not from controller state).
        // Return None to avoid deserialization errors with ControllerState enum.
        None
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
    use crate::game::command_parsing::normalize;
    use crate::game::GameState;

    #[test]
    fn test_normalize() {
        // Tests now use shared normalize function from command_parsing module
        assert_eq!(normalize("Black Knight"), "blackknight");
        assert_eq!(normalize("Serra_Angel"), "serraangel");
        assert_eq!(normalize("Royal  Assassin"), "royalassassin");
    }

    #[test]
    fn test_card_matches() {
        // Tests now use shared card_matches function from command_parsing module
        assert!(card_matches("Black Knight", "black"));
        assert!(card_matches("Black Knight", "blackkn"));
        assert!(card_matches("Serra Angel", "serra"));
        assert!(!card_matches("Black Knight", "white"));
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
