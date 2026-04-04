//! Tests for combat-related bugfixes
//!
//! Each test reproduces a specific bug found during playtesting and verifies the fix.

use crate::core::effects::{ActivatedAbility, Effect};
use crate::core::{Card, CardType, Color, Cost, Keyword};
use crate::game::state::GameState;
use crate::game::zero_controller::ZeroController;
use crate::game::GameLoop;

#[cfg(test)]
mod tests {
    use super::*;

    // ========================================================================
    // Bug 1: Summoning sickness - creatures can tap-activate the turn they enter
    // CR 302.6: A creature's activated abilities with the tap symbol can't be
    // activated unless the creature has been under its controller's control
    // continuously since the start of their most recent turn (or has haste).
    // ========================================================================

    #[test]
    fn test_summoning_sick_creature_cannot_use_tap_ability() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let players: Vec<_> = game.players.iter().map(|p| p.id).collect();
        let p1_id = players[0];
        let p2_id = players[1];

        // Create a creature with a {T}: Deal 1 damage activated ability (like Prodigal Sorcerer)
        let creature_id = game.next_card_id();
        let mut creature = Card::new(creature_id, "Prodigal Sorcerer".to_string(), p1_id);
        creature.add_type(CardType::Creature);
        creature.set_base_power(Some(1));
        creature.set_base_toughness(Some(1));
        creature.controller = p1_id;
        // Creature entered THIS turn (summoning sickness)
        creature.turn_entered_battlefield = Some(game.turn.turn_number);

        // Add a tap-activated ability
        let ability = ActivatedAbility::new(
            Cost::Tap,
            vec![Effect::DealDamage {
                target: crate::core::TargetRef::None,
                amount: 1,
            }],
            "{T}: Deal 1 damage to any target".to_string(),
            false, // not a mana ability
        );
        creature.activated_abilities.push(ability);

        game.cards.insert(creature_id, creature);
        game.battlefield.add(creature_id);

        // Need a valid target so we isolate the summoning sickness check
        let target_id = game.next_card_id();
        let mut target = Card::new(target_id, "Grizzly Bears".to_string(), p2_id);
        target.add_type(CardType::Creature);
        target.set_base_power(Some(2));
        target.set_base_toughness(Some(2));
        target.controller = p2_id;
        game.cards.insert(target_id, target);
        game.battlefield.add(target_id);

        // Create GameLoop and check available abilities
        let mut game_loop = GameLoop::new(&mut game);
        game_loop.push_activatable_abilities_for_test(p1_id);

