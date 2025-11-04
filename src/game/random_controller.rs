//! Random AI controller implementing the new PlayerController interface
//!
//! This implementation uses specific callback methods instead of
//! generic action choices. Makes random choices from available options.

use crate::core::{CardId, ManaCost, PlayerId, SpellAbility};
use crate::game::controller::GameStateView;
use crate::game::controller::PlayerController;
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};
use smallvec::SmallVec;

/// A controller that makes random choices using its own independent RNG
///
/// This controller owns its own RNG, seeded independently from the game engine.
/// This separation ensures that controller decisions don't affect game engine
/// randomness (like shuffling), enabling proper deterministic replay.
///
/// Uses Xoshiro256PlusPlus which has built-in serde support without u128 fields.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RandomController {
    player_id: PlayerId,
    /// Independent RNG for this controller's decisions
    ///
    /// This RNG is seeded separately from the game engine's RNG to ensure
    /// complete independence between controller choices and game mechanics.
    ///
    /// We use Xoshiro256PlusPlus instead of StdRng because it has proper serde1 support
    /// that preserves the full RNG state with serde_json (no u128 fields).
    rng: rand_xoshiro::Xoshiro256PlusPlus,
}

impl RandomController {
    /// Create a random controller with a specific seed
    ///
    /// Use this for deterministic controller behavior (for testing, replay, or normal gameplay).
    /// The seed should be derived from a master seed with player-specific salt.
    ///
    /// IMPORTANT: This is the ONLY way to create a RandomController. There is no `new()`
    /// method because we want to enforce explicit seed management throughout the codebase.
    /// If you need non-deterministic behavior, use `--seed=from_entropy` in the CLI,
    /// which is the single point where system entropy is accessed.
    pub fn with_seed(player_id: PlayerId, seed: u64) -> Self {
        RandomController {
            player_id,
            rng: rand_xoshiro::Xoshiro256PlusPlus::seed_from_u64(seed),
        }
    }

    /// Get a debug representation of the RNG state
    ///
    /// This is useful for debugging non-determinism issues. The RNG state
    /// consists of 4 u64 values that fully determine future random numbers.
    pub fn debug_rng_state(&self) -> String {
        match serde_json::to_string(&self.rng) {
            Ok(json) => json,
            Err(_) => "<serialization failed>".to_string(),
        }
    }
}

impl PlayerController for RandomController {
    fn player_id(&self) -> PlayerId {
        self.player_id
    }

    fn choose_spell_ability_to_play(
        &mut self,
        view: &GameStateView,
        available: &[SpellAbility],
    ) -> Option<SpellAbility> {
        // INVARIANT: Choice 0 = pass priority (always available)
        //            Choice N (N > 0) = available[N-1]

        // Random controller passes priority with 30% probability
        // This allows actions to be taken most of the time while still preventing infinite loops
        if available.is_empty() || self.rng.gen_bool(0.3) {
            // Pass priority = choice 0
            let player_name = view.player_name();
            view.logger()
                .controller_choice("RANDOM", &format!("{} chose 'p' (pass priority)", player_name));
            return None;
        }

        // Randomly choose one of the available spell abilities
        let ability_index = self.rng.gen_range(0..available.len());

        // Display which choice was made
        let choice_description = match &available[ability_index] {
            SpellAbility::PlayLand { card_id } => {
                format!("Play land: {}", view.card_name(*card_id).unwrap_or_default())
            }
            SpellAbility::CastSpell { card_id } => {
                format!("Cast spell: {}", view.card_name(*card_id).unwrap_or_default())
            }
            SpellAbility::ActivateAbility { card_id, .. } => {
                format!("Activate ability: {}", view.card_name(*card_id).unwrap_or_default())
            }
        };

        let player_name = view.player_name();
        view.logger().controller_choice(
            "RANDOM",
            &format!("{} chose {} - {}", player_name, ability_index, choice_description),
        );
        Some(available[ability_index].clone())
    }

    fn choose_targets(
        &mut self,
        view: &GameStateView,
        _spell: CardId,
        valid_targets: &[CardId],
    ) -> SmallVec<[CardId; 4]> {
        // For now, just pick a random target if any are available
        // TODO: Improve targeting logic based on spell requirements
        if valid_targets.is_empty() {
            // Only log when there are no targets (could be meaningful)
            view.logger()
                .controller_choice("RANDOM", "chose no targets (none available)");
            SmallVec::new()
        } else if valid_targets.len() == 1 {
            // Only one target available - no choice to make, don't log
            let mut targets = SmallVec::new();
            targets.push(valid_targets[0]);
            targets
        } else {
            // Multiple targets - this is a real choice
            let index = self.rng.gen_range(0..valid_targets.len());
            view.logger().controller_choice(
                "RANDOM",
                &format!("chose target {} out of choices 0-{}", index, valid_targets.len() - 1),
            );
            let mut targets = SmallVec::new();
            targets.push(valid_targets[index]);
            targets
        }
    }

