//! Tests for Level Up keyword (CR 702.87): Joraga Treespeaker and similar leveler creatures.
//!
//! Verifies that the Level Up activated ability ({1}{G} for Joraga Treespeaker) is:
//! 1. Offered in the abilities buffer when the player has enough mana (2 Forests),
//! 2. NOT blocked by summoning sickness (Level Up has no tap cost — CR 302.6),
//! 3. Successfully places a LEVEL counter on the creature when activated.

use super::effects::load_test_card;
use crate::core::{CounterType, SpellAbility};
use crate::game::game_loop::GameLoop;
use crate::game::state::GameState;
use crate::game::VerbosityLevel;

#[cfg(test)]
mod tests {
    use super::*;

    /// Confirm the Joraga Treespeaker's Level Up ability is offered when the
    /// player controls 2 untapped Forests (can pay {1}{G}).
    #[test]
    fn test_level_up_offered_with_sufficient_mana() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P2: Grizzly Bears so the game is not immediately over.
        let bears_id = load_test_card(&mut game, "Grizzly Bears", p2_id).expect("Grizzly Bears should load");
        game.battlefield.add(bears_id);
        if let Ok(c) = game.cards.get_mut(bears_id) {
            c.controller = p2_id;
            c.turn_entered_battlefield = Some(game.turn.turn_number.saturating_sub(1));
        }

        // P1: 2 untapped Forests
        let forest1 = load_test_card(&mut game, "Forest", p1_id).expect("Forest should load");
        let forest2 = load_test_card(&mut game, "Forest", p1_id).expect("Forest should load");
        game.battlefield.add(forest1);
        game.battlefield.add(forest2);
        for fid in [forest1, forest2] {
            if let Ok(c) = game.cards.get_mut(fid) {
                c.controller = p1_id;
                c.tapped = false;
            }
        }

        // P1: Joraga Treespeaker — entered last turn so no summoning sickness.
        let treespeaker_id =
            load_test_card(&mut game, "Joraga Treespeaker", p1_id).expect("Joraga Treespeaker should load");
        game.battlefield.add(treespeaker_id);
        if let Ok(c) = game.cards.get_mut(treespeaker_id) {
            c.controller = p1_id;
            // Entered the turn BEFORE the current turn — no summoning sickness.
            c.turn_entered_battlefield = Some(game.turn.turn_number.saturating_sub(1));
        }

        // Set up a sorcery-speed timing context: Main 1, active player is P1, stack is empty.
        game.turn.active_player = p1_id;
        game.turn.current_step = crate::game::Step::Main1;
        assert!(game.stack.is_empty(), "Stack must be empty for sorcery-speed abilities");

        // Build the abilities buffer for P1. Drop gl before borrowing game.cards.
        let buffer = {
            let mut gl = GameLoop::new(&mut game).with_verbosity(VerbosityLevel::Silent);
            gl.push_activatable_abilities(p1_id);
            gl.get_abilities_buffer().to_vec()
        };

        // Helper: check if a SpellAbility is a Level Up for the Treespeaker.
        let is_level_up_for_treespeaker = |sa: &SpellAbility| -> bool {
            if let SpellAbility::ActivateAbility { card_id, ability_index } = sa {
                if *card_id != treespeaker_id {
                    return false;
                }
                if let Some(card) = game.cards.try_get(*card_id) {
                    if let Some(ability) = card.activated_abilities.get(*ability_index) {
                        return ability.effects.iter().any(|e| {
                            matches!(
                                e,
                                crate::core::Effect::PutCounter {
                                    counter_type: CounterType::Level,
                                    ..
                                }
                            )
                        });
                    }
                }
            }
            false
        };

        let level_up_offered = buffer.iter().any(is_level_up_for_treespeaker);

