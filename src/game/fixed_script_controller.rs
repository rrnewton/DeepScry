//! Fixed script controller for deterministic testing
//!
//! This controller follows a predetermined sequence of choices, making it ideal
//! for testing specific game scenarios. Once the script is exhausted, it defaults
//! to choosing the first option (index 0).

use crate::core::{CardId, ManaCost, PlayerId, SpellAbility};
use crate::game::controller::GameStateView;
use crate::game::controller::PlayerController;
use smallvec::SmallVec;

/// A controller that follows a fixed script of choices for testing
///
/// The script is a sequence of indices that will be used to select from
/// available options. When the script is exhausted, the controller defaults
/// to always choosing index 0.
///
/// This controller is serializable, allowing its state (including current position)
/// to be saved in snapshots and restored on resume.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FixedScriptController {
    player_id: PlayerId,
    /// The predetermined sequence of choice indices
    script: Vec<usize>,
    /// Current position in the script
    pub current_index: usize,
}

impl FixedScriptController {
    /// Create a new scripted controller with a predetermined sequence of choices
    ///
    /// # Arguments
    /// * `player_id` - The player this controller represents
    /// * `script` - A sequence of indices for making choices (e.g., `vec![1, 1, 3]`)
    ///
    /// # Example
    /// ```
    /// use mtg_forge_rs::game::FixedScriptController;
    /// use mtg_forge_rs::core::PlayerId;
    ///
    /// let controller = FixedScriptController::new(
    ///     PlayerId::new(0),
    ///     vec![0, 1, 2, 0]  // Will choose options 0, 1, 2, 0, then default to 0
    /// );
    /// ```
    pub fn new(player_id: PlayerId, script: Vec<usize>) -> Self {
        FixedScriptController {
            player_id,
            script,
            current_index: 0,
        }
    }

    /// Get the next choice index from the script
    ///
    /// Returns the next index from the script, or 0 if the script is exhausted.
    /// Advances the internal position counter.
    fn next_choice(&mut self) -> usize {
        if self.current_index < self.script.len() {
            let choice = self.script[self.current_index];
            self.current_index += 1;
            choice
        } else {
            // Script exhausted, default to 0
            0
        }
    }
}

impl PlayerController for FixedScriptController {
    fn player_id(&self) -> PlayerId {
        self.player_id
    }

    fn choose_spell_ability_to_play(
        &mut self,
        view: &GameStateView,
        available: &[SpellAbility],
    ) -> Option<SpellAbility> {
        let choice_index = self.next_choice();

        // INVARIANT: Choice indices match menu display
        // Index 0 = Pass priority
        // Index 1 to N = available[index - 1] (actions shifted by 1)
        // Out of bounds values (index > N) also pass priority

        if choice_index == 0 {
            // Choice 0 is always pass
            view.logger().controller_choice("SCRIPT", "chose [0] Pass priority");
            return None;
        }

        // Convert displayed index to array index (subtract 1)
        let action_index = choice_index - 1;

        if action_index >= available.len() {
            // Out of bounds, pass priority
            view.logger().controller_choice(
                "SCRIPT",
                &format!(
                    "chose {} (out of bounds, passing priority) from choices 0-{}",
                    choice_index,
                    available.len()
                ),
            );
            return None;
        }

        view.logger().controller_choice(
            "SCRIPT",
            &format!("chose [{}] from choices 0-{}", choice_index, available.len()),
        );
        Some(available[action_index].clone())
    }

    fn choose_targets(
        &mut self,
        view: &GameStateView,
        _spell: CardId,
        valid_targets: &[CardId],
    ) -> SmallVec<[CardId; 4]> {
        if valid_targets.is_empty() {
            view.logger()
                .controller_choice("SCRIPT", "chose no targets (none available)");
            return SmallVec::new();
        }

        if valid_targets.len() == 1 {
            // Only one target available - no choice to make, don't log or consume script
            let mut targets = SmallVec::new();
            targets.push(valid_targets[0]);
            return targets;
        }

        // Multiple targets - use script
        let choice_index = self.next_choice();
        let clamped_index = choice_index.min(valid_targets.len() - 1);

        if choice_index != clamped_index {
            view.logger().controller_choice(
                "SCRIPT",
                &format!(
                    "chose target {} (clamped from {}) out of choices 0-{}",
                    clamped_index,
                    choice_index,
                    valid_targets.len() - 1
                ),
            );
        } else {
            view.logger().controller_choice(
                "SCRIPT",
                &format!(
                    "chose target {} out of choices 0-{}",
                    choice_index,
                    valid_targets.len() - 1
                ),
            );
        }

        let mut targets = SmallVec::new();
        targets.push(valid_targets[clamped_index]);
        targets
    }