    fn choose_mana_sources_to_pay(
        &mut self,
        view: &GameStateView,
        cost: &ManaCost,
        available_sources: &[CardId],
    ) -> SmallVec<[CardId; 8]> {
        // Simple greedy approach: tap sources until we have enough mana
        // TODO: Improve to consider mana colors and optimization
        let mut sources = SmallVec::new();
        let needed = cost.cmc() as usize;

        // Shuffle to randomize which sources we choose
        let mut shuffled: Vec<CardId> = available_sources.to_vec();
        shuffled.shuffle(&mut self.rng);

        // Only log if there's a real choice (more sources than needed)
        if available_sources.len() > needed {
            view.logger().controller_choice(
                "RANDOM",
                &format!(
                    "chose {} mana sources (shuffled from {} available sources)",
                    needed.min(available_sources.len()),
                    available_sources.len()
                ),
            );
        }

        for &source_id in shuffled.iter().take(needed) {
            sources.push(source_id);
        }

        sources
    }

    fn choose_attackers(&mut self, view: &GameStateView, available_creatures: &[CardId]) -> SmallVec<[CardId; 8]> {
        // Randomly decide whether each creature attacks
        let mut attackers = SmallVec::new();

        for (idx, &creature_id) in available_creatures.iter().enumerate() {
            // 50% chance each creature attacks
            if self.rng.gen_bool(0.5) {
                view.logger().controller_choice(
                    "RANDOM",
                    &format!(
                        "chose creature {} to attack (50% probability) out of {} available creatures",
                        idx,
                        available_creatures.len()
                    ),
                );
                attackers.push(creature_id);
            }
        }

        if attackers.is_empty() && !available_creatures.is_empty() {
            view.logger().controller_choice(
                "RANDOM",
                &format!(
                    "chose no attackers from {} available creatures",
                    available_creatures.len()
                ),
            );
        }

        attackers
    }

    fn choose_blockers(
        &mut self,
        view: &GameStateView,
        available_blockers: &[CardId],
        attackers: &[CardId],
    ) -> SmallVec<[(CardId, CardId); 8]> {
        // Randomly assign blockers to attackers
        let mut blocks = SmallVec::new();

        if attackers.is_empty() {
            view.logger()
                .controller_choice("RANDOM", "chose no blockers (no attackers to block)");
            return blocks;
        }

        for (blocker_idx, &blocker_id) in available_blockers.iter().enumerate() {
            // 50% chance each creature blocks
            if self.rng.gen_bool(0.5) {
                // Pick a random attacker to block
                let attacker_idx = self.rng.gen_range(0..attackers.len());
                view.logger().controller_choice(
                    "RANDOM",
                    &format!(
                        "chose blocker {} (50% probability) to block attacker {} out of {} attackers",
                        blocker_idx,
                        attacker_idx,
                        attackers.len()
                    ),
                );
                blocks.push((blocker_id, attackers[attacker_idx]));
            }
        }

        if blocks.is_empty() && !available_blockers.is_empty() {
            view.logger().controller_choice(
                "RANDOM",
                &format!("chose no blockers from {} available blockers", available_blockers.len()),
            );
        }

        blocks
    }

    fn choose_damage_assignment_order(
        &mut self,
        view: &GameStateView,
        _attacker: CardId,
        blockers: &[CardId],
    ) -> SmallVec<[CardId; 4]> {
        // Randomly shuffle the blockers to create a damage assignment order
        let mut ordered_blockers: Vec<CardId> = blockers.to_vec();
        ordered_blockers.shuffle(&mut self.rng);

        // Only log if there's a real choice (2+ blockers to order)
        if blockers.len() >= 2 {
            view.logger().controller_choice(
                "RANDOM",
                &format!("chose damage assignment order (shuffled {} blockers)", blockers.len()),
            );
        }

        ordered_blockers.into_iter().collect()
    }

    fn choose_cards_to_discard(
        &mut self,
        view: &GameStateView,
        hand: &[CardId],
        count: usize,
    ) -> SmallVec<[CardId; 7]> {
        // Randomly choose cards to discard from hand
        let mut hand_vec: Vec<CardId> = hand.to_vec();
        hand_vec.shuffle(&mut self.rng);

        let num_discarding = count.min(hand.len());

        // Only log if there's a real choice (more cards than we need to discard)
        if hand.len() > count {
            view.logger().controller_choice(
                "RANDOM",
                &format!(
                    "chose {} cards to discard (shuffled from {} cards in hand)",
                    num_discarding,
                    hand.len()
                ),
            );
        }

        hand_vec.iter().take(num_discarding).copied().collect()
    }