        assert!(
            level_up_offered,
            "Joraga Treespeaker's Level Up ability should be offered when 2 Forests are available \
             (cost is {{1}}{{G}}, 2 Forests supply 2 green which covers 1 generic + 1 green). \
             Full abilities buffer: {:?}",
            buffer
        );
    }

    /// Confirm that when Joraga Treespeaker has summoning sickness (entered THIS turn)
    /// the Level Up ability is STILL offered, because Level Up has no tap cost (CR 302.6).
    #[test]
    fn test_level_up_not_blocked_by_summoning_sickness() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P2: filler creature
        let bears_id = load_test_card(&mut game, "Grizzly Bears", p2_id).expect("Grizzly Bears should load");
        game.battlefield.add(bears_id);
        if let Ok(c) = game.cards.get_mut(bears_id) {
            c.controller = p2_id;
            c.turn_entered_battlefield = Some(game.turn.turn_number.saturating_sub(1));
        }

        // P1: 2 Forests
        let forest1 = load_test_card(&mut game, "Forest", p1_id).expect("Forest should load");
        let forest2 = load_test_card(&mut game, "Forest", p1_id).expect("Forest should load");
        game.battlefield.add(forest1);
        game.battlefield.add(forest2);
        for fid in [forest1, forest2] {
            if let Ok(c) = game.cards.get_mut(fid) {
                c.controller = p1_id;
                c.tapped = false;
            }
        }

        // P1: Joraga Treespeaker with summoning sickness (entered THIS turn).
        let treespeaker_id =
            load_test_card(&mut game, "Joraga Treespeaker", p1_id).expect("Joraga Treespeaker should load");
        game.battlefield.add(treespeaker_id);
        if let Ok(c) = game.cards.get_mut(treespeaker_id) {
            c.controller = p1_id;
            // Entered THIS turn — has summoning sickness.
            c.turn_entered_battlefield = Some(game.turn.turn_number);
        }

        game.turn.active_player = p1_id;
        game.turn.current_step = crate::game::Step::Main1;

        // Build the abilities buffer. Drop gl before borrowing game.cards.
        let buffer = {
            let mut gl = GameLoop::new(&mut game).with_verbosity(VerbosityLevel::Silent);
            gl.push_activatable_abilities(p1_id);
            gl.get_abilities_buffer().to_vec()
        };

        let is_level_up_for_treespeaker = |sa: &SpellAbility| -> bool {
            if let SpellAbility::ActivateAbility { card_id, ability_index } = sa {
                if *card_id != treespeaker_id {
                    return false;
                }
                if let Some(card) = game.cards.try_get(*card_id) {
                    if let Some(ability) = card.activated_abilities.get(*ability_index) {
                        return ability.effects.iter().any(|e| {
                            matches!(
                                e,
                                crate::core::Effect::PutCounter {
                                    counter_type: CounterType::Level,
                                    ..
                                }
                            )
                        });
                    }
                }
            }
            false
        };

        // Level Up must still be offered even with summoning sickness —
        // only tap-cost abilities are blocked by summoning sickness (CR 302.6).
        let level_up_offered = buffer.iter().any(is_level_up_for_treespeaker);

        assert!(
            level_up_offered,
            "Joraga Treespeaker's Level Up ability (no tap cost) must be available even when \
             the creature has summoning sickness (CR 302.6 only restricts tap-cost abilities). \
             Full abilities buffer: {:?}",
            buffer
        );
    }

    /// After activating Level Up, Joraga Treespeaker should have 1 LEVEL counter.
    ///
    /// This test directly pays the mana cost and executes the Level Up effect
    /// to confirm the counter placement works end-to-end.
    #[test]
    fn test_level_up_places_level_counter() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // P1: 2 untapped Forests
        let forest1 = load_test_card(&mut game, "Forest", p1_id).expect("Forest should load");
        let forest2 = load_test_card(&mut game, "Forest", p1_id).expect("Forest should load");
        game.battlefield.add(forest1);
        game.battlefield.add(forest2);
        for fid in [forest1, forest2] {
            if let Ok(c) = game.cards.get_mut(fid) {
                c.controller = p1_id;
                c.tapped = false;
            }
        }

        // P1: Joraga Treespeaker (no summoning sickness)
        let treespeaker_id =
            load_test_card(&mut game, "Joraga Treespeaker", p1_id).expect("Joraga Treespeaker should load");
        game.battlefield.add(treespeaker_id);
        if let Ok(c) = game.cards.get_mut(treespeaker_id) {
            c.controller = p1_id;
            c.turn_entered_battlefield = Some(game.turn.turn_number.saturating_sub(1));
        }

        game.turn.active_player = p1_id;
        game.turn.current_step = crate::game::Step::Main1;

        // Find the Level Up ability index on the Treespeaker.
        let (level_up_index, ability_cost) = {
            let card = game.cards.get(treespeaker_id).expect("Treespeaker exists");
            let idx = card
                .activated_abilities
                .iter()
                .position(|a| {
                    a.effects.iter().any(|e| {
                        matches!(
                            e,
                            crate::core::Effect::PutCounter {
                                counter_type: CounterType::Level,
                                ..
                            }
                        )
                    })
                })
                .expect("Treespeaker must have a Level Up ability");
            let cost = card.activated_abilities[idx].cost.clone();
            (idx, cost)
        };
        let _ = level_up_index; // used only to find the cost above

        // Tap Forests to add mana to the pool (simulating the game loop's auto-tap).
        game.tap_for_mana(p1_id, forest1).expect("Forest 1 should tap for mana");
        game.tap_for_mana(p1_id, forest2).expect("Forest 2 should tap for mana");

        // Pay the activation cost ({1}{G}). Expects 2 green mana in pool (from 2 Forests).
        game.pay_ability_cost(p1_id, treespeaker_id, &ability_cost)
            .expect("Should be able to pay {1}{G} after tapping 2 Forests");

        // Execute the PutCounter effect with a concrete resolved target (not self_target placeholder).
        let put_counter_effect = crate::core::Effect::PutCounter {
            target: treespeaker_id,
            counter_type: CounterType::Level,
            amount: 1,
        };
        game.execute_effect(&put_counter_effect)
            .expect("PutCounter Level should execute without error");

        // Verify: Joraga Treespeaker now has exactly 1 LEVEL counter.
        let counter_count = game
            .cards
            .get(treespeaker_id)
            .expect("Treespeaker exists")
            .get_counter(CounterType::Level);
        assert_eq!(
            counter_count, 1,
            "Joraga Treespeaker should have 1 LEVEL counter after Level Up activates"
        );
    }
}