    fn choose_mana_sources_to_pay(
        &mut self,
        view: &GameStateView,
        cost: &ManaCost,
        available_sources: &[CardId],
    ) -> SmallVec<[CardId; 8]> {
        // Simple greedy approach: take sources in order until we have enough
        // Script controller doesn't use randomness, just takes first N sources
        let mut sources = SmallVec::new();
        let needed = cost.cmc() as usize;

        if available_sources.len() > needed {
            view.logger().controller_choice(
                "SCRIPT",
                &format!(
                    "chose first {} mana sources from {} available sources",
                    needed.min(available_sources.len()),
                    available_sources.len()
                ),
            );
        }

        for &source_id in available_sources.iter().take(needed) {
            sources.push(source_id);
        }

        sources
    }

    fn choose_attackers(&mut self, view: &GameStateView, available_creatures: &[CardId]) -> SmallVec<[CardId; 8]> {
        if available_creatures.is_empty() {
            view.logger()
                .controller_choice("SCRIPT", "chose no attackers (none available)");
            return SmallVec::new();
        }

        // Use script to decide how many creatures to attack with
        let choice_index = self.next_choice();
        let num_attackers = choice_index.min(available_creatures.len());

        view.logger().controller_choice(
            "SCRIPT",
            &format!(
                "chose {} attackers from {} available creatures",
                num_attackers,
                available_creatures.len()
            ),
        );

        // Attack with the first N creatures
        let mut attackers = SmallVec::new();
        for &creature_id in available_creatures.iter().take(num_attackers) {
            attackers.push(creature_id);
        }

        attackers
    }

    fn choose_blockers(
        &mut self,
        view: &GameStateView,
        available_blockers: &[CardId],
        attackers: &[CardId],
    ) -> SmallVec<[(CardId, CardId); 8]> {
        if attackers.is_empty() || available_blockers.is_empty() {
            view.logger()
                .controller_choice("SCRIPT", "chose no blockers (none available or no attackers)");
            return SmallVec::new();
        }

        // Use script to decide how many blockers to use
        let choice_index = self.next_choice();
        let num_blockers = choice_index.min(available_blockers.len());

        view.logger().controller_choice(
            "SCRIPT",
            &format!(
                "chose {} blockers from {} available blockers",
                num_blockers,
                available_blockers.len()
            ),
        );

        // Block with the first N blockers, each blocking the first attacker
        let mut blocks = SmallVec::new();
        for &blocker_id in available_blockers.iter().take(num_blockers) {
            blocks.push((blocker_id, attackers[0]));
        }

        blocks
    }

    fn choose_damage_assignment_order(
        &mut self,
        view: &GameStateView,
        _attacker: CardId,
        blockers: &[CardId],
    ) -> SmallVec<[CardId; 4]> {
        // Just return blockers in the order they were provided
        // Script controller doesn't reorder
        if blockers.len() >= 2 {
            view.logger().controller_choice(
                "SCRIPT",
                &format!(
                    "chose damage assignment order (kept original order of {} blockers)",
                    blockers.len()
                ),
            );
        }

        blockers.iter().copied().collect()
    }

    fn choose_cards_to_discard(
        &mut self,
        view: &GameStateView,
        hand: &[CardId],
        count: usize,
    ) -> SmallVec<[CardId; 7]> {
        // Discard first N cards from hand
        let num_discarding = count.min(hand.len());

        if hand.len() > count {
            view.logger().controller_choice(
                "SCRIPT",
                &format!(
                    "chose first {} cards to discard from {} cards in hand",
                    num_discarding,
                    hand.len()
                ),
            );
        }

        hand.iter().take(num_discarding).copied().collect()
    }

    fn on_priority_passed(&mut self, _view: &GameStateView) {
        // Script controller doesn't need to react to priority passes
    }

    fn on_game_end(&mut self, _view: &GameStateView, _won: bool) {
        // Script controller doesn't react to game end
    }

    fn get_snapshot_state(&self) -> Option<serde_json::Value> {
        // Serialize the entire controller state (script + current_index)
        serde_json::to_value(self).ok()
    }

    fn has_more_choices(&self) -> bool {
        self.current_index < self.script.len()
    }

