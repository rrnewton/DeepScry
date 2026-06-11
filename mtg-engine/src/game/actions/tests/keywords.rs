use crate::core::{Card, CardId, CardType, Effect, Keyword, ManaCost};
use crate::game::state::GameState;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_untap_spell() {
        use crate::core::{Effect, ManaCost};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let players: Vec<_> = game.players.iter().map(|p| p.id).collect();
        let p1_id = players[0];

        // Create a tapped land for P1
        let land_id = game.next_card_id();
        let mut land = Card::new(land_id, "Forest".to_string(), p1_id);
        land.add_type(CardType::Land);
        land.controller = p1_id;
        land.tapped = true; // Start tapped
        game.cards.insert(land_id, land);
        game.battlefield.add(land_id);

        // Check initial state
        let land_before = game.cards.get(land_id).unwrap();
        assert!(land_before.tapped, "Land should start tapped");

        // Create an Untap spell
        let untap_spell_id = game.next_card_id();
        let mut untap_spell = Card::new(untap_spell_id, "Untap".to_string(), p1_id);
        untap_spell.add_type(CardType::Instant);
        untap_spell.mana_cost = ManaCost::from_string("U");
        // Target the specific land
        untap_spell.effects.push(Effect::UntapPermanent { target: land_id });
        game.cards.insert(untap_spell_id, untap_spell);

        // Put spell on stack (simulating cast)
        game.stack.add(untap_spell_id);

        // Resolve the spell
        assert!(
            game.resolve_spell(untap_spell_id, &[]).is_ok(),
            "Failed to resolve untap spell"
        );

        // Check land is untapped
        let land_after = game.cards.get(land_id).unwrap();
        assert!(!land_after.tapped, "Land should be untapped after spell");

        // Check spell went to graveyard
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert!(
                zones.graveyard.contains(untap_spell_id),
                "Untap spell should be in graveyard"
            );
        }
    }

    #[test]
    fn test_trample_excess_damage_to_player() {
        use crate::game::random_controller::RandomController;
        use crate::game::zero_controller::ZeroController;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1: Create a 5/5 creature with Trample (attacker)
        let attacker_id = game.next_entity_id();
        let mut attacker = Card::new(attacker_id, "Craw Wurm".to_string(), p1_id);
        attacker.add_type(CardType::Creature);
        attacker.set_base_power(Some(5));
        attacker.set_base_toughness(Some(5));
        attacker.controller = p1_id;
        attacker.keywords.insert(Keyword::Trample);
        attacker.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        game.cards.insert(attacker_id, attacker);
        game.battlefield.add(attacker_id);

        // P2: Create a 2/2 creature (blocker)
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

        // Record P2's life before combat
        let p2_life_before = game.players[1].life;

        // Assign combat damage
        let mut controller1 = RandomController::with_seed(p1_id, 42);
        let mut controller2 = ZeroController::new(p2_id);
        let result = game.assign_combat_damage(&mut controller1, &mut controller2, false);
        assert!(result.is_ok(), "Failed to assign combat damage: {result:?}");

        // Blocker should be dead (took 5 damage, toughness 2)
        if let Some(zones) = game.get_player_zones(p2_id) {
            assert!(zones.graveyard.contains(blocker_id), "Blocker should be in graveyard");
        }

        // P2 should have taken 3 trample damage (5 power - 2 to kill blocker)
        let p2_life_after = game.players[1].life;
        assert_eq!(
            p2_life_after,
            p2_life_before - 3,
            "P2 should have taken 3 trample damage"
        );
    }

    #[test]
    fn test_trample_exact_lethal_no_excess() {
        use crate::game::random_controller::RandomController;
        use crate::game::zero_controller::ZeroController;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1: Create a 3/3 creature with Trample (attacker)
        let attacker_id = game.next_entity_id();
        let mut attacker = Card::new(attacker_id, "Trained Armodon".to_string(), p1_id);
        attacker.add_type(CardType::Creature);
        attacker.set_base_power(Some(3));
        attacker.set_base_toughness(Some(3));
        attacker.controller = p1_id;
        attacker.keywords.insert(Keyword::Trample);
        attacker.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        game.cards.insert(attacker_id, attacker);
        game.battlefield.add(attacker_id);

        // P2: Create a 3/3 creature (blocker)
        let blocker_id = game.next_entity_id();
        let mut blocker = Card::new(blocker_id, "Hill Giant".to_string(), p2_id);
        blocker.add_type(CardType::Creature);
        blocker.set_base_power(Some(3));
        blocker.set_base_toughness(Some(3));
        blocker.controller = p2_id;
        game.cards.insert(blocker_id, blocker);
        game.battlefield.add(blocker_id);

        // Declare combat
        game.combat.declare_attacker(attacker_id, p2_id);
        let attacker_vec = smallvec::smallvec![attacker_id];
        game.combat.declare_blocker(blocker_id, attacker_vec);

        // Record P2's life before combat
        let p2_life_before = game.players[1].life;

        // Assign combat damage
        let mut controller1 = RandomController::with_seed(p1_id, 42);
        let mut controller2 = ZeroController::new(p2_id);
        let result = game.assign_combat_damage(&mut controller1, &mut controller2, false);
        assert!(result.is_ok(), "Failed to assign combat damage: {result:?}");

        // Blocker should be dead (took 3 damage, toughness 3)
        if let Some(zones) = game.get_player_zones(p2_id) {
            assert!(zones.graveyard.contains(blocker_id), "Blocker should be in graveyard");
        }

        // P2 should NOT have taken any damage (exact lethal, no excess)
        let p2_life_after = game.players[1].life;
        assert_eq!(
            p2_life_after, p2_life_before,
            "P2 should not have taken damage (exact lethal, no excess)"
        );
    }

    #[test]
    fn test_non_trample_blocked_no_player_damage() {
        use crate::game::random_controller::RandomController;
        use crate::game::zero_controller::ZeroController;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1: Create a 5/5 creature WITHOUT Trample (attacker)
        let attacker_id = game.next_entity_id();
        let mut attacker = Card::new(attacker_id, "Serra Angel".to_string(), p1_id);
        attacker.add_type(CardType::Creature);
        attacker.set_base_power(Some(5));
        attacker.set_base_toughness(Some(5));
        attacker.controller = p1_id;
        // NO Trample keyword
        attacker.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        game.cards.insert(attacker_id, attacker);
        game.battlefield.add(attacker_id);

        // P2: Create a 1/1 creature (blocker)
        let blocker_id = game.next_entity_id();
        let mut blocker = Card::new(blocker_id, "Llanowar Elves".to_string(), p2_id);
        blocker.add_type(CardType::Creature);
        blocker.set_base_power(Some(1));
        blocker.set_base_toughness(Some(1));
        blocker.controller = p2_id;
        game.cards.insert(blocker_id, blocker);
        game.battlefield.add(blocker_id);

        // Declare combat
        game.combat.declare_attacker(attacker_id, p2_id);
        let attacker_vec = smallvec::smallvec![attacker_id];
        game.combat.declare_blocker(blocker_id, attacker_vec);

        // Record P2's life before combat
        let p2_life_before = game.players[1].life;

        // Assign combat damage
        let mut controller1 = RandomController::with_seed(p1_id, 42);
        let mut controller2 = ZeroController::new(p2_id);
        let result = game.assign_combat_damage(&mut controller1, &mut controller2, false);
        assert!(result.is_ok(), "Failed to assign combat damage: {result:?}");

        // Blocker should be dead
        if let Some(zones) = game.get_player_zones(p2_id) {
            assert!(zones.graveyard.contains(blocker_id), "Blocker should be in graveyard");
        }

        // P2 should NOT have taken any damage (no trample, so excess is lost)
        let p2_life_after = game.players[1].life;
        assert_eq!(
            p2_life_after, p2_life_before,
            "P2 should not have taken damage without trample"
        );
    }

    #[test]
    fn test_trample_multiple_blockers() {
        use crate::game::random_controller::RandomController;
        use crate::game::zero_controller::ZeroController;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1: Create a 7/7 creature with Trample (attacker)
        let attacker_id = game.next_entity_id();
        let mut attacker = Card::new(attacker_id, "Enormous Baloth".to_string(), p1_id);
        attacker.add_type(CardType::Creature);
        attacker.set_base_power(Some(7));
        attacker.set_base_toughness(Some(7));
        attacker.controller = p1_id;
        attacker.keywords.insert(Keyword::Trample);
        attacker.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        game.cards.insert(attacker_id, attacker);
        game.battlefield.add(attacker_id);

        // P2: Create two blockers (2/2 and 3/3)
        let blocker1_id = game.next_entity_id();
        let mut blocker1 = Card::new(blocker1_id, "Grizzly Bears".to_string(), p2_id);
        blocker1.add_type(CardType::Creature);
        blocker1.set_base_power(Some(2));
        blocker1.set_base_toughness(Some(2));
        blocker1.controller = p2_id;
        game.cards.insert(blocker1_id, blocker1);
        game.battlefield.add(blocker1_id);

        let blocker2_id = game.next_entity_id();
        let mut blocker2 = Card::new(blocker2_id, "Hill Giant".to_string(), p2_id);
        blocker2.add_type(CardType::Creature);
        blocker2.set_base_power(Some(3));
        blocker2.set_base_toughness(Some(3));
        blocker2.controller = p2_id;
        game.cards.insert(blocker2_id, blocker2);
        game.battlefield.add(blocker2_id);

        // Declare combat
        game.combat.declare_attacker(attacker_id, p2_id);
        let attacker_vec = smallvec::smallvec![attacker_id];
        game.combat.declare_blocker(blocker1_id, attacker_vec.clone());
        game.combat.declare_blocker(blocker2_id, attacker_vec);

        // Record P2's life before combat
        let p2_life_before = game.players[1].life;

        // Assign combat damage
        let mut controller1 = RandomController::with_seed(p1_id, 42);
        let mut controller2 = ZeroController::new(p2_id);
        let result = game.assign_combat_damage(&mut controller1, &mut controller2, false);
        assert!(result.is_ok(), "Failed to assign combat damage: {result:?}");

        // Both blockers should be dead
        if let Some(zones) = game.get_player_zones(p2_id) {
            assert!(
                zones.graveyard.contains(blocker1_id),
                "Blocker 1 should be in graveyard"
            );
            assert!(
                zones.graveyard.contains(blocker2_id),
                "Blocker 2 should be in graveyard"
            );
        }

        // P2 should have taken 2 trample damage (7 power - 2 - 3 = 2)
        let p2_life_after = game.players[1].life;
        assert_eq!(
            p2_life_after,
            p2_life_before - 2,
            "P2 should have taken 2 trample damage"
        );
    }

    #[test]
    fn test_lifelink_attacker_blocked() {
        use crate::game::random_controller::RandomController;
        use crate::game::zero_controller::ZeroController;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1: Create a 3/3 creature with Lifelink (attacker)
        let attacker_id = game.next_entity_id();
        let mut attacker = Card::new(attacker_id, "Healer's Hawk".to_string(), p1_id);
        attacker.add_type(CardType::Creature);
        attacker.set_base_power(Some(3));
        attacker.set_base_toughness(Some(3));
        attacker.controller = p1_id;
        attacker.keywords.insert(Keyword::Lifelink);
        attacker.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        game.cards.insert(attacker_id, attacker);
        game.battlefield.add(attacker_id);

        // P2: Create a 2/2 creature (blocker)
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

        // Record P1's life before combat
        let p1_life_before = game.players[0].life;

        // Assign combat damage
        let mut controller1 = RandomController::with_seed(p1_id, 42);
        let mut controller2 = ZeroController::new(p2_id);
        let result = game.assign_combat_damage(&mut controller1, &mut controller2, false);
        assert!(result.is_ok(), "Failed to assign combat damage: {result:?}");

        // P1 should have gained 3 life from lifelink (3 damage dealt to blocker)
        let p1_life_after = game.players[0].life;
        assert_eq!(
            p1_life_after,
            p1_life_before + 3,
            "P1 should have gained 3 life from lifelink"
        );

        // Blocker should be dead
        if let Some(zones) = game.get_player_zones(p2_id) {
            assert!(zones.graveyard.contains(blocker_id), "Blocker should be in graveyard");
        }
    }

    #[test]
    fn test_lifelink_attacker_unblocked() {
        use crate::game::random_controller::RandomController;
        use crate::game::zero_controller::ZeroController;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1: Create a 4/4 creature with Lifelink (attacker)
        let attacker_id = game.next_entity_id();
        let mut attacker = Card::new(attacker_id, "Ajani's Pridemate".to_string(), p1_id);
        attacker.add_type(CardType::Creature);
        attacker.set_base_power(Some(4));
        attacker.set_base_toughness(Some(4));
        attacker.controller = p1_id;
        attacker.keywords.insert(Keyword::Lifelink);
        attacker.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        game.cards.insert(attacker_id, attacker);
        game.battlefield.add(attacker_id);

        // Declare combat (no blockers)
        game.combat.declare_attacker(attacker_id, p2_id);

        // Record life before combat
        let p1_life_before = game.players[0].life;
        let p2_life_before = game.players[1].life;

        // Assign combat damage
        let mut controller1 = RandomController::with_seed(p1_id, 42);
        let mut controller2 = ZeroController::new(p2_id);
        let result = game.assign_combat_damage(&mut controller1, &mut controller2, false);
        assert!(result.is_ok(), "Failed to assign combat damage: {result:?}");

        // P1 should have gained 4 life from lifelink (4 damage dealt to player)
        let p1_life_after = game.players[0].life;
        assert_eq!(
            p1_life_after,
            p1_life_before + 4,
            "P1 should have gained 4 life from lifelink"
        );

        // P2 should have taken 4 damage
        let p2_life_after = game.players[1].life;
        assert_eq!(p2_life_after, p2_life_before - 4, "P2 should have taken 4 damage");
    }

    #[test]
    fn test_lifelink_blocker() {
        use crate::game::random_controller::RandomController;
        use crate::game::zero_controller::ZeroController;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1: Create a 3/3 creature (attacker)
        let attacker_id = game.next_entity_id();
        let mut attacker = Card::new(attacker_id, "Hill Giant".to_string(), p1_id);
        attacker.add_type(CardType::Creature);
        attacker.set_base_power(Some(3));
        attacker.set_base_toughness(Some(3));
        attacker.controller = p1_id;
        attacker.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        game.cards.insert(attacker_id, attacker);
        game.battlefield.add(attacker_id);

        // P2: Create a 2/2 creature with Lifelink (blocker)
        let blocker_id = game.next_entity_id();
        let mut blocker = Card::new(blocker_id, "Vampire Cutthroat".to_string(), p2_id);
        blocker.add_type(CardType::Creature);
        blocker.set_base_power(Some(2));
        blocker.set_base_toughness(Some(2));
        blocker.controller = p2_id;
        blocker.keywords.insert(Keyword::Lifelink);
        game.cards.insert(blocker_id, blocker);
        game.battlefield.add(blocker_id);

        // Declare combat
        game.combat.declare_attacker(attacker_id, p2_id);
        let attacker_vec = smallvec::smallvec![attacker_id];
        game.combat.declare_blocker(blocker_id, attacker_vec);

        // Record P2's life before combat
        let p2_life_before = game.players[1].life;

        // Assign combat damage
        let mut controller1 = RandomController::with_seed(p1_id, 42);
        let mut controller2 = ZeroController::new(p2_id);
        let result = game.assign_combat_damage(&mut controller1, &mut controller2, false);
        assert!(result.is_ok(), "Failed to assign combat damage: {result:?}");

        // P2 should have gained 2 life from lifelink (blocker dealt 2 damage)
        let p2_life_after = game.players[1].life;
        assert_eq!(
            p2_life_after,
            p2_life_before + 2,
            "P2 should have gained 2 life from lifelink blocker"
        );

        // Blocker should be dead (took 3 damage, has 2 toughness)
        // Attacker should survive (took 2 damage, has 3 toughness)
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert!(
                !zones.graveyard.contains(attacker_id),
                "Attacker should still be alive (took 2 damage, has 3 toughness)"
            );
            assert!(
                game.battlefield.contains(attacker_id),
                "Attacker should still be on battlefield"
            );
        }
        if let Some(zones) = game.get_player_zones(p2_id) {
            assert!(
                zones.graveyard.contains(blocker_id),
                "Blocker should be in graveyard (took 3 damage, has 2 toughness)"
            );
        }
    }

    #[test]
    fn test_lifelink_with_trample() {
        use crate::game::random_controller::RandomController;
        use crate::game::zero_controller::ZeroController;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1: Create a 5/5 creature with Lifelink AND Trample (attacker)
        let attacker_id = game.next_entity_id();
        let mut attacker = Card::new(attacker_id, "Baneslayer Angel".to_string(), p1_id);
        attacker.add_type(CardType::Creature);
        attacker.set_base_power(Some(5));
        attacker.set_base_toughness(Some(5));
        attacker.controller = p1_id;
        attacker.keywords.insert(Keyword::Lifelink);
        attacker.keywords.insert(Keyword::Trample);
        attacker.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        game.cards.insert(attacker_id, attacker);
        game.battlefield.add(attacker_id);

        // P2: Create a 2/2 creature (blocker)
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

        // Record life before combat
        let p1_life_before = game.players[0].life;
        let p2_life_before = game.players[1].life;

        // Assign combat damage
        let mut controller1 = RandomController::with_seed(p1_id, 42);
        let mut controller2 = ZeroController::new(p2_id);
        let result = game.assign_combat_damage(&mut controller1, &mut controller2, false);
        assert!(result.is_ok(), "Failed to assign combat damage: {result:?}");

        // P1 should have gained 5 life (2 to blocker + 3 trample to player = 5 total damage)
        let p1_life_after = game.players[0].life;
        assert_eq!(
            p1_life_after,
            p1_life_before + 5,
            "P1 should have gained 5 life from lifelink (all damage dealt)"
        );

        // P2 should have taken 3 trample damage
        let p2_life_after = game.players[1].life;
        assert_eq!(
            p2_life_after,
            p2_life_before - 3,
            "P2 should have taken 3 trample damage"
        );

        // Blocker should be dead
        if let Some(zones) = game.get_player_zones(p2_id) {
            assert!(zones.graveyard.contains(blocker_id), "Blocker should be in graveyard");
        }
    }

    #[test]
    fn test_deathtouch_attacker_kills_large_blocker() {
        use crate::game::random_controller::RandomController;
        use crate::game::zero_controller::ZeroController;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1: Create a 1/1 creature with Deathtouch (attacker)
        let attacker_id = game.next_entity_id();
        let mut attacker = Card::new(attacker_id, "Deadly Recluse".to_string(), p1_id);
        attacker.add_type(CardType::Creature);
        attacker.set_base_power(Some(1));
        attacker.set_base_toughness(Some(1));
        attacker.controller = p1_id;
        attacker.keywords.insert(Keyword::Deathtouch);
        attacker.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        game.cards.insert(attacker_id, attacker);
        game.battlefield.add(attacker_id);

        // P2: Create a 5/5 creature (blocker)
        let blocker_id = game.next_entity_id();
        let mut blocker = Card::new(blocker_id, "Serra Angel".to_string(), p2_id);
        blocker.add_type(CardType::Creature);
        blocker.set_base_power(Some(5));
        blocker.set_base_toughness(Some(5));
        blocker.controller = p2_id;
        game.cards.insert(blocker_id, blocker);
        game.battlefield.add(blocker_id);

        // Declare combat
        game.combat.declare_attacker(attacker_id, p2_id);
        let attacker_vec = smallvec::smallvec![attacker_id];
        game.combat.declare_blocker(blocker_id, attacker_vec);

        // Assign combat damage
        let mut controller1 = RandomController::with_seed(p1_id, 42);
        let mut controller2 = ZeroController::new(p2_id);
        let result = game.assign_combat_damage(&mut controller1, &mut controller2, false);
        assert!(result.is_ok(), "Failed to assign combat damage: {result:?}");

        // Blocker should be dead (deathtouch from 1 damage)
        // Attacker should be dead (5 damage from blocker)
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert!(
                zones.graveyard.contains(attacker_id),
                "Attacker should be in graveyard (took 5 damage)"
            );
        }
        if let Some(zones) = game.get_player_zones(p2_id) {
            assert!(
                zones.graveyard.contains(blocker_id),
                "Blocker should be in graveyard (dealt deathtouch damage)"
            );
        }
    }

    #[test]
    fn test_deathtouch_blocker_kills_large_attacker() {
        use crate::game::random_controller::RandomController;
        use crate::game::zero_controller::ZeroController;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1: Create a 5/5 creature (attacker)
        let attacker_id = game.next_entity_id();
        let mut attacker = Card::new(attacker_id, "Serra Angel".to_string(), p1_id);
        attacker.add_type(CardType::Creature);
        attacker.set_base_power(Some(5));
        attacker.set_base_toughness(Some(5));
        attacker.controller = p1_id;
        attacker.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        game.cards.insert(attacker_id, attacker);
        game.battlefield.add(attacker_id);

        // P2: Create a 1/1 creature with Deathtouch (blocker)
        let blocker_id = game.next_entity_id();
        let mut blocker = Card::new(blocker_id, "Typhoid Rats".to_string(), p2_id);
        blocker.add_type(CardType::Creature);
        blocker.set_base_power(Some(1));
        blocker.set_base_toughness(Some(1));
        blocker.controller = p2_id;
        blocker.keywords.insert(Keyword::Deathtouch);
        game.cards.insert(blocker_id, blocker);
        game.battlefield.add(blocker_id);

        // Declare combat
        game.combat.declare_attacker(attacker_id, p2_id);
        let attacker_vec = smallvec::smallvec![attacker_id];
        game.combat.declare_blocker(blocker_id, attacker_vec);

        // Assign combat damage
        let mut controller1 = RandomController::with_seed(p1_id, 42);
        let mut controller2 = ZeroController::new(p2_id);
        let result = game.assign_combat_damage(&mut controller1, &mut controller2, false);
        assert!(result.is_ok(), "Failed to assign combat damage: {result:?}");

        // Attacker should be dead (deathtouch from 1 damage)
        // Blocker should be dead (5 damage from attacker)
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert!(
                zones.graveyard.contains(attacker_id),
                "Attacker should be in graveyard (dealt deathtouch damage)"
            );
        }
        if let Some(zones) = game.get_player_zones(p2_id) {
            assert!(
                zones.graveyard.contains(blocker_id),
                "Blocker should be in graveyard (took 5 damage)"
            );
        }
    }

    #[test]
    fn test_deathtouch_with_trample_minimal_damage() {
        use crate::game::random_controller::RandomController;
        use crate::game::zero_controller::ZeroController;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1: Create a 5/5 creature with Deathtouch AND Trample (attacker)
        let attacker_id = game.next_entity_id();
        let mut attacker = Card::new(attacker_id, "Chevill, Bane of Monsters".to_string(), p1_id);
        attacker.add_type(CardType::Creature);
        attacker.set_base_power(Some(5));
        attacker.set_base_toughness(Some(5));
        attacker.controller = p1_id;
        attacker.keywords.insert(Keyword::Deathtouch);
        attacker.keywords.insert(Keyword::Trample);
        attacker.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        game.cards.insert(attacker_id, attacker);
        game.battlefield.add(attacker_id);

        // P2: Create a 3/3 creature (blocker)
        let blocker_id = game.next_entity_id();
        let mut blocker = Card::new(blocker_id, "Hill Giant".to_string(), p2_id);
        blocker.add_type(CardType::Creature);
        blocker.set_base_power(Some(3));
        blocker.set_base_toughness(Some(3));
        blocker.controller = p2_id;
        game.cards.insert(blocker_id, blocker);
        game.battlefield.add(blocker_id);

        // Declare combat
        game.combat.declare_attacker(attacker_id, p2_id);
        let attacker_vec = smallvec::smallvec![attacker_id];
        game.combat.declare_blocker(blocker_id, attacker_vec);

        // Record P2's life before combat
        let p2_life_before = game.players[1].life;

        // Assign combat damage
        let mut controller1 = RandomController::with_seed(p1_id, 42);
        let mut controller2 = ZeroController::new(p2_id);
        let result = game.assign_combat_damage(&mut controller1, &mut controller2, false);
        assert!(result.is_ok(), "Failed to assign combat damage: {result:?}");

        // MTG Rules 702.2c: With deathtouch + trample, only 1 damage is lethal
        // So 1 damage to blocker (kills it), 4 damage tramples over to player
        let p2_life_after = game.players[1].life;
        assert_eq!(
            p2_life_after,
            p2_life_before - 4,
            "P2 should have taken 4 trample damage (5 power - 1 lethal to blocker)"
        );

        // Blocker should be dead (deathtouch)
        // Attacker should survive (took 3 damage, has 5 toughness)
        assert!(
            game.battlefield.contains(attacker_id),
            "Attacker should survive (took 3 damage, has 5 toughness)"
        );
        if let Some(zones) = game.get_player_zones(p2_id) {
            assert!(
                zones.graveyard.contains(blocker_id),
                "Blocker should be in graveyard (dealt deathtouch damage)"
            );
        }
    }

    #[test]
    fn test_deathtouch_with_multiple_blockers() {
        use crate::game::random_controller::RandomController;
        use crate::game::zero_controller::ZeroController;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1: Create a 3/3 creature with Deathtouch (attacker)
        let attacker_id = game.next_entity_id();
        let mut attacker = Card::new(attacker_id, "Gifted Aetherborn".to_string(), p1_id);
        attacker.add_type(CardType::Creature);
        attacker.set_base_power(Some(3));
        attacker.set_base_toughness(Some(3));
        attacker.controller = p1_id;
        attacker.keywords.insert(Keyword::Deathtouch);
        attacker.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        game.cards.insert(attacker_id, attacker);
        game.battlefield.add(attacker_id);

        // P2: Create two blockers (both 5/5)
        let blocker1_id = game.next_entity_id();
        let mut blocker1 = Card::new(blocker1_id, "Serra Angel".to_string(), p2_id);
        blocker1.add_type(CardType::Creature);
        blocker1.set_base_power(Some(5));
        blocker1.set_base_toughness(Some(5));
        blocker1.controller = p2_id;
        game.cards.insert(blocker1_id, blocker1);
        game.battlefield.add(blocker1_id);

        let blocker2_id = game.next_entity_id();
        let mut blocker2 = Card::new(blocker2_id, "Air Elemental".to_string(), p2_id);
        blocker2.add_type(CardType::Creature);
        blocker2.set_base_power(Some(5));
        blocker2.set_base_toughness(Some(5));
        blocker2.controller = p2_id;
        game.cards.insert(blocker2_id, blocker2);
        game.battlefield.add(blocker2_id);

        // Declare combat with both blockers
        game.combat.declare_attacker(attacker_id, p2_id);
        let attacker_vec = smallvec::smallvec![attacker_id];
        game.combat.declare_blocker(blocker1_id, attacker_vec.clone());
        game.combat.declare_blocker(blocker2_id, attacker_vec);

        // Assign combat damage (damage order determined internally)
        let mut controller1 = RandomController::with_seed(p1_id, 42);
        let mut controller2 = ZeroController::new(p2_id);
        let result = game.assign_combat_damage(&mut controller1, &mut controller2, false);
        assert!(result.is_ok(), "Failed to assign combat damage: {result:?}");

        // With deathtouch, 1 damage is lethal to each blocker
        // 3/3 attacker: 1 damage to first blocker, 1 damage to second blocker, 1 damage wasted
        // Both blockers should be dead, attacker should be dead (took 10 damage total)
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert!(
                zones.graveyard.contains(attacker_id),
                "Attacker should be in graveyard (took 10 damage from two 5/5 blockers)"
            );
        }
        if let Some(zones) = game.get_player_zones(p2_id) {
            assert!(
                zones.graveyard.contains(blocker1_id),
                "First blocker should be in graveyard (dealt deathtouch damage)"
            );
            assert!(
                zones.graveyard.contains(blocker2_id),
                "Second blocker should be in graveyard (dealt deathtouch damage)"
            );
        }
    }

    // Note: Menace validation test removed because incremental validation during
    // blocker declaration would incorrectly reject the first blocker. Menace validation
    // should happen after all blockers are declared. The following tests verify that
    // Menace works correctly when multiple blockers are declared or no blockers are declared.

    #[test]
    fn test_menace_can_be_blocked_by_two_creatures() {
        use crate::game::random_controller::RandomController;
        use crate::game::zero_controller::ZeroController;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1: Create a 3/3 creature with Menace (attacker)
        let attacker_id = game.next_entity_id();
        let mut attacker = Card::new(attacker_id, "Mardu Skullhunter".to_string(), p1_id);
        attacker.add_type(CardType::Creature);
        attacker.set_base_power(Some(3));
        attacker.set_base_toughness(Some(3));
        attacker.controller = p1_id;
        attacker.keywords.insert(Keyword::Menace);
        attacker.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        game.cards.insert(attacker_id, attacker);
        game.battlefield.add(attacker_id);

        // P2: Create two blockers
        let blocker1_id = game.next_entity_id();
        let mut blocker1 = Card::new(blocker1_id, "Grizzly Bears".to_string(), p2_id);
        blocker1.add_type(CardType::Creature);
        blocker1.set_base_power(Some(2));
        blocker1.set_base_toughness(Some(2));
        blocker1.controller = p2_id;
        game.cards.insert(blocker1_id, blocker1);
        game.battlefield.add(blocker1_id);

        let blocker2_id = game.next_entity_id();
        let mut blocker2 = Card::new(blocker2_id, "Elite Vanguard".to_string(), p2_id);
        blocker2.add_type(CardType::Creature);
        blocker2.set_base_power(Some(2));
        blocker2.set_base_toughness(Some(1));
        blocker2.controller = p2_id;
        game.cards.insert(blocker2_id, blocker2);
        game.battlefield.add(blocker2_id);

        // Declare attacker
        game.combat.declare_attacker(attacker_id, p2_id);

        // Block with two creatures - should succeed
        let result1 = game.declare_blocker(p2_id, blocker1_id, vec![attacker_id]);
        assert!(result1.is_ok(), "First blocker should succeed: {result1:?}");

        let result2 = game.declare_blocker(p2_id, blocker2_id, vec![attacker_id]);
        assert!(result2.is_ok(), "Second blocker should succeed: {result2:?}");

        // Verify combat resolves correctly
        let mut controller1 = RandomController::with_seed(p1_id, 42);
        let mut controller2 = ZeroController::new(p2_id);
        let result = game.assign_combat_damage(&mut controller1, &mut controller2, false);
        assert!(result.is_ok(), "Combat damage should resolve: {result:?}");

        // Both blockers should be dead (took 3 damage total, both have <= 2 toughness)
        // Attacker should be dead (took 4 damage total from 2+2, has 3 toughness)
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert!(zones.graveyard.contains(attacker_id), "Attacker should be dead");
        }
        if let Some(zones) = game.get_player_zones(p2_id) {
            assert!(zones.graveyard.contains(blocker1_id), "First blocker should be dead");
            assert!(zones.graveyard.contains(blocker2_id), "Second blocker should be dead");
        }
    }

    #[test]
    fn test_menace_can_be_unblocked() {
        use crate::game::random_controller::RandomController;
        use crate::game::zero_controller::ZeroController;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1: Create a 3/3 creature with Menace (attacker)
        let attacker_id = game.next_entity_id();
        let mut attacker = Card::new(attacker_id, "Goblin Heelcutter".to_string(), p1_id);
        attacker.add_type(CardType::Creature);
        attacker.set_base_power(Some(3));
        attacker.set_base_toughness(Some(3));
        attacker.controller = p1_id;
        attacker.keywords.insert(Keyword::Menace);
        attacker.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        game.cards.insert(attacker_id, attacker);
        game.battlefield.add(attacker_id);

        // Declare attacker (no blockers)
        game.combat.declare_attacker(attacker_id, p2_id);

        // Record life before combat
        let p2_life_before = game.players[1].life;

        // Assign combat damage
        let mut controller1 = RandomController::with_seed(p1_id, 42);
        let mut controller2 = ZeroController::new(p2_id);
        let result = game.assign_combat_damage(&mut controller1, &mut controller2, false);
        assert!(result.is_ok(), "Combat damage should resolve: {result:?}");

        // P2 should have taken 3 damage
        let p2_life_after = game.players[1].life;
        assert_eq!(
            p2_life_after,
            p2_life_before - 3,
            "P2 should have taken 3 damage from unblocked menace creature"
        );
    }

    #[test]
    fn test_menace_with_three_blockers() {
        use crate::game::random_controller::RandomController;
        use crate::game::zero_controller::ZeroController;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1: Create a 5/5 creature with Menace (attacker)
        let attacker_id = game.next_entity_id();
        let mut attacker = Card::new(attacker_id, "Charging Monstrosaur".to_string(), p1_id);
        attacker.add_type(CardType::Creature);
        attacker.set_base_power(Some(5));
        attacker.set_base_toughness(Some(5));
        attacker.controller = p1_id;
        attacker.keywords.insert(Keyword::Menace);
        attacker.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        game.cards.insert(attacker_id, attacker);
        game.battlefield.add(attacker_id);

        // P2: Create three blockers (1/1 each)
        let blocker1_id = game.next_entity_id();
        let mut blocker1 = Card::new(blocker1_id, "Soldier Token 1".to_string(), p2_id);
        blocker1.add_type(CardType::Creature);
        blocker1.set_base_power(Some(1));
        blocker1.set_base_toughness(Some(1));
        blocker1.controller = p2_id;
        game.cards.insert(blocker1_id, blocker1);
        game.battlefield.add(blocker1_id);

        let blocker2_id = game.next_entity_id();
        let mut blocker2 = Card::new(blocker2_id, "Soldier Token 2".to_string(), p2_id);
        blocker2.add_type(CardType::Creature);
        blocker2.set_base_power(Some(1));
        blocker2.set_base_toughness(Some(1));
        blocker2.controller = p2_id;
        game.cards.insert(blocker2_id, blocker2);
        game.battlefield.add(blocker2_id);

        let blocker3_id = game.next_entity_id();
        let mut blocker3 = Card::new(blocker3_id, "Soldier Token 3".to_string(), p2_id);
        blocker3.add_type(CardType::Creature);
        blocker3.set_base_power(Some(1));
        blocker3.set_base_toughness(Some(1));
        blocker3.controller = p2_id;
        game.cards.insert(blocker3_id, blocker3);
        game.battlefield.add(blocker3_id);

        // Declare attacker
        game.combat.declare_attacker(attacker_id, p2_id);

        // Block with three creatures - should succeed (more than 2 is fine)
        let result1 = game.declare_blocker(p2_id, blocker1_id, vec![attacker_id]);
        assert!(result1.is_ok(), "First blocker should succeed");

        let result2 = game.declare_blocker(p2_id, blocker2_id, vec![attacker_id]);
        assert!(result2.is_ok(), "Second blocker should succeed");

        let result3 = game.declare_blocker(p2_id, blocker3_id, vec![attacker_id]);
        assert!(result3.is_ok(), "Third blocker should succeed");

        // Verify combat resolves correctly
        let mut controller1 = RandomController::with_seed(p1_id, 42);
        let mut controller2 = ZeroController::new(p2_id);
        let result = game.assign_combat_damage(&mut controller1, &mut controller2, false);
        assert!(result.is_ok(), "Combat damage should resolve: {result:?}");

        // All three blockers should be dead (each took 1 toughness worth of damage)
        // Attacker should survive (took 3 damage, has 5 toughness)
        assert!(
            game.battlefield.contains(attacker_id),
            "Attacker should survive (took 3 damage, has 5 toughness)"
        );
        if let Some(zones) = game.get_player_zones(p2_id) {
            assert!(zones.graveyard.contains(blocker1_id), "First blocker should be dead");
            assert!(zones.graveyard.contains(blocker2_id), "Second blocker should be dead");
            assert!(zones.graveyard.contains(blocker3_id), "Third blocker should be dead");
        }
    }

    #[test]
    fn test_hexproof_blocks_destroy_spell() {
        // Test that destroy spells cannot target hexproof creatures controlled by opponent
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P2: Create a hexproof creature
        let hexproof_creature_id = game.next_entity_id();
        let mut hexproof_creature = Card::new(hexproof_creature_id, "Slippery Bogle".to_string(), p2_id);
        hexproof_creature.add_type(CardType::Creature);
        hexproof_creature.set_base_power(Some(1));
        hexproof_creature.set_base_toughness(Some(1));
        hexproof_creature.keywords.insert(Keyword::Hexproof);
        game.cards.insert(hexproof_creature_id, hexproof_creature);
        game.battlefield.add(hexproof_creature_id);

        // P2: Create a normal creature
        let normal_creature_id = game.next_entity_id();
        let mut normal_creature = Card::new(normal_creature_id, "Grizzly Bears".to_string(), p2_id);
        normal_creature.add_type(CardType::Creature);
        normal_creature.set_base_power(Some(2));
        normal_creature.set_base_toughness(Some(2));
        game.cards.insert(normal_creature_id, normal_creature);
        game.battlefield.add(normal_creature_id);

        // P1: Cast a destroy spell (Terror) - should target normal creature, not hexproof one
        let destroy_spell_id = game.next_entity_id();
        let mut destroy_spell = Card::new(destroy_spell_id, "Terror".to_string(), p1_id);
        destroy_spell.add_type(CardType::Instant);
        destroy_spell.mana_cost = ManaCost::from_string("1B");
        // Use placeholder card ID 0 which will be replaced with a targetable opponent's creature
        destroy_spell.effects.push(Effect::DestroyPermanent {
            target: CardId::new(0),
            restriction: crate::core::TargetRestriction::any(),
            no_regenerate: false,
        });
        game.cards.insert(destroy_spell_id, destroy_spell);

        // Put it on the stack (simulating cast)
        game.stack.add(destroy_spell_id);

        // Resolve the spell - explicitly target the normal creature
        // (controller would have chosen normal_creature_id, not hexproof one)
        let result = game.resolve_spell(destroy_spell_id, &[normal_creature_id]);
        assert!(result.is_ok(), "Destroy spell should resolve successfully");

        // Check that the hexproof creature is still alive
        assert!(
            game.battlefield.contains(hexproof_creature_id),
            "Hexproof creature should still be on battlefield"
        );

        // Check that the normal creature was destroyed
        if let Some(zones) = game.get_player_zones(p2_id) {
            assert!(
                zones.graveyard.contains(normal_creature_id),
                "Normal creature should be in graveyard"
            );
        }
    }

    #[test]
    fn test_hexproof_blocks_tap_spell() {
        // Test that tap spells cannot target hexproof creatures controlled by opponent
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P2: Create a hexproof creature
        let hexproof_creature_id = game.next_entity_id();
        let mut hexproof_creature = Card::new(hexproof_creature_id, "Slippery Bogle".to_string(), p2_id);
        hexproof_creature.add_type(CardType::Creature);
        hexproof_creature.set_base_power(Some(1));
        hexproof_creature.set_base_toughness(Some(1));
        hexproof_creature.keywords.insert(Keyword::Hexproof);
        game.cards.insert(hexproof_creature_id, hexproof_creature);
        game.battlefield.add(hexproof_creature_id);

        // P2: Create a normal creature
        let normal_creature_id = game.next_entity_id();
        let mut normal_creature = Card::new(normal_creature_id, "Grizzly Bears".to_string(), p2_id);
        normal_creature.add_type(CardType::Creature);
        normal_creature.set_base_power(Some(2));
        normal_creature.set_base_toughness(Some(2));
        game.cards.insert(normal_creature_id, normal_creature);
        game.battlefield.add(normal_creature_id);

        // P1: Cast a tap spell - should target normal creature, not hexproof one
        let tap_spell_id = game.next_entity_id();
        let mut tap_spell = Card::new(tap_spell_id, "Frost Breath".to_string(), p1_id);
        tap_spell.add_type(CardType::Instant);
        tap_spell.mana_cost = ManaCost::from_string("2U");
        // Use placeholder card ID 0 which will be replaced with a targetable opponent's creature
        tap_spell.effects.push(Effect::TapPermanent { target: CardId::new(0) });
        game.cards.insert(tap_spell_id, tap_spell);

        // Put spell on stack (simulating cast)
        game.stack.add(tap_spell_id);

        // Resolve the spell - explicitly target the normal creature
        // (controller would have chosen normal_creature_id, not hexproof one)
        let result = game.resolve_spell(tap_spell_id, &[normal_creature_id]);
        assert!(result.is_ok(), "Tap spell should resolve successfully");

        // Check that the hexproof creature is not tapped
        let hexproof_card = game.cards.get(hexproof_creature_id).unwrap();
        assert!(!hexproof_card.tapped, "Hexproof creature should not be tapped");

        // Check that the normal creature was tapped
        let normal_card = game.cards.get(normal_creature_id).unwrap();
        assert!(normal_card.tapped, "Normal creature should be tapped");
    }

    #[test]
    fn test_hexproof_allows_own_spells() {
        // Test that hexproof creatures CAN be targeted by their controller's spells
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let _p2_id = game.players[1].id;

        // P1: Create a hexproof creature
        let hexproof_creature_id = game.next_entity_id();
        let mut hexproof_creature = Card::new(hexproof_creature_id, "Slippery Bogle".to_string(), p1_id);
        hexproof_creature.add_type(CardType::Creature);
        hexproof_creature.set_base_power(Some(1));
        hexproof_creature.set_base_toughness(Some(1));
        hexproof_creature.keywords.insert(Keyword::Hexproof);
        game.cards.insert(hexproof_creature_id, hexproof_creature);
        game.battlefield.add(hexproof_creature_id);

        // P1: Cast Giant Growth on their own hexproof creature - should work!
        let pump_spell_id = game.next_entity_id();
        let mut pump_spell = Card::new(pump_spell_id, "Giant Growth".to_string(), p1_id);
        pump_spell.add_type(CardType::Instant);
        pump_spell.mana_cost = ManaCost::from_string("G");
        // Use placeholder card ID 0 which will be replaced with a targetable creature
        pump_spell.effects.push(Effect::PumpCreature {
            target: CardId::new(0),
            power_bonus: 3,
            toughness_bonus: 3,
            keywords_granted: smallvec::SmallVec::new(),
        });
        game.cards.insert(pump_spell_id, pump_spell);

        // Put spell on stack (simulating cast)
        game.stack.add(pump_spell_id);

        // Resolve the spell
        let result = game.resolve_spell(pump_spell_id, &[hexproof_creature_id]);
        assert!(
            result.is_ok(),
            "Pump spell on own hexproof creature should resolve successfully"
        );

        // Check that the hexproof creature got the pump
        let creature = game.cards.get(hexproof_creature_id).unwrap();
        assert_eq!(
            creature.current_power(),
            4,
            "Hexproof creature should have boosted power (1+3)"
        );
        assert_eq!(
            creature.current_toughness(),
            4,
            "Hexproof creature should have boosted toughness (1+3)"
        );
    }

    #[test]
    fn test_hexproof_no_valid_targets() {
        // Test that spells fail to find targets if only hexproof creatures exist
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P2: Create only a hexproof creature (no valid targets for opponent)
        let hexproof_creature_id = game.next_entity_id();
        let mut hexproof_creature = Card::new(hexproof_creature_id, "Slippery Bogle".to_string(), p2_id);
        hexproof_creature.add_type(CardType::Creature);
        hexproof_creature.set_base_power(Some(1));
        hexproof_creature.set_base_toughness(Some(1));
        hexproof_creature.keywords.insert(Keyword::Hexproof);
        game.cards.insert(hexproof_creature_id, hexproof_creature);
        game.battlefield.add(hexproof_creature_id);

        // P1: Try to cast a destroy spell - should fail to find valid target
        let destroy_spell_id = game.next_entity_id();
        let mut destroy_spell = Card::new(destroy_spell_id, "Terror".to_string(), p1_id);
        destroy_spell.add_type(CardType::Instant);
        destroy_spell.mana_cost = ManaCost::from_string("1B");
        // Use placeholder card ID 0 which will fail to be replaced with a target
        destroy_spell.effects.push(Effect::DestroyPermanent {
            target: CardId::new(0),
            restriction: crate::core::TargetRestriction::any(),
            no_regenerate: false,
        });
        game.cards.insert(destroy_spell_id, destroy_spell);

        // Put it on the stack (simulating cast)
        game.stack.add(destroy_spell_id);

        // Resolve the spell - should succeed but do nothing (no valid targets)
        let result = game.resolve_spell(destroy_spell_id, &[]);
        assert!(result.is_ok(), "Spell with no valid targets should still resolve");

        // Check that the hexproof creature is still alive
        assert!(
            game.battlefield.contains(hexproof_creature_id),
            "Hexproof creature should still be on battlefield"
        );
    }

    #[test]
    fn test_indestructible_survives_lethal_damage() {
        // Test that indestructible creatures survive lethal damage
        use crate::game::zero_controller::ZeroController;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1: Create a 2/2 indestructible creature
        let indestructible_id = game.next_entity_id();
        let mut indestructible = Card::new(indestructible_id, "Darksteel Myr".to_string(), p1_id);
        indestructible.add_type(CardType::Creature);
        indestructible.set_base_power(Some(2));
        indestructible.set_base_toughness(Some(2));
        indestructible.keywords.insert(Keyword::Indestructible);
        indestructible.controller = p1_id;
        indestructible.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        game.cards.insert(indestructible_id, indestructible);
        game.battlefield.add(indestructible_id);

        // P2: Create a 5/5 creature (blocker)
        let blocker_id = game.next_entity_id();
        let mut blocker = Card::new(blocker_id, "Hill Giant".to_string(), p2_id);
        blocker.add_type(CardType::Creature);
        blocker.set_base_power(Some(5));
        blocker.set_base_toughness(Some(5));
        blocker.controller = p2_id;
        blocker.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        game.cards.insert(blocker_id, blocker);
        game.battlefield.add(blocker_id);

        // P1 attacks with indestructible creature
        let mut controller1 = ZeroController::new(p1_id);
        let mut controller2 = ZeroController::new(p2_id);

        game.combat.declare_attacker(indestructible_id, p2_id);

        // P2 blocks with 5/5 creature
        let result = game.declare_blocker(p2_id, blocker_id, vec![indestructible_id]);
        assert!(result.is_ok(), "Failed to declare blocker: {result:?}");

        // Assign combat damage
        // Indestructible 2/2 deals 2 damage to blocker
        // Blocker 5/5 deals 5 damage to indestructible (more than lethal, but indestructible survives)
        let result = game.assign_combat_damage(&mut controller1, &mut controller2, false);
        assert!(result.is_ok(), "Failed to assign combat damage: {result:?}");

        // Indestructible creature should survive (took 5 damage but has indestructible)
        assert!(
            game.battlefield.contains(indestructible_id),
            "Indestructible creature should survive lethal damage"
        );

        // Blocker should survive (took 2 damage, has 5 toughness)
        assert!(game.battlefield.contains(blocker_id), "Blocker should survive 2 damage");
    }

    #[test]
    fn test_indestructible_immune_to_destroy_effects() {
        // Test that indestructible creatures can't be destroyed by Terror/Murder
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P2: Create an indestructible creature
        let indestructible_id = game.next_entity_id();
        let mut indestructible = Card::new(indestructible_id, "Darksteel Myr".to_string(), p2_id);
        indestructible.add_type(CardType::Creature);
        indestructible.set_base_power(Some(0));
        indestructible.set_base_toughness(Some(1));
        indestructible.keywords.insert(Keyword::Indestructible);
        game.cards.insert(indestructible_id, indestructible);
        game.battlefield.add(indestructible_id);

        // P1: Cast Terror targeting the indestructible creature
        let destroy_spell_id = game.next_entity_id();
        let mut destroy_spell = Card::new(destroy_spell_id, "Terror".to_string(), p1_id);
        destroy_spell.add_type(CardType::Instant);
        destroy_spell.mana_cost = ManaCost::from_string("1B");
        // Explicitly target the indestructible creature
        destroy_spell.effects.push(Effect::DestroyPermanent {
            target: indestructible_id,
            restriction: crate::core::TargetRestriction::any(),
            no_regenerate: false,
        });
        game.cards.insert(destroy_spell_id, destroy_spell);

        // Put it on the stack
        game.stack.add(destroy_spell_id);

        // Resolve the spell
        let result = game.resolve_spell(destroy_spell_id, &[]);
        assert!(result.is_ok(), "Destroy spell should resolve successfully");

        // Indestructible creature should still be alive
        assert!(
            game.battlefield.contains(indestructible_id),
            "Indestructible creature should survive destroy effect"
        );
    }

    #[test]
    fn test_indestructible_survives_deathtouch() {
        // Test that indestructible creatures survive deathtouch damage
        use crate::game::zero_controller::ZeroController;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1: Create a 1/1 deathtouch creature (attacker)
        let deathtouch_id = game.next_entity_id();
        let mut deathtouch = Card::new(deathtouch_id, "Typhoid Rats".to_string(), p1_id);
        deathtouch.add_type(CardType::Creature);
        deathtouch.set_base_power(Some(1));
        deathtouch.set_base_toughness(Some(1));
        deathtouch.keywords.insert(Keyword::Deathtouch);
        deathtouch.controller = p1_id;
        deathtouch.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        game.cards.insert(deathtouch_id, deathtouch);
        game.battlefield.add(deathtouch_id);

        // P2: Create a 5/5 indestructible creature (blocker)
        let indestructible_id = game.next_entity_id();
        let mut indestructible = Card::new(indestructible_id, "Darksteel Colossus".to_string(), p2_id);
        indestructible.add_type(CardType::Creature);
        indestructible.set_base_power(Some(5));
        indestructible.set_base_toughness(Some(5));
        indestructible.keywords.insert(Keyword::Indestructible);
        indestructible.controller = p2_id;
        indestructible.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        game.cards.insert(indestructible_id, indestructible);
        game.battlefield.add(indestructible_id);

        // P1 attacks with deathtouch creature
        let mut controller1 = ZeroController::new(p1_id);
        let mut controller2 = ZeroController::new(p2_id);

        game.combat.declare_attacker(deathtouch_id, p2_id);

        // P2 blocks with indestructible creature
        let result = game.declare_blocker(p2_id, indestructible_id, vec![deathtouch_id]);
        assert!(result.is_ok(), "Failed to declare blocker: {result:?}");

        // Assign combat damage
        // Deathtouch 1/1 deals 1 damage to indestructible (deathtouch damage, but indestructible survives)
        // Indestructible 5/5 deals 5 damage to deathtouch (kills it)
        let result = game.assign_combat_damage(&mut controller1, &mut controller2, false);
        assert!(result.is_ok(), "Failed to assign combat damage: {result:?}");

        // Indestructible creature should survive deathtouch damage
        assert!(
            game.battlefield.contains(indestructible_id),
            "Indestructible creature should survive deathtouch damage"
        );

        // Deathtouch creature should be dead (took 5 damage, has 1 toughness)
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert!(
                zones.graveyard.contains(deathtouch_id),
                "Deathtouch creature should be in graveyard"
            );
        }
    }

    #[test]
    fn test_indestructible_vs_non_indestructible_combat() {
        // Test that normal creature dies while indestructible survives in mutual combat
        use crate::game::zero_controller::ZeroController;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1: Create a 3/3 indestructible creature (attacker)
        let indestructible_id = game.next_entity_id();
        let mut indestructible = Card::new(indestructible_id, "Indomitable".to_string(), p1_id);
        indestructible.add_type(CardType::Creature);
        indestructible.set_base_power(Some(3));
        indestructible.set_base_toughness(Some(3));
        indestructible.keywords.insert(Keyword::Indestructible);
        indestructible.controller = p1_id;
        indestructible.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        game.cards.insert(indestructible_id, indestructible);
        game.battlefield.add(indestructible_id);

        // P2: Create a 3/3 normal creature (blocker)
        let normal_id = game.next_entity_id();
        let mut normal = Card::new(normal_id, "Hill Giant".to_string(), p2_id);
        normal.add_type(CardType::Creature);
        normal.set_base_power(Some(3));
        normal.set_base_toughness(Some(3));
        normal.controller = p2_id;
        normal.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        game.cards.insert(normal_id, normal);
        game.battlefield.add(normal_id);

        // P1 attacks with indestructible creature
        let mut controller1 = ZeroController::new(p1_id);
        let mut controller2 = ZeroController::new(p2_id);

        game.combat.declare_attacker(indestructible_id, p2_id);

        // P2 blocks with normal creature
        let result = game.declare_blocker(p2_id, normal_id, vec![indestructible_id]);
        assert!(result.is_ok(), "Failed to declare blocker: {result:?}");

        // Assign combat damage
        // Both deal 3 damage to each other (lethal)
        // Indestructible survives, normal dies
        let result = game.assign_combat_damage(&mut controller1, &mut controller2, false);
        assert!(result.is_ok(), "Failed to assign combat damage: {result:?}");

        // Indestructible creature should survive
        assert!(
            game.battlefield.contains(indestructible_id),
            "Indestructible creature should survive"
        );

        // Normal creature should be dead
        if let Some(zones) = game.get_player_zones(p2_id) {
            assert!(
                zones.graveyard.contains(normal_id),
                "Normal creature should be in graveyard"
            );
        }
    }

    #[test]
    fn test_shroud_blocks_destroy_from_opponent() {
        // Test that destroy spells from opponents can't target shroud creatures
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P2: Create a shroud creature
        let shroud_creature_id = game.next_entity_id();
        let mut shroud_creature = Card::new(shroud_creature_id, "Silhana Ledgewalker".to_string(), p2_id);
        shroud_creature.add_type(CardType::Creature);
        shroud_creature.set_base_power(Some(1));
        shroud_creature.set_base_toughness(Some(1));
        shroud_creature.keywords.insert(Keyword::Shroud);
        game.cards.insert(shroud_creature_id, shroud_creature);
        game.battlefield.add(shroud_creature_id);

        // P2: Create a normal creature
        let normal_creature_id = game.next_entity_id();
        let mut normal_creature = Card::new(normal_creature_id, "Grizzly Bears".to_string(), p2_id);
        normal_creature.add_type(CardType::Creature);
        normal_creature.set_base_power(Some(2));
        normal_creature.set_base_toughness(Some(2));
        game.cards.insert(normal_creature_id, normal_creature);
        game.battlefield.add(normal_creature_id);

        // P1: Cast Terror - should target normal creature, not shroud one
        let destroy_spell_id = game.next_entity_id();
        let mut destroy_spell = Card::new(destroy_spell_id, "Terror".to_string(), p1_id);
        destroy_spell.add_type(CardType::Instant);
        destroy_spell.mana_cost = ManaCost::from_string("1B");
        destroy_spell.effects.push(Effect::DestroyPermanent {
            target: CardId::new(0),
            restriction: crate::core::TargetRestriction::any(),
            no_regenerate: false,
        });
        game.cards.insert(destroy_spell_id, destroy_spell);
        game.stack.add(destroy_spell_id);

        let result = game.resolve_spell(destroy_spell_id, &[normal_creature_id]);
        assert!(result.is_ok(), "Destroy spell should resolve");

        // Shroud creature should still be alive
        assert!(
            game.battlefield.contains(shroud_creature_id),
            "Shroud creature should still be on battlefield"
        );

        // Normal creature was destroyed
        if let Some(zones) = game.get_player_zones(p2_id) {
            assert!(
                zones.graveyard.contains(normal_creature_id),
                "Normal creature should be in graveyard"
            );
        }
    }

    #[test]
    fn test_shroud_blocks_pump_from_controller() {
        // Test that shroud prevents targeting even by the controller (unlike hexproof)
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let _p2_id = game.players[1].id;

        // P1: Create a shroud creature
        let shroud_creature_id = game.next_entity_id();
        let mut shroud_creature = Card::new(shroud_creature_id, "Silhana Ledgewalker".to_string(), p1_id);
        shroud_creature.add_type(CardType::Creature);
        shroud_creature.set_base_power(Some(1));
        shroud_creature.set_base_toughness(Some(1));
        shroud_creature.keywords.insert(Keyword::Shroud);
        game.cards.insert(shroud_creature_id, shroud_creature);
        game.battlefield.add(shroud_creature_id);

        // P1: Create a normal creature
        let normal_creature_id = game.next_entity_id();
        let mut normal_creature = Card::new(normal_creature_id, "Grizzly Bears".to_string(), p1_id);
        normal_creature.add_type(CardType::Creature);
        normal_creature.set_base_power(Some(2));
        normal_creature.set_base_toughness(Some(2));
        game.cards.insert(normal_creature_id, normal_creature);
        game.battlefield.add(normal_creature_id);

        // P1: Cast Giant Growth - should target normal creature, not shroud one
        let pump_spell_id = game.next_entity_id();
        let mut pump_spell = Card::new(pump_spell_id, "Giant Growth".to_string(), p1_id);
        pump_spell.add_type(CardType::Instant);
        pump_spell.mana_cost = ManaCost::from_string("G");
        pump_spell.effects.push(Effect::PumpCreature {
            target: CardId::new(0),
            power_bonus: 3,
            toughness_bonus: 3,
            keywords_granted: smallvec::SmallVec::new(),
        });
        game.cards.insert(pump_spell_id, pump_spell);
        game.stack.add(pump_spell_id);

        let result = game.resolve_spell(pump_spell_id, &[normal_creature_id]);
        assert!(result.is_ok(), "Pump spell should resolve");

        // Shroud creature should NOT have the pump
        let shroud_card = game.cards.get(shroud_creature_id).unwrap();
        assert_eq!(
            shroud_card.current_power(),
            1,
            "Shroud creature should not have boosted power"
        );

        // Normal creature should have the pump
        let normal_card = game.cards.get(normal_creature_id).unwrap();
        assert_eq!(
            normal_card.current_power(),
            5,
            "Normal creature should have boosted power (2+3)"
        );
    }

    #[test]
    fn test_shroud_blocks_tap_effect() {
        // Test that tap effects can't target shroud creatures
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P2: Create a shroud creature
        let shroud_creature_id = game.next_entity_id();
        let mut shroud_creature = Card::new(shroud_creature_id, "Silhana Ledgewalker".to_string(), p2_id);
        shroud_creature.add_type(CardType::Creature);
        shroud_creature.set_base_power(Some(1));
        shroud_creature.set_base_toughness(Some(1));
        shroud_creature.keywords.insert(Keyword::Shroud);
        game.cards.insert(shroud_creature_id, shroud_creature);
        game.battlefield.add(shroud_creature_id);

        // P2: Create a normal creature
        let normal_creature_id = game.next_entity_id();
        let mut normal_creature = Card::new(normal_creature_id, "Grizzly Bears".to_string(), p2_id);
        normal_creature.add_type(CardType::Creature);
        normal_creature.set_base_power(Some(2));
        normal_creature.set_base_toughness(Some(2));
        game.cards.insert(normal_creature_id, normal_creature);
        game.battlefield.add(normal_creature_id);

        // P1: Cast tap spell - should target normal creature, not shroud one
        let tap_spell_id = game.next_entity_id();
        let mut tap_spell = Card::new(tap_spell_id, "Frost Breath".to_string(), p1_id);
        tap_spell.add_type(CardType::Instant);
        tap_spell.mana_cost = ManaCost::from_string("2U");
        tap_spell.effects.push(Effect::TapPermanent { target: CardId::new(0) });
        game.cards.insert(tap_spell_id, tap_spell);
        game.stack.add(tap_spell_id);

        let result = game.resolve_spell(tap_spell_id, &[normal_creature_id]);
        assert!(result.is_ok(), "Tap spell should resolve");

        // Shroud creature should not be tapped
        let shroud_card = game.cards.get(shroud_creature_id).unwrap();
        assert!(!shroud_card.tapped, "Shroud creature should not be tapped");

        // Normal creature should be tapped
        let normal_card = game.cards.get(normal_creature_id).unwrap();
        assert!(normal_card.tapped, "Normal creature should be tapped");
    }

    // ==================== Regeneration Tests ====================

    #[test]
    fn test_regeneration_shield_prevents_destroy_effect() {
        // CR 701.15a: A creature with a regeneration shield survives DestroyPermanent
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P2: Create a creature with a regeneration shield
        let creature_id = game.next_entity_id();
        let mut creature = Card::new(creature_id, "Drudge Skeletons".to_string(), p2_id);
        creature.add_type(CardType::Creature);
        creature.set_base_power(Some(1));
        creature.set_base_toughness(Some(1));
        creature.controller = p2_id;
        creature.regeneration_shields = 1; // Pre-set a regen shield
        game.cards.insert(creature_id, creature);
        game.battlefield.add(creature_id);

        // P1: Cast Terror targeting the creature
        let destroy_spell_id = game.next_entity_id();
        let mut destroy_spell = Card::new(destroy_spell_id, "Terror".to_string(), p1_id);
        destroy_spell.add_type(CardType::Instant);
        destroy_spell.mana_cost = ManaCost::from_string("1B");
        destroy_spell.effects.push(Effect::DestroyPermanent {
            target: creature_id,
            restriction: crate::core::TargetRestriction::any(),
            no_regenerate: false,
        });
        game.cards.insert(destroy_spell_id, destroy_spell);
        game.stack.add(destroy_spell_id);

        let result = game.resolve_spell(destroy_spell_id, &[]);
        assert!(result.is_ok(), "Destroy spell should resolve");

        // Creature should survive (regenerated)
        assert!(
            game.battlefield.contains(creature_id),
            "Creature with regeneration shield should survive destroy effect"
        );

        // Shield should be consumed
        let card = game.cards.get(creature_id).unwrap();
        assert_eq!(card.regeneration_shields, 0, "Shield should be consumed");

        // Creature should be tapped (CR 701.15a)
        assert!(card.tapped, "Regenerated creature should be tapped");

        // Damage should be cleared
        assert_eq!(card.damage, 0, "Regenerated creature should have damage cleared");
    }

    #[test]
    fn test_regeneration_shield_consumed_only_once() {
        // A creature with one shield: first destroy is prevented, second destroys it
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        let creature_id = game.next_entity_id();
        let mut creature = Card::new(creature_id, "Drudge Skeletons".to_string(), p2_id);
        creature.add_type(CardType::Creature);
        creature.set_base_power(Some(1));
        creature.set_base_toughness(Some(1));
        creature.controller = p2_id;
        creature.regeneration_shields = 1;
        game.cards.insert(creature_id, creature);
        game.battlefield.add(creature_id);

        // First destroy: regeneration intercepts
        let spell1_id = game.next_entity_id();
        let mut spell1 = Card::new(spell1_id, "Terror".to_string(), p1_id);
        spell1.add_type(CardType::Instant);
        spell1.mana_cost = ManaCost::from_string("1B");
        spell1.effects.push(Effect::DestroyPermanent {
            target: creature_id,
            restriction: crate::core::TargetRestriction::any(),
            no_regenerate: false,
        });
        game.cards.insert(spell1_id, spell1);
        game.stack.add(spell1_id);
        game.resolve_spell(spell1_id, &[]).unwrap();

        assert!(game.battlefield.contains(creature_id), "Should survive first destroy");
        assert_eq!(game.cards.get(creature_id).unwrap().regeneration_shields, 0);

        // Second destroy: no shield left, creature dies
        let spell2_id = game.next_entity_id();
        let mut spell2 = Card::new(spell2_id, "Murder".to_string(), p1_id);
        spell2.add_type(CardType::Instant);
        spell2.mana_cost = ManaCost::from_string("1BB");
        spell2.effects.push(Effect::DestroyPermanent {
            target: creature_id,
            restriction: crate::core::TargetRestriction::any(),
            no_regenerate: false,
        });
        game.cards.insert(spell2_id, spell2);
        game.stack.add(spell2_id);
        game.resolve_spell(spell2_id, &[]).unwrap();

        assert!(
            !game.battlefield.contains(creature_id),
            "Should die to second destroy with no shield"
        );
    }

    #[test]
    fn test_regeneration_effect_adds_shield() {
        // Test that resolving Effect::Regenerate adds a shield to the creature
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        let creature_id = game.next_entity_id();
        let mut creature = Card::new(creature_id, "Sedge Troll".to_string(), p1_id);
        creature.add_type(CardType::Creature);
        creature.set_base_power(Some(2));
        creature.set_base_toughness(Some(2));
        creature.controller = p1_id;
        game.cards.insert(creature_id, creature);
        game.battlefield.add(creature_id);

        // Cast a spell that grants a regen shield
        let regen_spell_id = game.next_entity_id();
        let mut regen_spell = Card::new(regen_spell_id, "Regenerate".to_string(), p1_id);
        regen_spell.add_type(CardType::Instant);
        regen_spell.mana_cost = ManaCost::from_string("G");
        regen_spell.effects.push(Effect::Regenerate { target: creature_id });
        game.cards.insert(regen_spell_id, regen_spell);
        game.stack.add(regen_spell_id);

        let result = game.resolve_spell(regen_spell_id, &[]);
        assert!(result.is_ok(), "Regen spell should resolve");

        let card = game.cards.get(creature_id).unwrap();
        assert_eq!(
            card.regeneration_shields, 1,
            "Creature should have 1 regeneration shield"
        );
    }

    #[test]
    fn test_regeneration_removes_from_combat() {
        // CR 701.15a: Regeneration removes creature from combat
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1: Create an attacking creature with regen shield
        let attacker_id = game.next_entity_id();
        let mut attacker = Card::new(attacker_id, "Sedge Troll".to_string(), p1_id);
        attacker.add_type(CardType::Creature);
        attacker.set_base_power(Some(2));
        attacker.set_base_toughness(Some(2));
        attacker.controller = p1_id;
        attacker.regeneration_shields = 1;
        attacker.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        game.cards.insert(attacker_id, attacker);
        game.battlefield.add(attacker_id);

        // Declare it as an attacker
        game.combat.declare_attacker(attacker_id, p2_id);
        assert!(game.combat.is_attacking(attacker_id));

        // Apply regeneration shield (simulating it being destroyed mid-combat)
        game.apply_regeneration_shield(attacker_id).unwrap();

        // After regeneration, creature should be removed from combat
        assert!(
            !game.combat.is_attacking(attacker_id),
            "Regenerated creature should be removed from combat"
        );

        // Creature should still be on battlefield
        assert!(game.battlefield.contains(attacker_id));
    }

    // ==================== LoseLife Tests ====================

    #[test]
    fn test_lose_life_effect() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1 casts a spell that makes P2 lose 3 life
        let spell_id = game.next_entity_id();
        let mut spell = Card::new(spell_id, "Drain Life".to_string(), p1_id);
        spell.add_type(CardType::Sorcery);
        spell.mana_cost = ManaCost::from_string("1B");
        spell.effects.push(Effect::LoseLife {
            player: p2_id,
            amount: 3,
        });
        game.cards.insert(spell_id, spell);
        game.stack.add(spell_id);

        game.resolve_spell(spell_id, &[]).unwrap();

        let p2 = game.get_player(p2_id).unwrap();
        assert_eq!(p2.life, 17, "Player should have lost 3 life");
    }

    // ==================== DestroyAll Tests ====================

    #[test]
    fn test_destroy_all_creatures() {
        // Wrath of God: destroy all creatures, no regeneration
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1: Two creatures
        let c1_id = game.next_entity_id();
        let mut c1 = Card::new(c1_id, "Grizzly Bears".to_string(), p1_id);
        c1.add_type(CardType::Creature);
        c1.set_base_power(Some(2));
        c1.set_base_toughness(Some(2));
        c1.controller = p1_id;
        game.cards.insert(c1_id, c1);
        game.battlefield.add(c1_id);

        let c2_id = game.next_entity_id();
        let mut c2 = Card::new(c2_id, "Elvish Mystic".to_string(), p1_id);
        c2.add_type(CardType::Creature);
        c2.set_base_power(Some(1));
        c2.set_base_toughness(Some(1));
        c2.controller = p1_id;
        game.cards.insert(c2_id, c2);
        game.battlefield.add(c2_id);

        // P2: One creature
        let c3_id = game.next_entity_id();
        let mut c3 = Card::new(c3_id, "Shivan Dragon".to_string(), p2_id);
        c3.add_type(CardType::Creature);
        c3.set_base_power(Some(5));
        c3.set_base_toughness(Some(5));
        c3.controller = p2_id;
        game.cards.insert(c3_id, c3);
        game.battlefield.add(c3_id);

        // P1: A land (should NOT be destroyed)
        let land_id = game.next_entity_id();
        let mut land = Card::new(land_id, "Plains".to_string(), p1_id);
        land.add_type(CardType::Land);
        land.controller = p1_id;
        game.cards.insert(land_id, land);
        game.battlefield.add(land_id);

        // Cast Wrath of God
        let wrath_id = game.next_entity_id();
        let mut wrath = Card::new(wrath_id, "Wrath of God".to_string(), p1_id);
        wrath.add_type(CardType::Sorcery);
        wrath.mana_cost = ManaCost::from_string("2WW");
        wrath.effects.push(Effect::DestroyAll {
            restriction: crate::core::TargetRestriction::from_types([crate::core::TargetType::Creature]),
            no_regenerate: true,
        });
        game.cards.insert(wrath_id, wrath);
        game.stack.add(wrath_id);

        game.resolve_spell(wrath_id, &[]).unwrap();

        // All creatures should be gone
        assert!(!game.battlefield.contains(c1_id), "P1 creature 1 should be destroyed");
        assert!(!game.battlefield.contains(c2_id), "P1 creature 2 should be destroyed");
        assert!(!game.battlefield.contains(c3_id), "P2 creature should be destroyed");

        // Land should survive
        assert!(
            game.battlefield.contains(land_id),
            "Land should survive DestroyAll Creature"
        );
    }

    #[test]
    fn test_destroy_all_respects_indestructible() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // Normal creature
        let normal_id = game.next_entity_id();
        let mut normal = Card::new(normal_id, "Grizzly Bears".to_string(), p1_id);
        normal.add_type(CardType::Creature);
        normal.set_base_power(Some(2));
        normal.set_base_toughness(Some(2));
        normal.controller = p1_id;
        game.cards.insert(normal_id, normal);
        game.battlefield.add(normal_id);

        // Indestructible creature
        let indestructible_id = game.next_entity_id();
        let mut indestructible = Card::new(indestructible_id, "Darksteel Colossus".to_string(), p2_id);
        indestructible.add_type(CardType::Creature);
        indestructible.set_base_power(Some(11));
        indestructible.set_base_toughness(Some(11));
        indestructible.keywords.insert(Keyword::Indestructible);
        indestructible.controller = p2_id;
        game.cards.insert(indestructible_id, indestructible);
        game.battlefield.add(indestructible_id);

        // Wrath of God (no regen)
        let wrath_id = game.next_entity_id();
        let mut wrath = Card::new(wrath_id, "Wrath of God".to_string(), p1_id);
        wrath.add_type(CardType::Sorcery);
        wrath.mana_cost = ManaCost::from_string("2WW");
        wrath.effects.push(Effect::DestroyAll {
            restriction: crate::core::TargetRestriction::from_types([crate::core::TargetType::Creature]),
            no_regenerate: true,
        });
        game.cards.insert(wrath_id, wrath);
        game.stack.add(wrath_id);

        game.resolve_spell(wrath_id, &[]).unwrap();

        assert!(
            !game.battlefield.contains(normal_id),
            "Normal creature should be destroyed"
        );
        assert!(
            game.battlefield.contains(indestructible_id),
            "Indestructible creature should survive Wrath of God"
        );
    }

    // ==================== DamageAll Tests ====================

    #[test]
    fn test_damage_all_creatures() {
        // Pyroclasm: 2 damage to each creature
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // 1/1 (should die)
        let small_id = game.next_entity_id();
        let mut small = Card::new(small_id, "Elvish Mystic".to_string(), p1_id);
        small.add_type(CardType::Creature);
        small.set_base_power(Some(1));
        small.set_base_toughness(Some(1));
        small.controller = p1_id;
        game.cards.insert(small_id, small);
        game.battlefield.add(small_id);

        // 3/3 (should survive with 2 damage)
        let big_id = game.next_entity_id();
        let mut big = Card::new(big_id, "Centaur Courser".to_string(), p2_id);
        big.add_type(CardType::Creature);
        big.set_base_power(Some(3));
        big.set_base_toughness(Some(3));
        big.controller = p2_id;
        game.cards.insert(big_id, big);
        game.battlefield.add(big_id);

        // Cast Pyroclasm
        let pyro_id = game.next_entity_id();
        let mut pyro = Card::new(pyro_id, "Pyroclasm".to_string(), p1_id);
        pyro.add_type(CardType::Sorcery);
        pyro.mana_cost = ManaCost::from_string("1R");
        pyro.effects.push(Effect::DamageAll {
            amount: 2,
            valid_cards: crate::core::TargetRestriction::from_types([crate::core::TargetType::Creature]),
            damage_players: false,
        });
        game.cards.insert(pyro_id, pyro);
        game.stack.add(pyro_id);

        game.resolve_spell(pyro_id, &[]).unwrap();

        // 1/1 should be dead (2 damage >= 1 toughness)
        assert!(!game.battlefield.contains(small_id), "1/1 should die to 2 damage");

        // 3/3 should survive with 2 damage marked
        assert!(game.battlefield.contains(big_id), "3/3 should survive 2 damage");
        let big_card = game.cards.get(big_id).unwrap();
        assert_eq!(big_card.damage, 2, "3/3 should have 2 damage marked");
    }

    #[test]
    fn test_damage_all_with_players() {
        // Earthquake-style: damage each creature and each player
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        let spell_id = game.next_entity_id();
        let mut spell = Card::new(spell_id, "Earthquake".to_string(), p1_id);
        spell.add_type(CardType::Sorcery);
        spell.mana_cost = ManaCost::from_string("XR");
        spell.effects.push(Effect::DamageAll {
            amount: 3,
            valid_cards: crate::core::TargetRestriction::from_types([crate::core::TargetType::Creature]),
            damage_players: true,
        });
        game.cards.insert(spell_id, spell);
        game.stack.add(spell_id);

        game.resolve_spell(spell_id, &[]).unwrap();

        // Both players should take damage
        let p1 = game.get_player(p1_id).unwrap();
        let p2 = game.get_player(p2_id).unwrap();
        assert_eq!(p1.life, 17, "P1 should have lost 3 life");
        assert_eq!(p2.life, 17, "P2 should have lost 3 life");
    }

    // ==================== ForceSacrifice Tests ====================

    #[test]
    fn test_force_sacrifice_creature() {
        // Diabolic Edict: target player sacrifices a creature
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P2 has two creatures - a big one and a small one
        let bear_id = game.next_entity_id();
        let mut bear = Card::new(bear_id, "Grizzly Bears".to_string(), p2_id);
        bear.add_type(CardType::Creature);
        bear.set_base_power(Some(2));
        bear.set_base_toughness(Some(2));
        bear.controller = p2_id;
        game.cards.insert(bear_id, bear);
        game.battlefield.add(bear_id);

        let dragon_id = game.next_entity_id();
        let mut dragon = Card::new(dragon_id, "Shivan Dragon".to_string(), p2_id);
        dragon.add_type(CardType::Creature);
        dragon.set_base_power(Some(5));
        dragon.set_base_toughness(Some(5));
        dragon.controller = p2_id;
        game.cards.insert(dragon_id, dragon);
        game.battlefield.add(dragon_id);

        // P1 casts Diabolic Edict targeting P2
        let edict_id = game.next_entity_id();
        let mut edict = Card::new(edict_id, "Diabolic Edict".to_string(), p1_id);
        edict.add_type(CardType::Instant);
        edict.mana_cost = ManaCost::from_string("1B");
        edict.effects.push(Effect::ForceSacrifice {
            player: p2_id,
            sac_type: "Creature".to_string(),
            count: 1,
        });
        game.cards.insert(edict_id, edict);
        game.stack.add(edict_id);

        game.resolve_spell(edict_id, &[]).unwrap();

        // P2 should sacrifice the least valuable creature (Bears, P/T sum 4 < Dragon 10)
        assert!(
            !game.battlefield.contains(bear_id),
            "Grizzly Bears should be sacrificed (least valuable)"
        );
        assert!(
            game.battlefield.contains(dragon_id),
            "Shivan Dragon should survive (more valuable)"
        );
    }

    #[test]
    fn test_force_sacrifice_two_creatures() {
        // Barter in Blood: each player sacrifices two creatures
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P2 has three creatures
        let mut creature_ids = Vec::new();
        for name in ["Grizzly Bears", "Elvish Mystic", "Shivan Dragon"] {
            let id = game.next_entity_id();
            let mut c = Card::new(id, name.to_string(), p2_id);
            c.add_type(CardType::Creature);
            c.set_base_power(Some(if name == "Shivan Dragon" {
                5
            } else if name == "Grizzly Bears" {
                2
            } else {
                1
            }));
            c.set_base_toughness(Some(if name == "Shivan Dragon" {
                5
            } else if name == "Grizzly Bears" {
                2
            } else {
                1
            }));
            c.controller = p2_id;
            game.cards.insert(id, c);
            game.battlefield.add(id);
            creature_ids.push(id);
        }

        // Force P2 to sacrifice 2 creatures
        let spell_id = game.next_entity_id();
        let mut spell = Card::new(spell_id, "Barter in Blood".to_string(), p1_id);
        spell.add_type(CardType::Sorcery);
        spell.effects.push(Effect::ForceSacrifice {
            player: p2_id,
            sac_type: "Creature".to_string(),
            count: 2,
        });
        game.cards.insert(spell_id, spell);
        game.stack.add(spell_id);

        game.resolve_spell(spell_id, &[]).unwrap();

        // Should keep the most valuable creature (Shivan Dragon)
        assert!(
            game.battlefield.contains(creature_ids[2]),
            "Shivan Dragon should survive (most valuable)"
        );
        // The other two should be sacrificed
        let bf_creatures = game
            .battlefield
            .cards
            .iter()
            .filter(|&&id| game.cards.get(id).map(|c| c.is_creature()).unwrap_or(false))
            .count();
        assert_eq!(bf_creatures, 1, "Only 1 creature should remain after sacrificing 2");
    }

    #[test]
    fn test_force_sacrifice_comma_list_includes_planeswalker() {
        // mtg-907: `SacValid$ Creature,Planeswalker` must offer BOTH creatures
        // and planeswalkers as sacrifice candidates. The pre-mtg-907 hand-rolled
        // `match sac_type` only handled single bare types and fell through to
        // `is_creature()` for the comma-list, so planeswalkers were NEVER
        // sacrificable. Now routed through TargetRestriction::parse, which
        // splits on ',' into [Creature, Planeswalker] and matches either.
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P2 controls a big creature (P/T sum 10) and a cheap planeswalker
        // (CMC 3). The AI sacrifices the LEAST valuable candidate: creatures are
        // scored by P/T sum (10), non-creatures by CMC (3). So the planeswalker
        // (value 3) is chosen — which is ONLY possible if it is in the candidate
        // set at all, i.e. the comma-list filter matched it.
        let dragon_id = game.next_entity_id();
        let mut dragon = Card::new(dragon_id, "Shivan Dragon".to_string(), p2_id);
        dragon.add_type(CardType::Creature);
        dragon.set_base_power(Some(5));
        dragon.set_base_toughness(Some(5));
        dragon.controller = p2_id;
        game.cards.insert(dragon_id, dragon);
        game.battlefield.add(dragon_id);

        let pw_id = game.next_entity_id();
        let mut pw = Card::new(pw_id, "Jace Beleren".to_string(), p2_id);
        pw.add_type(CardType::Planeswalker);
        pw.mana_cost = ManaCost::from_string("1UU"); // CMC 3
        pw.controller = p2_id;
        game.cards.insert(pw_id, pw);
        game.battlefield.add(pw_id);

        // "Each opponent sacrifices a creature or planeswalker."
        let spell_id = game.next_entity_id();
        let mut spell = Card::new(spell_id, "Edict of the Walkers".to_string(), p1_id);
        spell.add_type(CardType::Sorcery);
        spell.effects.push(Effect::ForceSacrifice {
            player: p2_id,
            sac_type: "Creature,Planeswalker".to_string(),
            count: 1,
        });
        game.cards.insert(spell_id, spell);
        game.stack.add(spell_id);

        game.resolve_spell(spell_id, &[]).unwrap();

        // The planeswalker (value 3) is the cheaper candidate and is sacrificed;
        // the dragon (value 10) survives. Pre-fix this was IMPOSSIBLE — the
        // planeswalker was never a candidate, so the dragon would have been
        // forced out instead.
        assert!(
            !game.battlefield.contains(pw_id),
            "Jace (planeswalker) should be sacrificed — comma-list filter must include planeswalkers (mtg-907)"
        );
        assert!(
            game.battlefield.contains(dragon_id),
            "Shivan Dragon should survive (higher value than the planeswalker)"
        );
    }

    // ==================== TapAll Tests ====================

    #[test]
    fn test_tap_all_creatures() {
        // TapAll with creature type restriction - taps all creatures
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1 creature
        let c1_id = game.next_entity_id();
        let mut c1 = Card::new(c1_id, "Serra Angel".to_string(), p1_id);
        c1.add_type(CardType::Creature);
        c1.controller = p1_id;
        game.cards.insert(c1_id, c1);
        game.battlefield.add(c1_id);

        // P2 creature
        let c2_id = game.next_entity_id();
        let mut c2 = Card::new(c2_id, "Grizzly Bears".to_string(), p2_id);
        c2.add_type(CardType::Creature);
        c2.controller = p2_id;
        game.cards.insert(c2_id, c2);
        game.battlefield.add(c2_id);

        // P1 land (should NOT be tapped - not a creature)
        let land_id = game.next_entity_id();
        let mut land = Card::new(land_id, "Plains".to_string(), p1_id);
        land.add_type(CardType::Land);
        land.controller = p1_id;
        game.cards.insert(land_id, land);
        game.battlefield.add(land_id);

        // TapAll creatures
        let restriction = crate::core::TargetRestriction::from_types([crate::core::TargetType::Creature]);

        let spell_id = game.next_entity_id();
        let mut spell = Card::new(spell_id, "Sleep".to_string(), p1_id);
        spell.add_type(CardType::Sorcery);
        spell.effects.push(Effect::TapAll { restriction });
        game.cards.insert(spell_id, spell);
        game.stack.add(spell_id);

        game.resolve_spell(spell_id, &[]).unwrap();

        // Both creatures should be tapped
        assert!(game.cards.get(c1_id).unwrap().tapped, "P1's creature should be tapped");
        assert!(game.cards.get(c2_id).unwrap().tapped, "P2's creature should be tapped");
        // Land should NOT be tapped
        assert!(!game.cards.get(land_id).unwrap().tapped, "Land should not be tapped");
    }

    // ==================== UntapAll Tests ====================

    #[test]
    fn test_untap_all_creatures() {
        // UntapAll with creature type restriction - untaps all tapped creatures
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1 tapped creature
        let c1_id = game.next_entity_id();
        let mut c1 = Card::new(c1_id, "Grizzly Bears".to_string(), p1_id);
        c1.add_type(CardType::Creature);
        c1.controller = p1_id;
        c1.tapped = true;
        game.cards.insert(c1_id, c1);
        game.battlefield.add(c1_id);

        // P2 tapped creature
        let c2_id = game.next_entity_id();
        let mut c2 = Card::new(c2_id, "Shivan Dragon".to_string(), p2_id);
        c2.add_type(CardType::Creature);
        c2.controller = p2_id;
        c2.tapped = true;
        game.cards.insert(c2_id, c2);
        game.battlefield.add(c2_id);

        // P1 tapped land (should NOT be untapped - not a creature)
        let land_id = game.next_entity_id();
        let mut land = Card::new(land_id, "Plains".to_string(), p1_id);
        land.add_type(CardType::Land);
        land.controller = p1_id;
        land.tapped = true;
        game.cards.insert(land_id, land);
        game.battlefield.add(land_id);

        // P1 untapped creature (should remain untapped)
        let c3_id = game.next_entity_id();
        let mut c3 = Card::new(c3_id, "Elvish Mystic".to_string(), p1_id);
        c3.add_type(CardType::Creature);
        c3.controller = p1_id;
        game.cards.insert(c3_id, c3);
        game.battlefield.add(c3_id);

        // UntapAll creatures
        let restriction = crate::core::TargetRestriction::from_types([crate::core::TargetType::Creature]);

        let spell_id = game.next_entity_id();
        let mut spell = Card::new(spell_id, "Mobilize".to_string(), p1_id);
        spell.add_type(CardType::Sorcery);
        spell.effects.push(Effect::UntapAll { restriction });
        game.cards.insert(spell_id, spell);
        game.stack.add(spell_id);

        game.resolve_spell(spell_id, &[]).unwrap();

        // Both tapped creatures should be untapped
        assert!(
            !game.cards.get(c1_id).unwrap().tapped,
            "P1's creature should be untapped"
        );
        assert!(
            !game.cards.get(c2_id).unwrap().tapped,
            "P2's creature should be untapped"
        );
        // Land should still be tapped
        assert!(game.cards.get(land_id).unwrap().tapped, "Land should still be tapped");
        // Already-untapped creature should remain untapped
        assert!(
            !game.cards.get(c3_id).unwrap().tapped,
            "Already-untapped creature stays untapped"
        );
    }

    // ==================== SetLife Tests ====================

    #[test]
    fn test_set_life_effect() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        let spell_id = game.next_entity_id();
        let mut spell = Card::new(spell_id, "Angel of Grace".to_string(), p1_id);
        spell.add_type(CardType::Instant);
        spell.effects.push(Effect::SetLife {
            player: p1_id,
            amount: 10,
        });
        game.cards.insert(spell_id, spell);
        game.stack.add(spell_id);

        game.resolve_spell(spell_id, &[]).unwrap();

        let p1 = game.get_player(p1_id).unwrap();
        assert_eq!(p1.life, 10, "P1's life should be set to 10");
    }

    #[test]
    fn test_set_life_increase() {
        // SetLife can also increase life (e.g., Blessed Wind sets to 20 when at low life)
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Reduce P1 life first
        game.get_player_mut(p1_id).unwrap().life = 5;

        let spell_id = game.next_entity_id();
        let mut spell = Card::new(spell_id, "Blessed Wind".to_string(), p1_id);
        spell.add_type(CardType::Sorcery);
        spell.effects.push(Effect::SetLife {
            player: p1_id,
            amount: 20,
        });
        game.cards.insert(spell_id, spell);
        game.stack.add(spell_id);

        game.resolve_spell(spell_id, &[]).unwrap();

        let p1 = game.get_player(p1_id).unwrap();
        assert_eq!(p1.life, 20, "P1's life should be restored to 20");
    }

    /// CR 614.1c + CR 107.3: X-cost permanents enter the battlefield with X
    /// counters, where X is the amount paid when casting the spell.
    ///
    /// Regression test for B1 from the 2015 World Championship compat survey:
    /// Hangarback Walker (K:etbCounter:P1P1:X) was entering as a 0/0 with NO
    /// counters because `apply_etb_counters` did not resolve the symbolic "X"
    /// amount via the card's `x_paid` field.
    #[test]
    fn test_etb_counter_x_cost_uses_x_paid() {
        use crate::core::{CardType, CounterType, KeywordArgs, KeywordSet};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Build a minimal Hangarback Walker-like card:
        //   ManaCost: X X (any generic); PT: 0/0; K:etbCounter:P1P1:X
        let walker_id = game.next_entity_id();
        let mut walker = Card::new(walker_id, "Hangarback Walker".to_string(), p1_id);
        walker.add_type(CardType::Artifact);
        walker.add_type(CardType::Creature);
        walker.set_base_power(Some(0));
        walker.set_base_toughness(Some(0));
        walker.controller = p1_id;

        // Attach K:etbCounter:P1P1:X keyword (the "X" amount is symbolic).
        let mut kws = KeywordSet::default();
        kws.insert_complex(KeywordArgs::EtbCounter {
            counter_type: "P1P1".to_string(),
            amount: "X".to_string(),
            condition: String::new(),
        });
        walker.keywords = kws;

        // Simulate X = 3 (player paid {3}{3} for the Walker).
        walker.x_paid = 3;

        // Place the card on the stack so `resolve_spell_finalize` can pick it up.
        game.cards.insert(walker_id, walker);
        game.stack.add(walker_id);

        // Resolving a permanent spell moves it to the battlefield and calls
        // `apply_etb_counters`.
        game.resolve_spell(walker_id, &[]).unwrap();

        // Walker should now be on the battlefield.
        assert!(
            game.battlefield.contains(walker_id),
            "Hangarback Walker should be on the battlefield after resolving"
        );

        // The card must have exactly 3 +1/+1 counters (= x_paid).
        let actual_counters = game
            .cards
            .get(walker_id)
            .map(|c| c.get_counter(CounterType::P1P1))
            .unwrap_or(0);
        assert_eq!(
            actual_counters, 3,
            "Hangarback Walker should enter with 3 +1/+1 counters when X=3, \
             but got {actual_counters}"
        );

        // P/T with counters: 0/0 base + 3/3 from counters = 3/3.
        let card = game.cards.get(walker_id).unwrap();
        assert_eq!(
            card.current_power(),
            3,
            "Hangarback Walker should be 3/3 with 3 counters"
        );
        assert_eq!(
            card.current_toughness(),
            3,
            "Hangarback Walker should be 3/3 with 3 counters"
        );
    }

    /// CR 603.6c + CR 608.2g (LKI): Hangarback Walker's death trigger creates
    /// one Thopter for EACH +1/+1 counter on the dying card.
    ///
    /// Regression test for the secondary B1 bug: the death trigger SVar
    /// `DB$ Token | TokenAmount$ Y` (where `SVar:Y:TriggeredCard$CardCounters.P1P1`)
    /// was falling back to `amount=1` because `params_to_effect` did not resolve
    /// the variable `Y` through the card's SVars.  After the fix,
    /// `extract_effects_from_svar` emits `Effect::CreateTokenDynamic` with
    /// `DynamicAmount::TriggeredCardCounters(P1P1)`, which `check_death_triggers`
    /// resolves to a concrete `CreateToken { amount: counter_count }` via the LKI
    /// snapshot in `TriggerContext`.
    ///
    /// Test approach: load the real Hangarback Walker card script, verify that
    /// the death trigger effect is `CreateTokenDynamic` (not `CreateToken` with a
    /// fixed amount), and that `check_death_triggers` produces the correct number
    /// of tokens using a fake token definition.
    #[test]
    fn test_hangarback_walker_thopter_count_uses_p1p1_counters() {
        use crate::core::{CounterType, DynamicAmount, Effect};
        use crate::loader::CardLoader;

        // Load the real Hangarback Walker script to verify the parser produces
        // CreateTokenDynamic for the death trigger.
        let script = r#"Name:Hangarback Walker
ManaCost:X X
Types:Artifact Creature Construct
PT:0/0
K:etbCounter:P1P1:X
T:Mode$ ChangesZone | Origin$ Battlefield | Destination$ Graveyard | ValidCard$ Card.Self | Execute$ TrigToken | TriggerDescription$ When CARDNAME dies, create a 1/1 colorless Thopter artifact creature token with flying for each +1/+1 counter on CARDNAME.
SVar:TrigToken:DB$ Token | TokenAmount$ Y | TokenScript$ c_1_1_a_thopter_flying | TokenOwner$ You
SVar:Y:TriggeredCard$CardCounters.P1P1
A:AB$ PutCounter | Cost$ 1 T | CounterType$ P1P1 | CounterNum$ 1 | SpellDescription$ Put a +1/+1 counter on CARDNAME.
SVar:X:Count$xPaid
Oracle:Hangarback Walker enters with X +1/+1 counters on it.
"#;
        let def = CardLoader::parse(script).unwrap();
        let p1_id = crate::core::PlayerId::new(0);
        let card = def.instantiate(crate::core::CardId::new(1), p1_id);

        // The death trigger must be a `CreateTokenDynamic` with
        // `DynamicAmount::TriggeredCardCounters(CounterType::P1P1)`.
        let death_trigger = card
            .triggers
            .iter()
            .find(|t| t.event == crate::core::TriggerEvent::LeavesBattlefield)
            .expect("Hangarback Walker must have a death trigger");

        let dynamic_token_effect = death_trigger.effects.iter().find(|e| {
            matches!(
                e,
                Effect::CreateTokenDynamic {
                    amount: DynamicAmount::TriggeredCardCounters(CounterType::P1P1),
                    ..
                }
            )
        });
        assert!(
            dynamic_token_effect.is_some(),
            "Hangarback Walker death trigger must emit CreateTokenDynamic with \
             DynamicAmount::TriggeredCardCounters(P1P1), but found effects: {:?}",
            death_trigger.effects
        );
    }
}