    fn on_priority_passed(&mut self, _view: &GameStateView) {
        // Random AI doesn't need to react to priority passes
    }

    fn on_game_end(&mut self, _view: &GameStateView, _won: bool) {
        // Could log game result here for statistics
        // Disabled for quiet operation during benchmarks and batch runs
    }

    fn get_snapshot_state(&self) -> Option<serde_json::Value> {
        // Wrap in ControllerState::Random to match the expected format
        // This ensures the JSON has the correct "controller_type": "Random" tag
        let state = crate::game::ControllerState::Random(self.clone());
        serde_json::to_value(state).ok()
    }

    fn get_controller_type(&self) -> crate::game::snapshot::ControllerType {
        crate::game::snapshot::ControllerType::Random
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::EntityId;
    use crate::game::GameState;

    #[test]
    fn test_random_controller_creation() {
        let player_id = EntityId::new(1);
        let controller = RandomController::with_seed(player_id, 42);
        assert_eq!(controller.player_id(), player_id);
    }

    #[test]
    fn test_seeded_controller() {
        let player_id = EntityId::new(1);
        let controller = RandomController::with_seed(player_id, 12345);
        assert_eq!(controller.player_id(), player_id);
    }

    #[test]
    fn test_choose_spell_ability_empty() {
        let player_id = EntityId::new(1);
        let mut controller = RandomController::with_seed(player_id, 100);
        let game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
        let view = GameStateView::new(&game, player_id);

        // With no available abilities, should return None
        let choice = controller.choose_spell_ability_to_play(&view, &[]);
        assert_eq!(choice, None);
    }

    #[test]
    fn test_choose_spell_ability() {
        let player_id = EntityId::new(1);
        let mut controller = RandomController::with_seed(player_id, 200);
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

        // May choose an ability or pass (due to 30% pass probability)
        // Try multiple times to ensure it makes choices sometimes
        let mut found_choice = false;
        for _ in 0..20 {
            let choice = controller.choose_spell_ability_to_play(&view, &abilities);
            if let Some(chosen) = choice {
                found_choice = true;
                // The choice should be one of the available abilities
                assert!(abilities.contains(&chosen));
            }
        }
        // With 30% pass rate, over 20 tries we should see at least one choice
        assert!(found_choice);
    }

    #[test]
    fn test_choose_targets() {
        let player_id = EntityId::new(1);
        let mut controller = RandomController::with_seed(player_id, 300);
        let game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
        let view = GameStateView::new(&game, player_id);

        let spell_id = EntityId::new(100);
        let valid_targets = vec![EntityId::new(20), EntityId::new(21), EntityId::new(22)];
        let targets = controller.choose_targets(&view, spell_id, &valid_targets);

        // Should choose exactly one target
        assert_eq!(targets.len(), 1);
        // Target should be from the valid list
        assert!(valid_targets.contains(&targets[0]));
    }

    #[test]
    fn test_choose_mana_sources() {
        let player_id = EntityId::new(1);
        let mut controller = RandomController::with_seed(player_id, 400);
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

        // Should choose exactly 4 sources (equal to CMC)
        assert_eq!(sources.len(), 4);
        // All sources should be from the available list
        for source in sources.iter() {
            assert!(available.contains(source));
        }
    }

    #[test]
    fn test_choose_attackers() {
        let player_id = EntityId::new(1);
        let mut controller = RandomController::with_seed(player_id, 500);
        let game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
        let view = GameStateView::new(&game, player_id);

        let creatures = vec![EntityId::new(20), EntityId::new(21), EntityId::new(22)];
        let attackers = controller.choose_attackers(&view, &creatures);

        // Should return a SmallVec (possibly empty)
        // All attackers should be from the available creatures
        for attacker in attackers.iter() {
            assert!(creatures.contains(attacker));
        }
    }

    #[test]
    fn test_choose_cards_to_discard() {
        let player_id = EntityId::new(1);
        let mut controller = RandomController::with_seed(player_id, 600);
        let game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
        let view = GameStateView::new(&game, player_id);

        let hand = vec![
            EntityId::new(30),
            EntityId::new(31),
            EntityId::new(32),
            EntityId::new(33),
        ];

        let discards = controller.choose_cards_to_discard(&view, &hand, 2);

        // Should discard exactly 2 cards
        assert_eq!(discards.len(), 2);

        // All discarded cards should be from hand
        for card in discards.iter() {
            assert!(hand.contains(card));
        }
    }
}
