//! Zero controller implementing the new PlayerController interface
//!
//! This controller uses simple deterministic heuristics:
//! - Plays the first land available
//! - Casts the first spell available
//! - Chooses first targets
//! - Taps first mana sources
//! - Attacks with all creatures
//! - Blocks each attacker with one blocker
//! - Discards the first N cards from hand

use crate::core::{CardId, ManaCost, PlayerId, SpellAbility};
use crate::game::controller::GameStateView;
use crate::game::controller::PlayerController;
use smallvec::SmallVec;

/// A controller that uses simple "first choice" heuristics
///
/// This is useful for:
/// - Automated testing
/// - Running games without interaction
/// - Benchmarking the game engine
/// - Ensuring games can complete deterministically
pub struct ZeroController {
    player_id: PlayerId,
}

impl ZeroController {
    /// Create a new zero controller
    pub fn new(player_id: PlayerId) -> Self {
        ZeroController { player_id }
    }
}

impl PlayerController for ZeroController {
    fn player_id(&self) -> PlayerId {
        self.player_id
    }

    fn choose_spell_ability_to_play(
        &mut self,
        _view: &GameStateView,
        available: &[SpellAbility],
    ) -> Option<SpellAbility> {
        // Play the first available ability
        available.first().cloned()
    }

    fn choose_targets(
        &mut self,
        _view: &GameStateView,
        _spell: CardId,
        valid_targets: &[CardId],
    ) -> SmallVec<[CardId; 4]> {
        // Choose the first valid target if any
        if let Some(&first_target) = valid_targets.first() {
            let mut targets = SmallVec::new();
            targets.push(first_target);
            targets
        } else {
            SmallVec::new()
        }
    }

    fn choose_mana_sources_to_pay(
        &mut self,
        _view: &GameStateView,
        cost: &ManaCost,
        available_sources: &[CardId],
    ) -> SmallVec<[CardId; 8]> {
        // Tap the first N sources needed to pay the cost
        let needed = cost.cmc() as usize;
        available_sources.iter().take(needed).copied().collect()
    }

    fn choose_attackers(&mut self, _view: &GameStateView, available_creatures: &[CardId]) -> SmallVec<[CardId; 8]> {
        // Attack with all available creatures
        available_creatures.iter().copied().collect()
    }

    fn choose_blockers(
        &mut self,
        _view: &GameStateView,
        available_blockers: &[CardId],
        attackers: &[CardId],
    ) -> SmallVec<[(CardId, CardId); 8]> {
        // Block each attacker with one blocker (if available)
        let mut blocks = SmallVec::new();

        for (i, &attacker_id) in attackers.iter().enumerate() {
            if let Some(&blocker_id) = available_blockers.get(i) {
                blocks.push((blocker_id, attacker_id));
            } else {
                // No more blockers available
                break;
            }
        }

        blocks
    }

    fn choose_damage_assignment_order(
        &mut self,
        _view: &GameStateView,
        _attacker: CardId,
        blockers: &[CardId],
    ) -> SmallVec<[CardId; 4]> {
        // Keep blockers in the order they were provided
        blockers.iter().copied().collect()
    }

    fn choose_cards_to_discard(
        &mut self,
        _view: &GameStateView,
        hand: &[CardId],
        count: usize,
    ) -> SmallVec<[CardId; 7]> {
        // Discard the first N cards from hand
        hand.iter().take(count.min(hand.len())).copied().collect()
    }

    fn on_priority_passed(&mut self, _view: &GameStateView) {
        // Zero controller doesn't need to log
    }

    fn on_game_end(&mut self, _view: &GameStateView, _won: bool) {
        // Zero controller doesn't need to log
    }