    fn get_controller_type(&self) -> crate::game::snapshot::ControllerType {
        crate::game::snapshot::ControllerType::Fixed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::EntityId;
    use crate::game::GameState;

    #[test]
    fn test_script_controller_creation() {
        let player_id = EntityId::new(1);
        let controller = FixedScriptController::new(player_id, vec![0, 1, 2]);
        assert_eq!(controller.player_id(), player_id);
    }

    #[test]
    fn test_next_choice_sequence() {
        let player_id = EntityId::new(1);
        let mut controller = FixedScriptController::new(player_id, vec![1, 2, 3]);

        // Should return script values in order
        assert_eq!(controller.next_choice(), 1);
        assert_eq!(controller.next_choice(), 2);
        assert_eq!(controller.next_choice(), 3);

        // After script is exhausted, should return 0
        assert_eq!(controller.next_choice(), 0);
        assert_eq!(controller.next_choice(), 0);
    }

    #[test]
    fn test_choose_spell_ability() {
        let player_id = EntityId::new(1);
        let mut controller = FixedScriptController::new(player_id, vec![1, 2]);
        let game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
        let view = GameStateView::new(&game, player_id);

        let abilities = vec![
            SpellAbility::PlayLand {
                card_id: EntityId::new(10),
            },
            SpellAbility::CastSpell {
                card_id: EntityId::new(11),
            },
        ];

        // INVARIANT: Index 0 = Pass, Index 1+ = actions (shifted by 1)
        // First choice: index 1 → abilities[0] (first ability)
        let choice1 = controller.choose_spell_ability_to_play(&view, &abilities);
        assert_eq!(choice1, Some(abilities[0].clone()));

        // Second choice: index 2 → abilities[1] (second ability)
        let choice2 = controller.choose_spell_ability_to_play(&view, &abilities);
        assert_eq!(choice2, Some(abilities[1].clone()));
    }

    #[test]
    fn test_choose_spell_ability_pass() {
        let player_id = EntityId::new(1);
        // Choice index 0 = pass, index 3+ out of bounds for 2 abilities also passes
        let mut controller = FixedScriptController::new(player_id, vec![0, 99]);
        let game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
        let view = GameStateView::new(&game, player_id);

        let abilities = vec![
            SpellAbility::PlayLand {
                card_id: EntityId::new(10),
            },
            SpellAbility::CastSpell {
                card_id: EntityId::new(11),
            },
        ];

        // Index 0 = pass priority
        let choice1 = controller.choose_spell_ability_to_play(&view, &abilities);
        assert_eq!(choice1, None);

        // Out of bounds choice (99) should also pass
        let choice2 = controller.choose_spell_ability_to_play(&view, &abilities);
        assert_eq!(choice2, None);
    }

    #[test]
    fn test_choose_targets() {
        let player_id = EntityId::new(1);
        let mut controller = FixedScriptController::new(player_id, vec![2, 0]);
        let game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
        let view = GameStateView::new(&game, player_id);

        let spell_id = EntityId::new(100);
        let valid_targets = vec![EntityId::new(20), EntityId::new(21), EntityId::new(22)];

        // First choice: index 2 (third target)
        let targets1 = controller.choose_targets(&view, spell_id, &valid_targets);
        assert_eq!(targets1.len(), 1);
        assert_eq!(targets1[0], valid_targets[2]);

        // Second choice: index 0 (first target)
        let targets2 = controller.choose_targets(&view, spell_id, &valid_targets);
        assert_eq!(targets2.len(), 1);
        assert_eq!(targets2[0], valid_targets[0]);
    }

    #[test]
    fn test_choose_attackers() {
        let player_id = EntityId::new(1);
        // Choose 2 attackers, then 0 attackers
        let mut controller = FixedScriptController::new(player_id, vec![2, 0]);
        let game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
        let view = GameStateView::new(&game, player_id);

        let creatures = vec![EntityId::new(20), EntityId::new(21), EntityId::new(22)];

        // First choice: 2 attackers
        let attackers1 = controller.choose_attackers(&view, &creatures);
        assert_eq!(attackers1.len(), 2);
        assert_eq!(attackers1[0], creatures[0]);
        assert_eq!(attackers1[1], creatures[1]);

        // Second choice: 0 attackers
        let attackers2 = controller.choose_attackers(&view, &creatures);
        assert_eq!(attackers2.len(), 0);
    }

    #[test]
    fn test_exhausted_script_defaults_to_zero() {
        let player_id = EntityId::new(1);
        let mut controller = FixedScriptController::new(player_id, vec![1]); // Only one choice
        let game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
        let view = GameStateView::new(&game, player_id);

        let abilities = vec![
            SpellAbility::PlayLand {
                card_id: EntityId::new(10),
            },
            SpellAbility::CastSpell {
                card_id: EntityId::new(11),
            },
        ];

        // INVARIANT: Index 0 = Pass, Index 1+ = actions (shifted by 1)
        // First choice: index 1 → abilities[0] (first ability)
        let choice1 = controller.choose_spell_ability_to_play(&view, &abilities);
        assert_eq!(choice1, Some(abilities[0].clone()));

        // Script exhausted, should default to index 0 → pass priority
        let choice2 = controller.choose_spell_ability_to_play(&view, &abilities);
        assert_eq!(choice2, None);

        // Should keep passing
        let choice3 = controller.choose_spell_ability_to_play(&view, &abilities);
        assert_eq!(choice3, None);
    }
}