        // The tap ability should NOT be available due to summoning sickness
        let abilities = game_loop.get_abilities_buffer();
        let has_tap_ability = abilities.iter().any(
            |a| matches!(a, crate::core::SpellAbility::ActivateAbility { card_id, .. } if *card_id == creature_id),
        );
        assert!(
            !has_tap_ability,
            "Creature with summoning sickness should not be able to activate tap ability"
        );
    }

    #[test]
    fn test_creature_with_haste_can_use_tap_ability_same_turn() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let players: Vec<_> = game.players.iter().map(|p| p.id).collect();
        let p1_id = players[0];
        let p2_id = players[1];

        // Create a creature with Haste and a tap ability
        let creature_id = game.next_card_id();
        let mut creature = Card::new(creature_id, "Hasty Pinger".to_string(), p1_id);
        creature.add_type(CardType::Creature);
        creature.set_base_power(Some(1));
        creature.set_base_toughness(Some(1));
        creature.controller = p1_id;
        creature.turn_entered_battlefield = Some(game.turn.turn_number);
        creature.keywords.insert(Keyword::Haste);

        let ability = ActivatedAbility::new(
            Cost::Tap,
            vec![Effect::DealDamage {
                target: crate::core::TargetRef::None,
                amount: 1,
            }],
            "{T}: Deal 1 damage to any target".to_string(),
            false,
        );
        creature.activated_abilities.push(ability);

        game.cards.insert(creature_id, creature);
        game.battlefield.add(creature_id);

        // Need a valid target
        let target_id = game.next_card_id();
        let mut target = Card::new(target_id, "Grizzly Bears".to_string(), p2_id);
        target.add_type(CardType::Creature);
        target.set_base_power(Some(2));
        target.set_base_toughness(Some(2));
        target.controller = p2_id;
        game.cards.insert(target_id, target);
        game.battlefield.add(target_id);

        let mut game_loop = GameLoop::new(&mut game);
        game_loop.push_activatable_abilities_for_test(p1_id);

        let abilities = game_loop.get_abilities_buffer();
        let has_tap_ability = abilities.iter().any(
            |a| matches!(a, crate::core::SpellAbility::ActivateAbility { card_id, .. } if *card_id == creature_id),
        );
        assert!(
            has_tap_ability,
            "Creature with Haste should be able to activate tap ability the turn it enters"
        );
    }

    #[test]
    fn test_noncreature_tap_ability_not_affected_by_summoning_sickness() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let players: Vec<_> = game.players.iter().map(|p| p.id).collect();
        let p1_id = players[0];
        let p2_id = players[1];

        // Create an artifact with a tap ability (not a creature, no summoning sickness)
        let artifact_id = game.next_card_id();
        let mut artifact = Card::new(artifact_id, "Rod of Ruin".to_string(), p1_id);
        artifact.add_type(CardType::Artifact);
        artifact.controller = p1_id;
        artifact.turn_entered_battlefield = Some(game.turn.turn_number);

        let ability = ActivatedAbility::new(
            Cost::Tap,
            vec![Effect::DealDamage {
                target: crate::core::TargetRef::None,
                amount: 1,
            }],
            "{T}: Deal 1 damage to any target".to_string(),
            false,
        );
        artifact.activated_abilities.push(ability);

        game.cards.insert(artifact_id, artifact);
        game.battlefield.add(artifact_id);

        // Need a valid target for the ability (opponent's creature)
        let target_id = game.next_card_id();
        let mut target = Card::new(target_id, "Grizzly Bears".to_string(), p2_id);
        target.add_type(CardType::Creature);
        target.set_base_power(Some(2));
        target.set_base_toughness(Some(2));
        target.controller = p2_id;
        game.cards.insert(target_id, target);
        game.battlefield.add(target_id);

        let mut game_loop = GameLoop::new(&mut game);
        game_loop.push_activatable_abilities_for_test(p1_id);

        let abilities = game_loop.get_abilities_buffer();
        let has_tap_ability = abilities.iter().any(
            |a| matches!(a, crate::core::SpellAbility::ActivateAbility { card_id, .. } if *card_id == artifact_id),
        );
        assert!(
            has_tap_ability,
            "Non-creature artifacts should not be affected by summoning sickness"
        );
    }

    // ========================================================================
    // Bug 2: Tapped creatures can illegally block
    // CR 509.1a: Only an untapped creature can be declared as a blocker.
    // ========================================================================

    #[test]
    fn test_tapped_creature_cannot_block() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let players: Vec<_> = game.players.iter().map(|p| p.id).collect();
        let p1_id = players[0];
        let p2_id = players[1];

        // Create an attacker
        let attacker_id = game.next_card_id();
        let mut attacker = Card::new(attacker_id, "Grizzly Bears".to_string(), p1_id);
        attacker.add_type(CardType::Creature);
        attacker.set_base_power(Some(2));
        attacker.set_base_toughness(Some(2));
        attacker.controller = p1_id;
        game.cards.insert(attacker_id, attacker);
        game.battlefield.add(attacker_id);
        game.combat.declare_attacker(attacker_id, p2_id);

        // Create a TAPPED creature as potential blocker
        let blocker_id = game.next_card_id();
        let mut blocker = Card::new(blocker_id, "Royal Assassin Target".to_string(), p2_id);
        blocker.add_type(CardType::Creature);
        blocker.set_base_power(Some(2));
        blocker.set_base_toughness(Some(2));
        blocker.controller = p2_id;
        blocker.tapped = true; // Already tapped
        game.cards.insert(blocker_id, blocker);
        game.battlefield.add(blocker_id);

        // Attempting to declare a tapped creature as blocker should fail
        let result = game.declare_blocker(p2_id, blocker_id, vec![attacker_id]);
        assert!(result.is_err(), "Tapped creature should not be able to block");
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("tapped"),
            "Error should mention tapped status, got: {err_msg}"
        );
    }

    // ========================================================================
    // Bug 3: Trample damage not assigned to defending player
    // CR 702.19c: If an attacking creature with trample is blocked,
    // it assigns lethal damage to each creature blocking it, then
    // assigns the rest to the defending player.
    // ========================================================================

    #[test]
    fn test_trample_carnage_tyrant_excess_to_player() {
        use crate::game::random_controller::RandomController;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1: Carnage Tyrant 7/6 with Trample
        let attacker_id = game.next_entity_id();
        let mut attacker = Card::new(attacker_id, "Carnage Tyrant".to_string(), p1_id);
        attacker.add_type(CardType::Creature);
        attacker.set_base_power(Some(7));
        attacker.set_base_toughness(Some(6));
        attacker.controller = p1_id;
        attacker.keywords.insert(Keyword::Trample);
        attacker.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        game.cards.insert(attacker_id, attacker);
        game.battlefield.add(attacker_id);

        // P2: 2/2 blocker
        let blocker_id = game.next_entity_id();
        let mut blocker = Card::new(blocker_id, "Grizzly Bears".to_string(), p2_id);
        blocker.add_type(CardType::Creature);
        blocker.set_base_power(Some(2));
        blocker.set_base_toughness(Some(2));
        blocker.controller = p2_id;
        game.cards.insert(blocker_id, blocker);
        game.battlefield.add(blocker_id);

        // Declare combat
        game.combat.declare_attacker(attacker_id, p2_id);
        let attacker_vec = smallvec::smallvec![attacker_id];
        game.combat.declare_blocker(blocker_id, attacker_vec);

        let p2_life_before = game.players[1].life;

        // Assign combat damage
        let mut controller1 = RandomController::with_seed(p1_id, 42);
        let mut controller2 = ZeroController::new(p2_id);
        let result = game.assign_combat_damage(&mut controller1, &mut controller2, false);
        assert!(result.is_ok(), "Combat damage assignment failed: {result:?}");

        // Blocker should be dead
        if let Some(zones) = game.get_player_zones(p2_id) {
            assert!(zones.graveyard.contains(blocker_id), "Blocker should be in graveyard");
        }

        // P2 should have taken 5 trample damage (7 power - 2 to kill blocker)
        let p2_life_after = game.players[1].life;
        assert_eq!(
            p2_life_after,
            p2_life_before - 5,
            "P2 should take 5 trample damage (7 power - 2 lethal to blocker), \
             but life went from {p2_life_before} to {p2_life_after}"
        );
    }

    // ========================================================================
    // Bug 4: Protection from black not enforced for targeting
    // CR 702.16b: A permanent or player with protection can't be targeted by
    // spells with the stated quality.
    // ========================================================================

    #[test]
    fn test_protection_prevents_targeting() {
        use crate::game::actions::targeting::is_legal_target;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // White Knight has Protection from Black
        let knight_id = game.next_card_id();
        let mut knight = Card::new(knight_id, "White Knight".to_string(), p2_id);
        knight.add_type(CardType::Creature);
        knight.set_base_power(Some(2));
        knight.set_base_toughness(Some(2));
        knight.controller = p2_id;
        knight.keywords.insert(Keyword::ProtectionFromBlack);
        knight.colors.push(Color::White);
        game.cards.insert(knight_id, knight);
        game.battlefield.add(knight_id);

        // A black source (Terror, Doom Blade, etc.) should not be able to target White Knight
        let black_source_colors = &[Color::Black];
        let knight = game.cards.get(knight_id).unwrap();
        assert!(
            !is_legal_target(knight, p1_id, black_source_colors),
            "White Knight with Protection from Black should not be targetable by black spells"
        );

        // A red source SHOULD be able to target White Knight
        let red_source_colors = &[Color::Red];
        assert!(
            is_legal_target(knight, p1_id, red_source_colors),
            "White Knight should be targetable by non-black spells"
        );

        // A colorless source SHOULD be able to target White Knight
        let colorless_source_colors: &[Color] = &[];
        assert!(
            is_legal_target(knight, p1_id, colorless_source_colors),
            "White Knight should be targetable by colorless spells"
        );
    }

    #[test]
    fn test_protection_from_multiple_colors() {
        use crate::game::actions::targeting::is_legal_target;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // A creature with protection from both black and red
        let creature_id = game.next_card_id();
        let mut creature = Card::new(creature_id, "Progenitus Jr".to_string(), p2_id);
        creature.add_type(CardType::Creature);
        creature.set_base_power(Some(3));
        creature.set_base_toughness(Some(3));
        creature.controller = p2_id;
        creature.keywords.insert(Keyword::ProtectionFromBlack);
        creature.keywords.insert(Keyword::ProtectionFromRed);
        game.cards.insert(creature_id, creature);
        game.battlefield.add(creature_id);

        let creature = game.cards.get(creature_id).unwrap();

        // A multicolor black/red spell should be blocked
        assert!(
            !is_legal_target(creature, p1_id, &[Color::Black, Color::Red]),
            "Should not be targetable by black/red spell"
        );

        // A green spell should be fine
        assert!(
            is_legal_target(creature, p1_id, &[Color::Green]),
            "Should be targetable by green spell"
        );
    }

    // ========================================================================
    // Bug 5: Cleanup step - non-active player incorrectly forced to discard
    // CR 514.1: "First, if the active player's hand contains more cards than
    // their maximum hand size, they discard enough cards to reduce their hand
    // size to that number." Only the ACTIVE player discards.
    // ========================================================================

    #[test]
    fn test_cleanup_only_active_player_discards() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // Set active player to P1
        game.turn.active_player = p1_id;

        // Give P1 exactly 7 cards (max hand size, no discard needed)
        for _ in 0..7 {
            let card_id = game.next_card_id();
            let card = Card::new(card_id, "Forest".to_string(), p1_id);
            game.cards.insert(card_id, card);
            game.get_player_zones_mut(p1_id).unwrap().hand.add(card_id);
        }

        // Give P2 (non-active) 9 cards (2 over max hand size)
        for _ in 0..9 {
            let card_id = game.next_card_id();
            let card = Card::new(card_id, "Swamp".to_string(), p2_id);
            game.cards.insert(card_id, card);
            game.get_player_zones_mut(p2_id).unwrap().hand.add(card_id);
        }

        // Add library cards so game doesn't end from draw
        for _ in 0..10 {
            let card_id = game.next_card_id();
            let card = Card::new(card_id, "Mountain".to_string(), p1_id);
            game.cards.insert(card_id, card);
            game.get_player_zones_mut(p1_id).unwrap().library.add(card_id);
        }
        for _ in 0..10 {
            let card_id = game.next_card_id();
            let card = Card::new(card_id, "Island".to_string(), p2_id);
            game.cards.insert(card_id, card);
            game.get_player_zones_mut(p2_id).unwrap().library.add(card_id);
        }

        // Run cleanup step
        let mut controller1 = ZeroController::new(p1_id);
        let mut controller2 = ZeroController::new(p2_id);
        let mut game_loop = GameLoop::new(&mut game).with_verbosity(crate::game::VerbosityLevel::Silent);
        let result = game_loop.cleanup_step_for_test(&mut controller1, &mut controller2);
        assert!(result.is_ok(), "Cleanup step failed: {result:?}");

        // P1 (active) had 7 cards — no discard needed
        let p1_hand = game_loop.game.get_player_zones(p1_id).unwrap().hand.len();
        assert_eq!(p1_hand, 7, "P1 should still have 7 cards");

        // P2 (non-active) had 9 cards — should NOT have been forced to discard
        // CR 514.1 says only active player discards during cleanup
        let p2_hand = game_loop.game.get_player_zones(p2_id).unwrap().hand.len();
        assert_eq!(
            p2_hand, 9,
            "P2 (non-active) should NOT discard during opponent's cleanup step, \
             hand should still be 9 but was {p2_hand}"
        );
    }
}