    fn get_controller_type(&self) -> crate::game::snapshot::ControllerType {
        crate::game::snapshot::ControllerType::Zero
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::EntityId;
    use crate::game::GameState;

    #[test]
    fn test_zero_controller_creation() {
        let player_id = EntityId::new(1);
        let controller = ZeroController::new(player_id);
        assert_eq!(controller.player_id(), player_id);
    }

    #[test]
    fn test_choose_spell_ability_empty() {
        let player_id = EntityId::new(1);
        let mut controller = ZeroController::new(player_id);
        let game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
        let view = GameStateView::new(&game, player_id);

        // With no available abilities, should return None
        let choice = controller.choose_spell_ability_to_play(&view, &[]);
        assert_eq!(choice, None);
    }

    #[test]
    fn test_choose_spell_ability_land() {
        let player_id = EntityId::new(1);
        let mut controller = ZeroController::new(player_id);
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

        let chosen = controller.choose_spell_ability_to_play(&view, &abilities);

        // Should choose the first ability (PlayLand)
        assert_eq!(
            chosen,
            Some(SpellAbility::PlayLand {
                card_id: EntityId::new(10)
            })
        );
    }

    #[test]
    fn test_choose_targets() {
        let player_id = EntityId::new(1);
        let mut controller = ZeroController::new(player_id);
        let game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
        let view = GameStateView::new(&game, player_id);

        let spell_id = EntityId::new(100);
        let valid_targets = vec![EntityId::new(20), EntityId::new(21), EntityId::new(22)];
        let targets = controller.choose_targets(&view, spell_id, &valid_targets);

        // Should choose the first target
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0], EntityId::new(20));
    }

    #[test]
    fn test_choose_targets_empty() {
        let player_id = EntityId::new(1);
        let mut controller = ZeroController::new(player_id);
        let game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
        let view = GameStateView::new(&game, player_id);

        let spell_id = EntityId::new(100);
        let targets = controller.choose_targets(&view, spell_id, &[]);

        // No targets available
        assert_eq!(targets.len(), 0);
    }

    #[test]
    fn test_choose_mana_sources() {
        let player_id = EntityId::new(1);
        let mut controller = ZeroController::new(player_id);
        let game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
        let view = GameStateView::new(&game, player_id);

        let cost = ManaCost::from_string("2RR"); // CMC = 4
        let available = vec![
            EntityId::new(10),
            EntityId::new(11),
            EntityId::new(12),
            EntityId::new(13),
            EntityId::new(14),
        ];

        let sources = controller.choose_mana_sources_to_pay(&view, &cost, &available);

        // Should choose first 4 sources (equal to CMC)
        assert_eq!(sources.len(), 4);
        assert_eq!(sources[0], EntityId::new(10));
        assert_eq!(sources[1], EntityId::new(11));
        assert_eq!(sources[2], EntityId::new(12));
        assert_eq!(sources[3], EntityId::new(13));
    }

    #[test]
    fn test_choose_attackers() {
        let player_id = EntityId::new(1);
        let mut controller = ZeroController::new(player_id);
        let game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
        let view = GameStateView::new(&game, player_id);

        let creatures = vec![EntityId::new(30), EntityId::new(31), EntityId::new(32)];
        let attackers = controller.choose_attackers(&view, &creatures);

        // Should attack with all creatures
        assert_eq!(attackers.len(), 3);
        assert_eq!(attackers[0], EntityId::new(30));
        assert_eq!(attackers[1], EntityId::new(31));
        assert_eq!(attackers[2], EntityId::new(32));
    }

    #[test]
    fn test_choose_blockers() {
        let player_id = EntityId::new(1);
        let mut controller = ZeroController::new(player_id);
        let game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
        let view = GameStateView::new(&game, player_id);

        let blockers = vec![EntityId::new(40), EntityId::new(41)];
        let attackers = vec![EntityId::new(50), EntityId::new(51), EntityId::new(52)];
        let blocks = controller.choose_blockers(&view, &blockers, &attackers);

        // Should block first 2 attackers (limited by blocker count)
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0], (EntityId::new(40), EntityId::new(50)));
        assert_eq!(blocks[1], (EntityId::new(41), EntityId::new(51)));
    }

    #[test]
    fn test_choose_cards_to_discard() {
        let player_id = EntityId::new(1);
        let mut controller = ZeroController::new(player_id);
        let game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
        let view = GameStateView::new(&game, player_id);

        let hand = vec![
            EntityId::new(60),
            EntityId::new(61),
            EntityId::new(62),
            EntityId::new(63),
        ];

        let discards = controller.choose_cards_to_discard(&view, &hand, 2);

        // Should discard first 2 cards
        assert_eq!(discards.len(), 2);
        assert_eq!(discards[0], EntityId::new(60));
        assert_eq!(discards[1], EntityId::new(61));
    }
}
